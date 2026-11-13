//! The surface a match runs a world through: what the game loop asks of it.

use bota_core::order_halts as is_halt;
use bota_core::{AllyUse, ItemDef, clamp_to_map, hero_spawn_pos, item_def, map_of, rules};
use bota_proto::{Aim, MatchStats, Order, RejectReason, SlotId, SlotStats, Target, Team};

use crate::game::{
    ActionPhase, ActionState, Command, Entity, Event, MatchConfig, MatchRng, PendingCast, Seat,
    UnitOrder, World, in_backpack, in_stash, wire_id,
};

pub use bota_core::aimed_right;

impl World {
    /// A world at tick zero for a match: the map standing, the setup's
    /// modifiers put on, and a seat, a hero and a courier per pick.
    pub fn for_match(cfg: &MatchConfig, rng: MatchRng) -> World {
        if let Err(error) = cfg.validate() {
            panic!("the match setup was refused: {error}");
        }
        let map = map_of(cfg.map);
        let mut world = World::on_map(map);
        world.rng = rng;
        world.cheats = cfg.cheats;
        world.catalogue = cfg.catalogue;
        world.shop_stock = crate::game::fresh_stock(cfg.catalogue);
        world.immortal_structures = cfg.immortal_structures;
        world.immortal_heroes = cfg.immortal_heroes;
        world.spawn_modifiers = cfg.spawn_modifiers.clone();
        world.apply_spawn_modifiers_to_all();
        // In tick order; events of one tick keep the order the setup gave.
        let mut events = cfg.economy_events.clone();
        events.sort_by_key(|event| event.tick);
        world.economy_events = crate::game::EconomySchedule(events.into());
        world.stand_up_objectives(cfg.objectives);
        world.stand_up_runes(cfg.runes);
        for pick in &cfg.picks {
            let at = hero_spawn_pos(map, pick.team);
            let hero = world.spawn_hero(pick.team, at, pick.slot, pick.hero);
            let mut seat = Seat::new(
                pick.slot,
                pick.team,
                pick.hero,
                rules::STARTING_GOLD - cfg.gold_lost_by(pick.slot),
                rules::STASH_SLOTS,
            );
            seat.unit = Some(hero);
            // The scroll every hero starts with comes on a wait.
            seat.item_clocks.push((
                bota_proto::ItemId::TOWN_PORTAL_SCROLL,
                world.schedule().scroll_first_ready(),
            ));
            world.seats.push(seat);
        }
        for seat in 0..world.seats.len() {
            world.stand_up_courier(seat);
        }
        world.settle();
        // Passability ignores dead structures; stats must fill new bodies' pools first.
        world.lay_passability();
        world
    }

    /// One tick of the match: every command taken in the order given, then
    /// [`World::step`]. Commands are trusted simulation inputs, not player
    /// requests: use [`super::DelayLine`] for untrusted orders. A completed
    /// Map2 ignores commands and advances no further. What it refused is
    /// left in [`World::refused`].
    pub fn advance_trusted(&mut self, cmds: &[Command]) -> Vec<Event> {
        self.advance_commands(cmds, false)
    }

    /// Rechecks every player request in application order, including cooldowns
    /// and queue capacity, against the state left by earlier commands.
    pub(super) fn advance_checked(&mut self, cmds: &[Command]) -> Vec<Event> {
        self.advance_commands(cmds, true)
    }

    fn advance_commands(&mut self, cmds: &[Command], check: bool) -> Vec<Event> {
        self.refused.clear();
        if self.map2_finished() {
            return Vec::new();
        }
        let mut events = Vec::new();
        self.economy_events_due(&mut events);
        for cmd in cmds {
            if check
                && let Err(reason) = self.check_order(cmd.slot, cmd.unit, &cmd.order, cmd.queue)
            {
                self.refused.push(crate::game::RefusedOrder {
                    slot: cmd.slot,
                    seq: cmd.seq,
                    reason,
                });
                continue;
            }
            self.take_order(cmd, &mut events);
            if inventory_transaction(cmd.order) {
                // Stat-dependent uses and casts must see this transaction too,
                // including pools rescaled by a changed attribute bonus.
                self.settle();
            }
        }
        events.extend(self.step());
        events
    }

    /// Carries out the setup's economic events due this tick, before its
    /// orders: gold into or out of the seat's hand, never below nothing, and
    /// items handed out as the shop would deliver them.
    fn economy_events_due(&mut self, events: &mut Vec<Event>) {
        while self
            .economy_events
            .0
            .front()
            .is_some_and(|event| event.tick <= self.tick)
        {
            let Some(event) = self.economy_events.0.pop_front() else {
                break;
            };
            let Some(seat) = self.seats.iter().position(|seat| seat.slot == event.slot) else {
                continue;
            };
            let gold = &mut self.seats[seat].gold;
            *gold = gold.saturating_add(event.gold).max(0);
            for item in event.items {
                self.hand_out(seat, item, events);
            }
        }
    }

    /// Hands one order to the unit it is for: into its queue when it is
    /// queued behind something, else to be carried out now.
    fn take_order(&mut self, cmd: &Command, events: &mut Vec<Event>) {
        if cmd.order == Order::Glyph {
            self.use_glyph(cmd.slot);
            return;
        }
        if self.for_dead_hero(cmd.slot, cmd.unit) {
            self.take_order_while_down(cmd.slot, cmd.order, events);
            return;
        }
        let Some(unit) = self.driven_by(cmd.slot, cmd.unit) else {
            return;
        };
        // Any command to a hero risen at its fountain ends its fountain
        // invulnerability, a queued one too: moving, attacking, casting,
        // and shop dealings alike ("until the respawning hero is issued a
        // command", Liquipedia, *Hero (unit type)*). A cheat is no command
        // to the hero.
        if !matches!(cmd.order, Order::Cheat { .. })
            && self
                .seat(cmd.slot)
                .is_some_and(|seat| seat.unit == Some(unit))
        {
            self.take_modifier(
                unit,
                bota_core::ModifierKind::FountainInvulnerable,
                Some(unit),
            );
        }
        if cmd.queued() && !self.free_to_take_up(unit) {
            self.enqueue(unit, cmd);
            return;
        }
        // An order to the body given unqueued replaces the queue; what goes
        // off beside the body, and what needs no body, leaves it be.
        if bota_core::order_queues(cmd.order) && self.immediate_cast(unit, cmd.order).is_none() {
            self.queue.remove(unit);
        }
        self.carry_out(cmd.slot, unit, cmd.order, cmd.seq, events);
    }

    /// Carries one order out on a unit of the seat that gave it.
    pub(crate) fn carry_out(
        &mut self,
        seat: SlotId,
        unit: Entity,
        order: Order,
        seq: u32,
        events: &mut Vec<Event>,
    ) {
        // Learning, talents, the bag, the shop, cheats and immediate casts interrupt
        // nothing the body is doing.
        if let Some(cast) = self.immediate_cast(unit, order) {
            // It goes off beside a swing or a cast point, but a channel it
            // does not ignore it breaks, as Dota's immediate casts do.
            if self.is_channelling(unit) && !self.ignores_channel(unit, &order) {
                self.cancel_action(unit);
            }
            if let Some(orders) = self.orders.get_mut(unit) {
                orders.immediate = Some(cast);
            }
            return;
        }
        match order {
            Order::Learn { slot } => {
                self.learn(unit, usize::from(slot.0));
            }
            Order::LearnTalent { talent } => {
                self.learn_talent(unit, talent);
            }
            Order::LearnAttributeBonus => {
                self.learn_attribute_bonus(unit);
            }
            Order::Swap { from, to } => {
                self.move_item(seat, unit, usize::from(from.0), usize::from(to.0));
            }
            Order::Sell { slot: at } => {
                self.sell_item(seat, unit, usize::from(at.0));
            }
            Order::Buy { item } => {
                self.buy(seat, item, events);
            }
            Order::SwitchWard { slot } => self.switch_ward(unit, usize::from(slot.0)),
            Order::Disassemble { slot: at } => {
                self.disassemble_item(seat, unit, usize::from(at.0));
            }
            Order::SetCombineLock { slot: at, locked } => {
                self.set_combine_lock(seat, unit, usize::from(at.0), locked);
            }
            Order::Cheat { cheat } => {
                self.cheat(seat, Some(unit), cheat, events);
            }
            order if self.is_channelling(unit) && self.ignores_channel(unit, &order) => {
                self.take_body_order(unit, order);
            }
            // An order to cast what Dota marks DONT_CANCEL_CHANNEL leaves the
            // channel running and asks nothing more: the warp's gate clicked
            // again while the warp runs.
            order if self.is_channelling(unit) && self.keeps_channel(unit, &order) => {}
            order if bota_core::order_queues(order) && !is_halt(order) && self.is_casting(unit) => {
                self.wait_for_cast(seat, unit, order, seq);
            }
            order => {
                self.interrupt(unit, order);
                self.take_body_order(unit, order);
            }
        }
    }

    /// The cast an order makes when what it casts or uses goes off beside
    /// whatever the body is doing: Dota's `DOTA_ABILITY_BEHAVIOR_IMMEDIATE`.
    fn immediate_cast(&self, unit: Entity, order: Order) -> Option<PendingCast> {
        match order {
            Order::Cast { slot, target } => bota_core::ability_def(self.ability_in(unit, slot))
                .filter(|def| def.immediate)
                .map(|_| PendingCast::Ability { slot, target }),
            Order::Use { slot, target } => self
                .item_def_in(unit, usize::from(slot.0))
                .filter(|def| def.immediate)
                .map(|_| PendingCast::Item { slot, target }),
            _ => None,
        }
    }

    /// Whether an order casts an ability Dota marks
    /// `DOTA_ABILITY_BEHAVIOR_DONT_CANCEL_CHANNEL`: a cast order of it, or an
    /// order at a map objective whose hidden ability it is.
    fn keeps_channel(&self, unit: Entity, order: &Order) -> bool {
        let slot = match *order {
            Order::Cast { slot, .. } => Some(slot),
            Order::Move {
                target: Target::Unit(at),
            }
            | Order::Attack {
                target: Target::Unit(at),
            } => self.interaction_at(unit, at),
            _ => None,
        };
        slot.and_then(|slot| bota_core::ability_def(self.ability_in(unit, slot)))
            .is_some_and(|def| def.dont_cancel_channel)
    }

    /// Whether an order is a cast or a use of something that leaves a
    /// channel be: Dota's `DOTA_ABILITY_BEHAVIOR_IGNORE_CHANNEL`.
    fn ignores_channel(&self, unit: Entity, order: &Order) -> bool {
        match *order {
            Order::Cast { slot, .. } => bota_core::ability_def(self.ability_in(unit, slot))
                .is_some_and(|def| def.ignores_channel),
            Order::Use { slot, .. } => self
                .item_def_in(unit, usize::from(slot.0))
                .is_some_and(|def| def.ignores_channel),
            _ => false,
        }
    }

    /// Holds an order given in a cast point until the cast has gone off, as
    /// Dota's pseudo-queue does (Liquipedia, *Turn Rate*: a command given then
    /// "gets queued up and executed once the spell is cast"; the 2017-05-31
    /// update lets casts given then go off in turn). Only a stop or a hold
    /// breaks the cast off. A cast given then waits behind the casts already
    /// waiting, or replaces the one move, attack or the like waiting; any
    /// other order replaces whatever waits. Refused when the casts waiting
    /// fill [`MAX_QUEUED_ORDERS`](bota_core::rules::MAX_QUEUED_ORDERS).
    fn wait_for_cast(&mut self, seat: SlotId, unit: Entity, order: Order, seq: u32) {
        let mut waiting = self.waiting.remove(unit).unwrap_or_default();
        if !bota_core::order_is_cast(order)
            || !waiting
                .iter()
                .all(|queued| bota_core::order_is_cast(queued.order))
        {
            waiting = crate::game::OrderQueue::default();
        }
        if !waiting.push(crate::game::Queued { order, seq }) {
            self.refused.push(crate::game::RefusedOrder {
                slot: seat,
                seq,
                reason: RejectReason::QueueFull,
            });
        }
        self.waiting.insert(unit, waiting);
    }

    /// What an order to the body breaks off: the action under way, as an
    /// animation cancel, a held cast, and an errand.
    fn interrupt(&mut self, unit: Entity, order: Order) {
        self.waiting.remove(unit);
        self.cancel_action(unit);
        self.break_swing_off(unit, order);
        self.handling.remove(unit);
        if let Some(orders) = self.orders.get_mut(unit) {
            orders.pending = None;
        }
        if let Some(errand) = self.errand.get_mut(unit) {
            *errand = crate::game::Errand::None;
        }
    }

    /// Breaks off a swing in its windup, at no cost, for a move, a cast, or an attack at
    /// anything but the swing's own target, as Dota's animation cancel does (Liquipedia, "Attack
    /// Animation"); an attack order at that target lets it strike.
    fn break_swing_off(&mut self, unit: Entity, order: Order) {
        let Some(action) = self.action.get(unit).copied() else {
            return;
        };
        let ActionState::Attack {
            target,
            phase: ActionPhase::Before { .. },
        } = action.state
        else {
            return;
        };
        let breaks = match order {
            Order::Attack {
                target: Target::Unit(named),
            } => named != wire_id(target),
            Order::Move { .. }
            | Order::MoveDirection { .. }
            | Order::Attack { .. }
            | Order::Cast { .. } => true,
            _ => false,
        };
        if breaks && let Some(action) = self.action.get_mut(unit) {
            action.state = ActionState::Ready;
            action.attack_cooldown = 0;
        }
    }

    /// Sets a body on what an order asks of it.
    fn take_body_order(&mut self, unit: Entity, order: Order) {
        let wanted = match order {
            Order::Move {
                target: Target::Pos(pos),
            } => UnitOrder::Move { pos },
            Order::Attack {
                target: Target::Pos(pos),
            } => UnitOrder::AttackMove { pos },
            Order::MoveDirection { toward } => {
                let by = clamp_to_map(toward) - self.pos_of(unit);
                if by == bota_proto::Vec2::ZERO {
                    UnitOrder::Stand
                } else {
                    UnitOrder::Direction { by }
                }
            }
            Order::Move {
                target: Target::None,
            } => UnitOrder::Stand,
            Order::Attack {
                target: Target::None,
            } => UnitOrder::Hold,
            Order::Attack {
                target: Target::Unit(target),
            } if self
                .of_wire(target)
                .is_some_and(|mark| self.rune.get(mark).is_some()) =>
            {
                self.deny_rune_order(unit, target);
                return;
            }
            // Following a rune is walking to it.
            Order::Move {
                target: Target::Unit(target),
            } if self
                .of_wire(target)
                .is_some_and(|mark| self.rune.get(mark).is_some()) =>
            {
                self.take_item(unit, target);
                return;
            }
            Order::Move {
                target: Target::Unit(target),
            }
            | Order::Attack {
                target: Target::Unit(target),
            } if self.interaction_at(unit, target).is_some() => {
                if let Some(slot) = self.interaction_at(unit, target) {
                    self.take_cast(unit, slot, Target::Unit(target));
                }
                return;
            }
            Order::Move {
                target: Target::Unit(target),
            } => {
                let Some(mark) = self.of_wire(target) else {
                    return;
                };
                UnitOrder::Follow {
                    target: mark,
                    last_seen: self.pos_of(mark),
                }
            }
            Order::Attack {
                target: Target::Unit(target),
            } => {
                let Some(mark) = self.of_wire(target) else {
                    return;
                };
                self.rouse_bystanders(unit, mark);
                UnitOrder::Attack {
                    target: mark,
                    last_seen: self.pos_of(mark),
                }
            }
            Order::Cast { slot, target } => {
                self.take_cast(unit, slot, target);
                return;
            }
            Order::Use { slot, target } => {
                self.order_cast(unit, PendingCast::Item { slot, target });
                return;
            }
            Order::Put { slot, target } => {
                self.put_item(unit, usize::from(slot.0), target);
                return;
            }
            Order::Take {
                target: Target::Unit(target),
            } => {
                self.take_item(unit, target);
                return;
            }
            Order::Take { .. }
            | Order::Learn { .. }
            | Order::LearnTalent { .. }
            | Order::LearnAttributeBonus
            | Order::Glyph
            | Order::Swap { .. }
            | Order::Sell { .. }
            | Order::SwitchWard { .. }
            | Order::Disassemble { .. }
            | Order::SetCombineLock { .. }
            | Order::Buy { .. }
            | Order::Cheat { .. } => return,
        };
        self.set_order(unit, wanted);
    }

    /// Where an entity stands, or the map's origin for one that stands
    /// nowhere.
    fn pos_of(&self, entity: Entity) -> bota_proto::Vec2 {
        self.transform
            .get(entity)
            .map_or(bota_proto::Vec2::ZERO, |t| t.pos)
    }

    /// Orders a cast. A spell draws neither creeps nor towers, whoever it
    /// is aimed at; an aimed cast takes the body over, so what it was doing
    /// is not returned to, while a cast at oneself leaves its order be.
    fn take_cast(&mut self, unit: Entity, slot: bota_proto::AbilitySlot, target: Target) {
        let order = Order::Cast { slot, target };
        if bota_core::ends_standing_order(order, Some(self.ability_in(unit, slot)), None) {
            self.set_order(unit, UnitOrder::Idle);
        }
        self.order_cast(unit, PendingCast::Ability { slot, target });
    }

    /// The unit an order is for, if the seat drives it. Naming nobody means
    /// the seat's own hero.
    pub fn driven_by(&self, slot: SlotId, named: Option<bota_proto::EntityId>) -> Option<Entity> {
        let seat = self.seat(slot)?;
        let Some(named) = named else {
            return seat.unit;
        };
        let unit = self.of_wire(named)?;
        (self.owner.get(unit) == Some(&slot)).then_some(unit)
    }

    /// Whether an order is for a seat's hero while it lies dead: naming
    /// nobody, or naming the hero itself.
    fn for_dead_hero(&self, slot: SlotId, named: Option<bota_proto::EntityId>) -> bool {
        self.seat(slot).is_some_and(|seat| {
            seat.dead() && named.is_none_or(|named| seat.unit.map(wire_id) == Some(named))
        })
    }

    /// The seat at a slot, if that slot is in the match.
    pub fn seat(&self, slot: SlotId) -> Option<&Seat> {
        self.seats.iter().find(|s| s.slot == slot)
    }

    /// Admission cannot read an inventory that preceding transactions have
    /// yet to change. Check authority for dependent requests here;
    /// [`Self::advance_checked`] rechecks every player request, including
    /// cooldowns and queue capacity, against the state left by its predecessors.
    pub(crate) fn check_incoming_order(
        &self,
        slot: SlotId,
        named: Option<bota_proto::EntityId>,
        order: &Order,
        queue: bool,
    ) -> Result<(), RejectReason> {
        if !inventory_dependent(*order) {
            return self.check_order(slot, named, order, queue);
        }
        self.driven_by(slot, named).ok_or(if named.is_some() {
            RejectReason::NotYourUnit
        } else {
            RejectReason::HeroDead
        })?;
        Ok(())
    }

    /// Whether a seat may issue an order right now.
    ///
    /// A seat with no body standing may order nothing, and a target it cannot
    /// see may as well not exist. Everything else is allowed to be asked for,
    /// whether or not it can be carried out.
    pub fn validate_order(
        &self,
        slot: SlotId,
        named: Option<bota_proto::EntityId>,
        order: &Order,
    ) -> Result<(), RejectReason> {
        if *order == Order::Glyph {
            return self.check_glyph(slot);
        }
        let Some(seat) = self.seat(slot) else {
            return Err(RejectReason::HeroDead);
        };
        if self.for_dead_hero(slot, named) {
            return self.check_while_down(seat, order);
        }
        // A named unit has to be one this seat drives; naming nobody means
        // its hero.
        let unit = match named {
            None => seat.unit.ok_or(RejectReason::HeroDead)?,
            Some(named) => self
                .driven_by(slot, Some(named))
                .ok_or(RejectReason::NotYourUnit)?,
        };
        if !self.alive(unit) {
            return Err(RejectReason::HeroDead);
        }
        match order {
            Order::Move { target } | Order::Attack { target } => {
                // Pointing an attack at one of your own is an order like any
                // other: what it cannot do is land, and that is settled when
                // targets are chosen.
                if let Target::Unit(named) = target {
                    self.seen_by(seat, *named)?;
                    // An order at a map objective casts its hidden ability, as
                    // Dota turns the click into one: it is taken on a cast's
                    // terms.
                    if let Some(slot) = self.interaction_at(unit, *named) {
                        return self.check_interaction(unit, seat, slot, target);
                    }
                }
                Ok(())
            }
            Order::Cast { slot, target } => self.check_cast(unit, seat, *slot, target),
            Order::Buy { item } => self.check_buy(seat, *item),
            Order::Sell { slot: named } => self.check_sale(unit, seat, usize::from(named.0)),
            Order::SwitchWard { slot: named } => self.check_switch(unit, usize::from(named.0)),
            Order::Disassemble { slot: named } => {
                self.disassembly_refusal(unit, seat, usize::from(named.0))
            }
            Order::SetCombineLock { slot: named, .. } => {
                self.lock_refusal(unit, seat, usize::from(named.0))
            }
            Order::Put {
                slot: named,
                target,
            } => self.check_put(unit, seat, usize::from(named.0), target),
            Order::Take { target } => self.check_take(unit, seat, target),
            Order::Swap { from, to } => self
                .swap_refusal(unit, seat, usize::from(from.0), usize::from(to.0))
                .map_or(Ok(()), Err),
            Order::Use { slot, target } => self.check_use(unit, seat, usize::from(slot.0), target),
            Order::Cheat { cheat } => self.check_cheat(slot, Some(unit), cheat),
            Order::LearnTalent { talent } => {
                bota_core::learning_refusal(self.can_learn_talent(unit, *talent))
            }
            Order::LearnAttributeBonus => {
                bota_core::learning_refusal(self.can_learn_attribute_bonus(unit))
            }
            _ => Ok(()),
        }
    }

    /// Whether a seat may issue an order, queued or not: as
    /// [`World::validate_order`], and a queued order needs room in its
    /// unit's queue.
    pub fn check_order(
        &self,
        slot: SlotId,
        named: Option<bota_proto::EntityId>,
        order: &Order,
        queue: bool,
    ) -> Result<(), RejectReason> {
        self.validate_order(slot, named, order)?;
        let full = queue
            && bota_core::order_queues(*order)
            && self
                .driven_by(slot, named)
                .and_then(|unit| self.queue.get(unit))
                .is_some_and(crate::game::OrderQueue::is_full);
        if full {
            return Err(RejectReason::QueueFull);
        }
        Ok(())
    }

    /// The unit an order of a seat is for, in the simulation's handles, or
    /// nothing for an order that needs no body: on one tick, a later
    /// unqueued order replaces the earlier ones of the same key.
    pub fn order_key(
        &self,
        slot: SlotId,
        named: Option<bota_proto::EntityId>,
        order: &Order,
    ) -> Option<bota_proto::EntityId> {
        if *order == Order::Glyph {
            return None;
        }
        self.driven_by(slot, named).map(wire_id)
    }

    /// Whether a seat may buy an item: one that exists, with gold enough
    /// for what it lacks of it, room for what it buys, and some left on its
    /// side's shelf.
    fn check_buy(&self, seat: &Seat, item: bota_proto::ItemId) -> Result<(), RejectReason> {
        if self.catalogue.sold(item).is_none() {
            return Err(RejectReason::UnknownItem);
        }
        let plan = self
            .purchase_plan(seat.slot, item)
            .ok_or(RejectReason::HeroDead)?;
        if seat.gold < plan.cost {
            return Err(RejectReason::NotEnoughGold);
        }
        plan.berths.map(|_| ())?;
        if !self.purchase_in_stock(seat.team, &plan.parts) {
            return Err(RejectReason::OutOfStock);
        }
        Ok(())
    }

    /// Whether a unit with a bag may take a seen item off the ground.
    fn check_take(&self, unit: Entity, seat: &Seat, target: &Target) -> Result<(), RejectReason> {
        let Target::Unit(named) = target else {
            return Err(RejectReason::WrongTargetKind);
        };
        let mark = self.seen_by(seat, *named)?;
        bota_core::take_refusal(
            self.rune.contains(mark),
            self.is_real_hero(unit),
            self.loot
                .get(mark)
                .is_some_and(|loot| self.may_receive_item(unit, loot.0.id)),
        )
    }

    /// The entity a seat names on the wire, while its side's view shows it:
    /// whatever a side was shown it may name, and nothing else.
    fn seen_by(&self, seat: &Seat, named: bota_proto::EntityId) -> Result<Entity, RejectReason> {
        let entity = self.of_wire(named).ok_or(RejectReason::UnknownTarget)?;
        if !self.shows(seat.team, entity) {
            return Err(RejectReason::UnknownTarget);
        }
        Ok(entity)
    }

    /// Whether a unit may cast the ability in one of its slots at a target
    /// right now.
    fn check_cast(
        &self,
        unit: Entity,
        seat: &Seat,
        slot: bota_proto::AbilitySlot,
        target: &Target,
    ) -> Result<(), RejectReason> {
        let ability = self.slot_ability(unit, slot)?;
        // A hidden ability is no slot a seat sees: only an order at what it
        // works on casts it.
        let def = bota_core::ability_def(ability.id)
            .filter(|def| !def.hidden)
            .ok_or(RejectReason::EmptySlot)?;
        self.cast_refusal(unit, seat, ability, def, target)
    }

    /// Whether a unit may cast the hidden ability of the map objective an
    /// order at it names, as a cast of it would be taken.
    fn check_interaction(
        &self,
        unit: Entity,
        seat: &Seat,
        slot: bota_proto::AbilitySlot,
        target: &Target,
    ) -> Result<(), RejectReason> {
        let ability = self.slot_ability(unit, slot)?;
        let def = bota_core::ability_def(ability.id).ok_or(RejectReason::EmptySlot)?;
        self.cast_refusal(unit, seat, ability, def, target)
    }

    /// The ability a unit has in a slot.
    fn slot_ability(
        &self,
        unit: Entity,
        slot: bota_proto::AbilitySlot,
    ) -> Result<crate::game::AbilityState, RejectReason> {
        self.abilities
            .get(unit)
            .and_then(|book| book.slots.get(usize::from(slot.0)))
            .copied()
            .ok_or(RejectReason::EmptySlot)
    }

    /// What a cast of a held ability at a target is refused for right now:
    /// what keeps it from going off at all ([`World::cast_blocked`], the
    /// rule the cast itself goes by), a disabled caster, a target it may not
    /// be aimed at, and too little mana.
    fn cast_refusal(
        &self,
        unit: Entity,
        seat: &Seat,
        ability: crate::game::AbilityState,
        def: &bota_core::AbilityDef,
        target: &Target,
    ) -> Result<(), RejectReason> {
        if let Some(reason) = self.cast_blocked(unit, ability, def) {
            return Err(reason);
        }
        if !aimed_right(def.aim, target) {
            return Err(RejectReason::WrongTargetKind);
        }
        if let Target::Unit(target) = target {
            let mark = self.seen_by(seat, *target)?;
            if def.at_an_enemy && !self.hostile(unit, mark) {
                return Err(RejectReason::WrongTargetKind);
            }
            if !self.aimable(unit, def, mark) {
                return Err(RejectReason::WrongTargetKind);
            }
        }
        if self.short_of_mana_for_cast(unit, ability) {
            return Err(RejectReason::NotEnoughMana);
        }
        Ok(())
    }

    /// Whether a bag slot holds the Ward Dispenser an order would switch.
    fn check_switch(&self, unit: Entity, at: usize) -> Result<(), RejectReason> {
        let held = self
            .inventory
            .get(unit)
            .and_then(|bag| bag.slot(at).copied().flatten());
        bota_core::switch_refusal(at, held.map(|held| held.id))
    }

    /// Whether a seat may sell or mark what one of its slots holds. Away
    /// from the shop the order marks rather than sells, so only a stack
    /// that is not the seat's own is refused.
    fn check_sale(&self, unit: Entity, seat: &Seat, at: usize) -> Result<(), RejectReason> {
        bota_core::sale_refusal(
            at,
            self.copied_inventory(unit),
            self.stack_at(unit, seat, at).map(Into::into),
            seat.slot,
            self.at_any_shop(unit),
        )
    }

    /// What a seat whose hero lies dead may still do: buy into its inventory
    /// and overflow stash, swap its carried slots, and sell back what it bought
    /// since the hero fell ("Items cannot be sold while
    /// dead, unless they were purchased during that death", against buyback
    /// abuse). Anything else waits for the hero.
    fn check_while_down(&self, seat: &Seat, order: &Order) -> Result<(), RejectReason> {
        match order {
            Order::Buy { item } => self.check_buy(seat, *item),
            // The hero lies at no shop: its own slots alone.
            Order::Swap { from, to } => {
                let hero = seat.unit.ok_or(RejectReason::HeroDead)?;
                let (from, to) = (usize::from(from.0), usize::from(to.0));
                self.swap_refusal(hero, seat, from, to).map_or(Ok(()), Err)
            }
            Order::Sell { slot } => self.sale_while_down(seat, usize::from(slot.0)).map(|_| ()),
            Order::Disassemble { slot } => {
                let hero = seat.unit.ok_or(RejectReason::HeroDead)?;
                self.disassembly_refusal(hero, seat, usize::from(slot.0))
            }
            Order::SetCombineLock { slot, .. } => {
                let hero = seat.unit.ok_or(RejectReason::HeroDead)?;
                self.lock_refusal(hero, seat, usize::from(slot.0))
            }
            Order::Cheat { cheat } => self.check_cheat(seat.slot, None, cheat),
            _ => Err(RejectReason::HeroDead),
        }
    }

    /// An inventory or stash slot a seat whose hero lies dead may sell from: one holding
    /// a stack of its own bought since the hero fell (Dota: "Items cannot be
    /// sold while dead, unless they were purchased during that death").
    pub(crate) fn sale_while_down(&self, seat: &Seat, at: usize) -> Result<usize, RejectReason> {
        let fell = seat.fell;
        let hero = seat.unit.ok_or(RejectReason::HeroDead)?;
        self.check_sale(hero, seat, at)?;
        let held = self
            .stack_at(hero, seat, at)
            .ok_or(RejectReason::EmptySlot)?;
        if held.bought_tick < fell {
            return Err(RejectReason::HeroDead);
        }
        Ok(at)
    }

    /// Whether a unit may lay what a bag slot holds on open ground, underfoot
    /// or into a seen ally's bag.
    fn check_put(
        &self,
        unit: Entity,
        seat: &Seat,
        at: usize,
        target: &Target,
    ) -> Result<(), RejectReason> {
        let held = self.held_in_bag(unit, at);
        bota_core::put_item_refusal(at, self.copied_inventory(unit), held.map(Into::into))?;
        match target {
            Target::None => Ok(()),
            Target::Pos(pos) => self
                .clearance
                .stands_clear(*pos)
                .then_some(())
                .ok_or(RejectReason::ClosedGround),
            Target::Unit(target) => {
                let to = self.seen_by(seat, *target)?;
                bota_core::put_recipient_refusal(
                    to == unit,
                    self.team.get(to) == Some(&seat.team),
                    held.is_some_and(|held| self.may_receive_item(to, held.id)),
                )
            }
        }
    }

    /// Whether a unit may use the item in one of its inventory slots at a
    /// target right now.
    fn check_use(
        &self,
        unit: Entity,
        seat: &Seat,
        at: usize,
        target: &Target,
    ) -> Result<(), RejectReason> {
        if in_stash(at) || in_backpack(at) {
            return Err(RejectReason::WrongTargetKind);
        }
        let stack = self
            .inventory
            .get(unit)
            .and_then(|bag| bag.slot(at))
            .copied()
            .flatten()
            .ok_or(RejectReason::EmptySlot)?;
        let def = item_def(stack.id).ok_or(RejectReason::EmptySlot)?;
        if self.muted(unit) {
            return Err(RejectReason::Disabled);
        }
        let aim = self.usable(unit, &stack, def)?;
        // What is given away waits for nothing and costs nothing; what lands on an ally is a use.
        let ally = self.ally_use_on(unit, def, target);
        let gives = ally.is_some_and(|ally| matches!(ally, AllyUse::Give { .. }));
        self.ready(unit, &stack, def, gives)?;
        // What the side cannot see is refused as a dead target, whatever it is aimed for.
        if ally.is_some()
            && let Target::Unit(id) = target
        {
            self.seen_by(seat, *id)?;
        }
        // An item aimed at a unit already takes any unit (and does nothing for one that is not an
        // ally); one that is aimed at nothing or the ground takes an ally as a different use.
        let aimed = if let Some(ally) = ally.filter(|_| aim != Aim::Unit) {
            matches!(target, Target::Unit(id) if self.takes_ally_use(unit, seat, *id, ally))
        } else if def.mana_deficit {
            matches!(target, Target::None) || *target == Target::Unit(crate::game::wire_id(unit))
        } else {
            aimed_right(aim, target) || (def.ally_target && matches!(target, Target::Unit(_)))
        };
        if !aimed {
            return Err(RejectReason::WrongTargetKind);
        }
        if let Target::Unit(target) = target {
            self.seen_by(seat, *target)?;
        }
        if !gives && self.short_of_mana_for(unit, stack.id) {
            return Err(RejectReason::NotEnoughMana);
        }
        // A channel is no bar: the order breaks it off, as in Dota.
        if self.held(unit) || self.feared(unit) {
            return Err(RejectReason::Disabled);
        }
        if def.mana_deficit && !self.can_replenish_mana(unit, *target) {
            return Err(RejectReason::NotReady);
        }
        if aim == bota_proto::Aim::Building
            && def
                .scroll()
                .is_none_or(|scroll| self.scroll_destination(unit, *target, scroll).is_none())
        {
            return Err(RejectReason::WrongTargetKind);
        }
        Ok(())
    }

    /// What a use of `def` aimed at `target` does for an ally, when the target is another unit
    /// and the item has something to do for one.
    pub(crate) fn ally_use_on(
        &self,
        unit: Entity,
        def: &ItemDef,
        target: &Target,
    ) -> Option<AllyUse> {
        match (def.ally, target) {
            (Some(ally), Target::Unit(id)) if *id != crate::game::wire_id(unit) => Some(ally),
            _ => None,
        }
    }

    /// Whether a seen unit is one an ally use of this kind may be aimed at: another unit of the
    /// user's side, a hero for an effect and any unit with a bag for a give.
    fn takes_ally_use(
        &self,
        unit: Entity,
        seat: &Seat,
        id: bota_proto::EntityId,
        kind: AllyUse,
    ) -> bool {
        let Ok(to) = self.seen_by(seat, id) else {
            return false;
        };
        if to == unit || self.team.get(to) != Some(&seat.team) || !self.alive(to) {
            return false;
        }
        match kind {
            AllyUse::Give { .. } => self.inventory.get(to).is_some(),
            AllyUse::Effect { .. } => self.kind.get(to) == Some(&bota_proto::UnitKind::Hero),
        }
    }

    /// The entity behind a handle from the wire, while it still stands.
    pub fn of_wire(&self, id: bota_proto::EntityId) -> Option<Entity> {
        self.entities
            .resolve(crate::game::Index(id.idx), id.generation)
    }

    /// The match result, when complete. `Team::Neutral` denotes a Map2 draw.
    pub fn victor(&self) -> Option<Team> {
        self.winner
    }

    /// Final numbers for every seat.
    pub fn match_stats(&self) -> MatchStats {
        MatchStats {
            duration: self.tick,
            leaves: Vec::new(),
            slots: self
                .seats
                .iter()
                .map(|s| SlotStats {
                    slot: s.slot,
                    kills: s.kills,
                    deaths: s.deaths,
                    assists: s.assists,
                    last_hits: s.last_hits,
                    denies: s.denies,
                    net_worth: s.net_worth,
                    hero_damage: s.hero_damage,
                    structure_damage: s.structure_damage,
                })
                .collect(),
        }
    }
}

/// Transactions mutate storage immediately, beside the body's queued action.
/// Their validation and application must share the same ordered world state.
fn inventory_transaction(order: Order) -> bool {
    matches!(
        order,
        Order::Buy { .. }
            | Order::Sell { .. }
            | Order::Swap { .. }
            | Order::SwitchWard { .. }
            | Order::Disassemble { .. }
            | Order::SetCombineLock { .. }
    )
}

/// Inventory-dependent actions validate against the same ordered state as
/// storage transactions; reading an item or its bonuses is not an exception.
fn inventory_dependent(order: Order) -> bool {
    inventory_transaction(order)
        || matches!(
            order,
            Order::Use { .. } | Order::Put { .. } | Order::Take { .. } | Order::Cast { .. }
        )
}
