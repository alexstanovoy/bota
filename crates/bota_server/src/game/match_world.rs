//! The surface a match runs a world through: what the game loop asks of it.

use bota_core::{AllyUse, ItemDef, clamp_to_map, hero_spawn_pos, item_def, map_of, rules};
use bota_proto::{Aim, ItemId, MatchStats, Order, RejectReason, SlotId, SlotStats, Target, Team};

use crate::game::{
    ActionPhase, ActionState, BAG_SLOTS, Command, Entity, Event, MatchConfig, MatchRng,
    PendingCast, Seat, UnitOrder, World, in_backpack, in_stash, in_town_portal, wire_id,
};

/// Whether an order aims at the kind of thing something takes.
pub fn aimed_right(aim: Aim, target: &Target) -> bool {
    match aim {
        Aim::Own => matches!(target, Target::None),
        // A tree is named by the ground it stands on; which one was meant is
        // settled where the use is carried out.
        Aim::Point | Aim::Tree => matches!(target, Target::Pos(_)),
        // A building is named by a spot near it, by itself, or, by naming
        // nothing, the fountain.
        Aim::Building => true,
        Aim::Unit => matches!(target, Target::Unit(_)),
    }
}

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
                rules::SCROLL_FIRST_WAIT_TICKS,
            ));
            world.seats.push(seat);
        }
        for seat in 0..world.seats.len() {
            world.stand_up_courier(seat);
        }
        world.settle();
        world
    }

    /// One tick of the match: every command taken in the order given, then
    /// [`World::step`]. A completed Map2 ignores commands and advances no
    /// further. What it refused is left in [`World::refused`].
    pub fn advance(&mut self, cmds: &[Command]) -> Vec<Event> {
        self.refused.clear();
        if self.map2_finished() {
            return Vec::new();
        }
        let mut events = Vec::new();
        self.economy_events_due(&mut events);
        for cmd in cmds {
            self.take_order(cmd, &mut events);
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
        let is_cast = |order: &Order| matches!(order, Order::Cast { .. } | Order::Use { .. });
        let mut waiting = self.waiting.remove(unit).unwrap_or_default();
        if !is_cast(&order) || !waiting.iter().all(|queued| is_cast(&queued.order)) {
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
            Order::LearnTalent { talent } => self
                .can_learn_talent(unit, *talent)
                .then_some(())
                .ok_or(RejectReason::CannotLearn),
            Order::LearnAttributeBonus => self
                .can_learn_attribute_bonus(unit)
                .then_some(())
                .ok_or(RejectReason::CannotLearn),
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
        if plan.berths.is_none() {
            return Err(RejectReason::InventoryFull);
        }
        if !self.in_stock(seat.team, item) {
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
        // A rune is taken by a hero and no other unit.
        let takes = if self.rune.get(mark).is_some() {
            self.is_real_hero(unit)
        } else {
            self.loot.get(mark).is_some() && self.inventory.get(unit).is_some()
        };
        if !takes {
            return Err(RejectReason::WrongTargetKind);
        }
        Ok(())
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
        let held = self
            .abilities
            .get(unit)
            .and_then(|book| book.slots.get(usize::from(slot.0)))
            .copied()
            .ok_or(RejectReason::EmptySlot)?;
        // A hidden ability is no slot a seat sees: only an order at what it
        // works on casts it.
        let def = bota_core::ability_def(held.id)
            .filter(|def| !def.hidden)
            .ok_or(RejectReason::EmptySlot)?;
        if def.passive || self.wears_illusion(unit) {
            return Err(RejectReason::NotCastable);
        }
        if held.level == 0 {
            return Err(RejectReason::NotLearned);
        }
        if held.cooldown > 0 {
            return Err(RejectReason::OnCooldown);
        }
        // A channel is no bar: the order breaks it off, as in Dota.
        if self.held(unit) || self.feared(unit) {
            return Err(RejectReason::Disabled);
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
        let held_mana = self.mana.get(unit).map_or(0, |mana| mana.mana.to_int());
        if held_mana < self.ability_mana_cost(unit, held.id, held.level) {
            return Err(RejectReason::NotEnoughMana);
        }
        Ok(())
    }

    /// Whether a bag slot holds the Ward Dispenser an order would switch.
    fn check_switch(&self, unit: Entity, at: usize) -> Result<(), RejectReason> {
        if in_stash(at) {
            return Err(RejectReason::NotInBag);
        }
        let held = self
            .inventory
            .get(unit)
            .and_then(|bag| bag.slot(at).copied().flatten())
            .ok_or(RejectReason::EmptySlot)?;
        if held.id == ItemId::WARD_DISPENSER {
            Ok(())
        } else {
            Err(RejectReason::NotCastable)
        }
    }

    /// Whether a seat may sell or mark what one of its slots holds. Away
    /// from the shop the order marks rather than sells, so only a stack
    /// that is not the seat's own is refused.
    fn check_sale(&self, unit: Entity, seat: &Seat, at: usize) -> Result<(), RejectReason> {
        let held = if in_stash(at) {
            seat.stash.slots.get(at - BAG_SLOTS).copied().flatten()
        } else {
            self.inventory
                .get(unit)
                .and_then(|bag| bag.slot(at).copied().flatten())
        };
        let held = held.ok_or(RejectReason::EmptySlot)?;
        if held.owner != seat.slot {
            return Err(RejectReason::NotYourItem);
        }
        if held.free_charges > 0 {
            return Err(RejectReason::Unsellable);
        }
        if held.rune.is_some() {
            return Err(RejectReason::HoldsRune);
        }
        // Nothing is ever carried out of the town portal slot, so it sells
        // only at the shop and is never marked.
        if in_town_portal(at) && !self.at_shop(unit) {
            return Err(RejectReason::NotAtShop);
        }
        Ok(())
    }

    /// What a seat whose hero lies dead may still do, as in Dota (checked in
    /// the Dota 2 client, 7.41, 2026-10-24): buy (into the stash), swap what
    /// the hero carried between its slots, and sell back what it bought since
    /// the hero fell, which waits in the stash ("Items cannot be sold while
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

    /// The stash slot a seat whose hero lies dead may sell from: one holding
    /// a stack of its own bought since the hero fell (Dota: "Items cannot be
    /// sold while dead, unless they were purchased during that death").
    pub(crate) fn sale_while_down(&self, seat: &Seat, at: usize) -> Result<usize, RejectReason> {
        let fell = seat.fell;
        if !in_stash(at) {
            return Err(RejectReason::HeroDead);
        }
        let stash_slot = at - BAG_SLOTS;
        let held = seat.stash.slots.get(stash_slot).copied().flatten();
        let held = held.ok_or(RejectReason::EmptySlot)?;
        if held.owner != seat.slot {
            return Err(RejectReason::NotYourItem);
        }
        if held.free_charges > 0 {
            return Err(RejectReason::Unsellable);
        }
        if held.bought_tick < fell {
            return Err(RejectReason::HeroDead);
        }
        Ok(stash_slot)
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
        if at >= BAG_SLOTS {
            return Err(RejectReason::NotInBag);
        }
        if !self.holds(unit, seat, at) {
            return Err(RejectReason::EmptySlot);
        }
        if self.holds_rune_at(unit, at) {
            return Err(RejectReason::HoldsRune);
        }
        match target {
            Target::None => Ok(()),
            Target::Pos(pos) => self
                .clearance
                .stands_clear(*pos)
                .then_some(())
                .ok_or(RejectReason::ClosedGround),
            Target::Unit(target) => {
                let to = self.seen_by(seat, *target)?;
                if to == unit
                    || self.team.get(to) != Some(&seat.team)
                    || self.inventory.get(to).is_none()
                {
                    return Err(RejectReason::WrongTargetKind);
                }
                Ok(())
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
        // A stack just out of the backpack does nothing until it has woken up.
        if stack.mute > 0 {
            return Err(RejectReason::NotReady);
        }
        let aim = def.aim.ok_or(RejectReason::NotCastable)?;
        if self.wears_illusion(unit) {
            return Err(RejectReason::NotCastable);
        }
        // What is given away waits for nothing and costs nothing; what lands on an ally is a use.
        let ally = self.ally_use_on(unit, def, target);
        let gives = ally.is_some_and(|ally| matches!(ally, AllyUse::Give { .. }));
        if !gives && (stack.cooldown > 0 || (def.shared_wait && self.owes_wait(unit, stack.id))) {
            return Err(RejectReason::OnCooldown);
        }
        // What holds a rune lets it out, and needs no charge for that.
        if (def.charges > 0 || def.cast_charges > 0) && stack.charges == 0 && stack.rune.is_none() {
            return Err(RejectReason::NoCharges);
        }
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
        if !gives
            && self.mana.get(unit).map_or(0, |pool| pool.mana.to_int())
                < self.item_mana_cost(unit, stack.id)
        {
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

    /// Whether a bag slot of a unit holds a Bottle with a rune in it.
    fn holds_rune_at(&self, unit: Entity, at: usize) -> bool {
        self.held_in_bag(unit, at)
            .is_some_and(|stack| stack.rune.is_some())
    }

    /// Whether one of a seat's slots holds anything at all.
    fn holds(&self, unit: Entity, seat: &Seat, slot: usize) -> bool {
        if in_stash(slot) {
            return seat
                .stash
                .slots
                .get(slot - BAG_SLOTS)
                .is_some_and(|held| held.is_some());
        }
        self.inventory
            .get(unit)
            .and_then(|bag| bag.slot(slot))
            .is_some_and(|held| held.is_some())
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

/// Whether an order is a stop or a hold: the orders that break a cast off in
/// its cast point.
fn is_halt(order: Order) -> bool {
    matches!(
        order,
        Order::Move {
            target: Target::None
        } | Order::Attack {
            target: Target::None
        }
    )
}
