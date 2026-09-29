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
    /// A world at tick zero for a match: the map standing, a seat per player,
    /// and a hero at each fountain.
    ///
    /// No camp is filled; the jungle has not been carried over yet.
    pub fn for_match(cfg: &MatchConfig, rng: MatchRng) -> World {
        if let Err(error) = cfg.validate() {
            panic!("the match setup was refused: {error}");
        }
        let map = map_of(cfg.map);
        let mut world = World::on_map(map);
        world.rng = rng;
        world.cheats = cfg.cheats;
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

    /// One tick of the match.
    ///
    /// Only orders that send a body somewhere are carried over; anything else
    /// is dropped. A completed Map2 ignores commands and advances no further.
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
        // Business with the bag and the shop asks nothing of the body, so it
        // interrupts nothing the body is doing.
        match cmd.order {
            Order::Learn { slot } => {
                self.learn(unit, usize::from(slot.0), events);
                return;
            }
            Order::Swap { from, to } => {
                self.move_item(cmd.slot, unit, usize::from(from.0), usize::from(to.0));
                return;
            }
            Order::Sell { slot: at } => {
                self.sell_item(cmd.slot, unit, usize::from(at.0));
                return;
            }
            Order::Buy { item } => {
                self.buy(cmd.slot, item, events);
                return;
            }
            Order::Cheat { cheat } => {
                self.cheat(cmd.slot, unit, cheat, events);
                return;
            }
            _ => {}
        }
        // An order is an animation cancel: the recovery after a swing ends
        // with it, and so does an ability or item that runs on. Giving up a
        // swing that has not landed is the attack cycle's own business.
        self.cancel_action(unit);
        // An order also takes a held cast with it before starting another.
        self.handling.remove(unit);
        if let Some(orders) = self.orders.get_mut(unit) {
            orders.pending = None;
        }
        let keeps_errand = self.kind.get(unit) == Some(&UnitKind::Courier)
            && matches!(cmd.order, Order::Cast { slot, target: Target::None }
                if matches!(self.ability_in(unit, slot), crate::game::ability::BURST | crate::game::ability::SHIELD));
        if !keeps_errand && let Some(errand) = self.errand.get_mut(unit) {
            *errand = crate::game::Errand::None;
        }
        let wanted = match cmd.order {
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
                let at = self
                    .transform
                    .get(mark)
                    .map_or(bota_proto::Vec2::ZERO, |t| t.pos);
                UnitOrder::Follow {
                    target: mark,
                    last_seen: at,
                }
            }
            Order::Attack {
                target: Target::Unit(target),
            } => {
                let Some(mark) = self.of_wire(target) else {
                    return;
                };
                self.rouse_bystanders(unit, mark);
                let at = self
                    .transform
                    .get(mark)
                    .map_or(bota_proto::Vec2::ZERO, |t| t.pos);
                UnitOrder::Attack {
                    target: mark,
                    last_seen: at,
                }
            }
            Order::Cast { slot, target } => {
                // A spell aimed at somebody is as plain to the creeps as a
                // swing at them.
                if let Target::Unit(target) = target
                    && let Some(mark) = self.of_wire(target)
                {
                    self.rouse_by_cast(unit, mark);
                }
                // An aimed cast takes the body over: what it was doing
                // before is not returned to once the cast has gone off. A
                // cast at oneself asks nothing of the body and leaves its
                // order be.
                let aimed = crate::game::ability_def(self.ability_in(unit, slot))
                    .is_some_and(|def| def.aim != Aim::Own);
                if aimed {
                    self.set_order(unit, UnitOrder::Idle);
                }
                self.order_cast(unit, PendingCast::Ability { slot, target });
                return;
            }
            Order::Use { slot, target } => {
                self.order_cast(unit, PendingCast::Item { slot, target });
                return;
            }
            // Taken before the body was interrupted.
            Order::Learn { .. }
            | Order::Swap { .. }
            | Order::Sell { .. }
            | Order::Buy { .. }
            | Order::Cheat { .. } => {
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
            Order::Take { .. } => return,
        };
        self.set_order(unit, wanted);
    }

    /// The unit an order is for, if the seat drives it.
    ///
    /// Naming nobody means the seat's own hero, which is what most orders
    /// are for. Naming anything a seat does not drive is nobody at all.
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
                if let Target::Unit(named) = target {
                    let Some(entity) = self.of_wire(*named) else {
                        return Err(RejectReason::UnknownTarget);
                    };
                    if !self.can_see(seat.team, entity) {
                        return Err(RejectReason::UnknownTarget);
                    }
                    // Pointing an attack at one of your own is an order like
                    // any other: what it cannot do is land, and that is
                    // settled when targets are chosen. Turning it down here
                    // would take the creeps' answer to it with it.
                }
                Ok(())
            }
            Order::Cast { slot, target } => {
                let held = self
                    .abilities
                    .get(unit)
                    .and_then(|book| book.slots.get(usize::from(slot.0)))
                    .copied();
                let Some(held) = held else {
                    return Err(RejectReason::EmptySlot);
                };
                let Some(def) = crate::game::ability_def(held.id) else {
                    return Err(RejectReason::EmptySlot);
                };
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
                    let Some(mark) = self.of_wire(*target) else {
                        return Err(RejectReason::UnknownTarget);
                    };
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
            Order::Buy { item } => {
                if item_def(*item).is_none() {
                    return Err(RejectReason::UnknownItem);
                }
                let plan = self
                    .purchase_plan(slot, *item)
                    .ok_or(RejectReason::HeroDead)?;
                if seat.gold < plan.cost {
                    return Err(RejectReason::NotEnoughGold);
                }
                if !plan.fits {
                    return Err(RejectReason::InventoryFull);
                }
                Ok(())
            }
            Order::Sell { slot: named } => {
                let at = usize::from(named.0);
                let held = if in_stash(at) {
                    seat.stash.slots.get(at - BAG_SLOTS).copied().flatten()
                } else {
                    self.inventory
                        .get(unit)
                        .and_then(|bag| bag.slots.get(at).copied().flatten())
                };
                let Some(held) = held else {
                    return Err(RejectReason::EmptySlot);
                };
                // Away from the shop the order marks rather than sells, so
                // there is no place it is refused for — only a stack that is
                // not this seat's to sell.
                if held.owner != slot {
                    return Err(RejectReason::NotYourItem);
                }
                Ok(())
            }
            Order::Put {
                slot: named,
                target,
            } => {
                let at = usize::from(named.0);
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
                        let Some(to) = self.of_wire(*target) else {
                            return Err(RejectReason::UnknownTarget);
                        };
                        if !self.can_see(seat.team, to) {
                            return Err(RejectReason::UnknownTarget);
                        }
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
            Order::Take { target } => {
                let Target::Unit(named) = target else {
                    return Err(RejectReason::WrongTargetKind);
                };
                let Some(mark) = self.of_wire(*named) else {
                    return Err(RejectReason::UnknownTarget);
                };
                if !self.can_see(seat.team, mark) {
                    return Err(RejectReason::UnknownTarget);
                }
                if self.loot.get(mark).is_none() || self.inventory.get(unit).is_none() {
                    return Err(RejectReason::WrongTargetKind);
                }
                Ok(())
            }
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
            Order::Use { slot, target } => {
                let at = usize::from(slot.0);
                if in_stash(at) || in_backpack(at) {
                    return Err(RejectReason::WrongTargetKind);
                }
                let stack = self
                    .inventory
                    .get(unit)
                    .and_then(|bag| bag.slots.get(at))
                    .copied()
                    .flatten();
                let Some(stack) = stack else {
                    return Err(RejectReason::EmptySlot);
                };
                let Some(def) = item_def(stack.id) else {
                    return Err(RejectReason::EmptySlot);
                };
                // A stack just out of the backpack carries nothing and does
                // nothing until it has woken up.
                if stack.mute > 0 {
                    return Err(RejectReason::NotReady);
                }
                let Some(aim) = def.aim else {
                    return Err(RejectReason::NotCastable);
                };
                if stack.cooldown > 0 || (def.shared_wait && self.owes_wait(unit, stack.id)) {
                    return Err(RejectReason::OnCooldown);
                }
                if (def.charges > 0 || def.cast_charges > 0) && stack.charges == 0 {
                    return Err(RejectReason::NoCharges);
                }
                let aimed = if def.mana_deficit {
                    matches!(target, Target::None)
                        || *target == Target::Unit(crate::game::wire_id(unit))
                } else {
                    aimed_right(aim, target)
                };
                if !aimed {
                    return Err(RejectReason::WrongTargetKind);
                }
                if let Target::Unit(target) = target
                    && self.of_wire(*target).is_none()
                {
                    return Err(RejectReason::UnknownTarget);
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
            Order::Cheat { cheat } => {
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
            _ => Ok(()),
        }
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
            .iter()
            .find(|entity| crate::game::wire_id(*entity) == id)
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
                    hero_damage: 0,
                    structure_damage: 0,
                })
                .collect(),
        }
    }
}
