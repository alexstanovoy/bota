//! The surface a match runs a world through: what the game loop asks of it.

use bota_proto::{
    Aim, Cheat, MatchStats, Order, RejectReason, SlotId, SlotStats, Target, Team, UnitKind,
};

use crate::game::{BAG_SLOTS, Command, Event, MatchConfig, MatchRng, hero_spawn_pos, in_backpack};
use crate::game::{Entity, PendingCast, Seat, UnitOrder, World};
use crate::game::{in_stash, item_def, map_of, rules};

/// Whether an order aims at the kind of thing something takes.
pub fn aimed_right(aim: Aim, target: &Target) -> bool {
    match aim {
        Aim::Own => matches!(target, Target::None),
        // A tree and a landing spot are both named by the ground they stand
        // on; which one was meant is settled where the use is carried out.
        Aim::Point | Aim::Tree | Aim::Building => matches!(target, Target::Pos(_)),
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
        world.immortal_structures = cfg.immortal_structures;
        world.immortal_heroes = cfg.immortal_heroes;
        world.spawn_modifiers = cfg.spawn_modifiers.clone();
        world.apply_spawn_modifiers_to_all();
        for pick in &cfg.picks {
            let at = hero_spawn_pos(map, pick.team);
            let hero = world.spawn_hero(pick.team, at, pick.slot, pick.hero);
            let mut seat = Seat::new(
                pick.slot,
                pick.team,
                pick.hero,
                rules::STARTING_GOLD,
                rules::STASH_SLOTS,
            );
            seat.unit = Some(hero);
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
    /// further.
    pub fn advance(&mut self, cmds: &[Command]) -> Vec<Event> {
        if self.map2_finished() {
            return Vec::new();
        }
        let mut events = Vec::new();
        for cmd in cmds {
            self.take_order(cmd, &mut events);
        }
        events.extend(self.step());
        events
    }

    /// Hands one order to the body of the seat that gave it.
    fn take_order(&mut self, cmd: &Command, events: &mut Vec<Event>) {
        let Some(unit) = self.driven_by(cmd.slot, cmd.unit) else {
            return;
        };
        // Learning, the bag, the shop and cheats interrupt nothing the body
        // is doing.
        match cmd.order {
            Order::Learn { slot } => {
                self.learn(unit, usize::from(slot.0));
            }
            Order::Swap { from, to } => {
                self.move_item(cmd.slot, unit, usize::from(from.0), usize::from(to.0));
            }
            Order::Sell { slot: at } => {
                self.sell_item(cmd.slot, unit, usize::from(at.0));
            }
            Order::Buy { item } => {
                self.buy(cmd.slot, item, events);
            }
            Order::Cheat { cheat } => {
                self.cheat(cmd.slot, unit, cheat, events);
            }
            order => {
                self.interrupt(unit, order);
                self.take_body_order(unit, order);
            }
        }
    }

    /// What an order to the body breaks off: the action under way, as an
    /// animation cancel, a held cast, and an errand unless a courier keeps
    /// it for a burst or a shield.
    fn interrupt(&mut self, unit: Entity, order: Order) {
        self.cancel_action(unit);
        self.handling.remove(unit);
        if let Some(orders) = self.orders.get_mut(unit) {
            orders.pending = None;
        }
        let keeps_errand = self.kind.get(unit) == Some(&UnitKind::Courier)
            && matches!(order, Order::Cast { slot, target: Target::None }
                if matches!(self.ability_in(unit, slot), crate::game::ability::BURST | crate::game::ability::SHIELD));
        if !keeps_errand && let Some(errand) = self.errand.get_mut(unit) {
            *errand = crate::game::Errand::None;
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
            Order::Move {
                target: Target::None,
            } => UnitOrder::Stand,
            Order::Attack {
                target: Target::None,
            } => UnitOrder::Hold,
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
            | Order::Swap { .. }
            | Order::Sell { .. }
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

    /// Orders a cast. A spell aimed at somebody is as plain to the creeps as
    /// a swing at them; an aimed cast takes the body over, so what it was
    /// doing is not returned to, while a cast at oneself leaves its order be.
    fn take_cast(&mut self, unit: Entity, slot: bota_proto::AbilitySlot, target: Target) {
        if let Target::Unit(target) = target
            && let Some(mark) = self.of_wire(target)
        {
            self.rouse_by_cast(unit, mark);
        }
        let aimed = crate::game::ability_def(self.ability_in(unit, slot))
            .is_some_and(|def| def.aim != Aim::Own);
        if aimed {
            self.set_order(unit, UnitOrder::Idle);
        }
        self.order_cast(unit, PendingCast::Ability { slot, target });
    }

    /// The unit an order is for, if the seat drives it. Naming nobody means
    /// the seat's own hero.
    pub fn driven_by(&self, slot: SlotId, named: Option<bota_proto::EntityId>) -> Option<Entity> {
        let seat = self.seats.iter().find(|seat| seat.slot == slot)?;
        let Some(named) = named else {
            return seat.unit;
        };
        let unit = self.of_wire(named)?;
        (self.owner.get(unit) == Some(&slot)).then_some(unit)
    }

    /// The seat at a slot, if that slot is in the match.
    pub fn seat(&self, slot: SlotId) -> Option<&Seat> {
        self.seats.iter().find(|s| s.slot == slot)
    }

    /// The unit a cheat names, or why it cannot land there.
    ///
    /// Nothing names the unit the cheat was issued for. Only something that
    /// is a unit can be aimed at; a position names nothing at all.
    pub fn cheat_target(&self, unit: Entity, target: Target) -> Result<Entity, RejectReason> {
        match target {
            Target::None => Ok(unit),
            Target::Unit(id) => {
                let mark = self.of_wire(id).ok_or(RejectReason::UnknownTarget)?;
                if self.def.get(mark).is_none() {
                    return Err(RejectReason::UnknownTarget);
                }
                Ok(mark)
            }
            Target::Pos(_) => Err(RejectReason::WrongTargetKind),
        }
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
        let Some(seat) = self.seats.iter().find(|s| s.slot == slot) else {
            return Err(RejectReason::HeroDead);
        };
        // A named unit has to be one this seat drives; naming nobody means
        // its hero, and a seat with no hero standing drives nothing.
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
            Order::Put {
                slot: named,
                target,
            } => self.check_put(unit, seat, usize::from(named.0), target),
            Order::Take { target } => self.check_take(unit, seat, target),
            Order::Swap { from, to } => {
                let (from, to) = (usize::from(from.0), usize::from(to.0));
                if from == to || !self.holds(unit, seat, from) {
                    return Err(RejectReason::EmptySlot);
                }
                if (in_stash(from) || in_stash(to)) && !self.at_shop(unit) {
                    return Err(RejectReason::NotAtShop);
                }
                Ok(())
            }
            Order::Use { slot, target } => self.check_use(unit, seat, usize::from(slot.0), target),
            Order::Cheat { cheat } => self.check_cheat(unit, cheat),
            _ => Ok(()),
        }
    }

    /// Whether a seat may buy an item: one that exists, with gold enough
    /// for what it lacks of it and room for what it buys.
    fn check_buy(&self, seat: &Seat, item: bota_proto::ItemId) -> Result<(), RejectReason> {
        if item_def(item).is_none() {
            return Err(RejectReason::UnknownItem);
        }
        let plan = self
            .purchase_plan(seat.slot, item)
            .ok_or(RejectReason::HeroDead)?;
        if seat.gold < plan.cost {
            return Err(RejectReason::NotEnoughGold);
        }
        if !plan.fits {
            return Err(RejectReason::InventoryFull);
        }
        Ok(())
    }

    /// Whether a unit with a bag may take a seen item off the ground.
    fn check_take(&self, unit: Entity, seat: &Seat, target: &Target) -> Result<(), RejectReason> {
        let Target::Unit(named) = target else {
            return Err(RejectReason::WrongTargetKind);
        };
        let mark = self.seen_by(seat, *named)?;
        if self.loot.get(mark).is_none() || self.inventory.get(unit).is_none() {
            return Err(RejectReason::WrongTargetKind);
        }
        Ok(())
    }

    /// The entity a seat names on the wire, while its side sees it.
    fn seen_by(&self, seat: &Seat, named: bota_proto::EntityId) -> Result<Entity, RejectReason> {
        let entity = self.of_wire(named).ok_or(RejectReason::UnknownTarget)?;
        if !self.can_see(seat.team, entity) {
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
        let def = crate::game::ability_def(held.id).ok_or(RejectReason::EmptySlot)?;
        if def.passive {
            return Err(RejectReason::NotCastable);
        }
        if held.level == 0 {
            return Err(RejectReason::NotLearned);
        }
        if held.cooldown > 0 {
            return Err(RejectReason::OnCooldown);
        }
        if self.held(unit) || self.feared(unit) || self.is_channelling(unit) {
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
        }
        let held_mana = self.mana.get(unit).map_or(0, |mana| mana.mana.to_int());
        if held_mana < self.ability_mana_cost(unit, held.id, held.level) {
            return Err(RejectReason::NotEnoughMana);
        }
        Ok(())
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
                .and_then(|bag| bag.slots.get(at).copied().flatten())
        };
        let held = held.ok_or(RejectReason::EmptySlot)?;
        if held.owner != seat.slot {
            return Err(RejectReason::NotYourItem);
        }
        Ok(())
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
        match target {
            Target::None => Ok(()),
            Target::Pos(pos) => {
                if self.clearance.stands_clear(*pos) {
                    Ok(())
                } else {
                    Err(RejectReason::ClosedGround)
                }
            }
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
            .and_then(|bag| bag.slots.get(at))
            .copied()
            .flatten()
            .ok_or(RejectReason::EmptySlot)?;
        let def = item_def(stack.id).ok_or(RejectReason::EmptySlot)?;
        // A stack just out of the backpack does nothing until it has woken up.
        if stack.mute > 0 {
            return Err(RejectReason::NotReady);
        }
        let aim = def.aim.ok_or(RejectReason::NotCastable)?;
        if stack.cooldown > 0 || (def.shared_wait && self.owes_wait(unit, stack.id)) {
            return Err(RejectReason::OnCooldown);
        }
        if (def.charges > 0 || def.cast_charges > 0) && stack.charges == 0 {
            return Err(RejectReason::NoCharges);
        }
        let aimed = if def.mana_deficit {
            matches!(target, Target::None) || *target == Target::Unit(crate::game::wire_id(unit))
        } else {
            aimed_right(aim, target)
        };
        if !aimed {
            return Err(RejectReason::WrongTargetKind);
        }
        if let Target::Unit(target) = target {
            self.seen_by(seat, *target)?;
        }
        if self.mana.get(unit).map_or(0, |pool| pool.mana.to_int())
            < self.item_mana_cost(unit, stack.id)
        {
            return Err(RejectReason::NotEnoughMana);
        }
        if self.held(unit) || self.feared(unit) || self.is_channelling(unit) {
            return Err(RejectReason::Disabled);
        }
        if def.mana_deficit && !self.can_replenish_mana(unit, *target) {
            return Err(RejectReason::NotReady);
        }
        Ok(())
    }

    /// Whether a cheat may be issued: cheats on, and a modifier cheat
    /// bounded and aimed at a unit.
    fn check_cheat(&self, unit: Entity, cheat: &Cheat) -> Result<(), RejectReason> {
        if !self.cheats {
            return Err(RejectReason::NoCheats);
        }
        match cheat {
            Cheat::ApplyModifier {
                target,
                spec,
                ticks,
            } => {
                if !spec.is_bounded() || !bota_proto::modifier_ticks_bounded(*ticks) {
                    return Err(RejectReason::BadCheat);
                }
                self.cheat_target(unit, *target)?;
            }
            Cheat::ClearModifiers { target } => {
                self.cheat_target(unit, *target)?;
            }
            _ => {}
        }
        Ok(())
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
            .and_then(|bag| bag.slots.get(slot))
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
