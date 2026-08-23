//! Couriers: standing them up, sending them on errands, bringing them back.

use bota_proto::{Team, Vec2};

use crate::engine::Entity;
use crate::game::{COURIER, Errand, Inventory, Status, StatusKind, UnitOrder, World, rules};

impl World {
    /// Stands a courier up for one seat at its own fountain.
    pub fn stand_up_courier(&mut self, seat: usize) {
        let (Some(side), at) = (
            self.seats.get(seat).map(|seat| seat.team),
            crate::game::fountain_pos(self.map, self.seats[seat].team),
        ) else {
            return;
        };
        let courier = self.spawn_unit(&COURIER, side, at);
        // What the last courier carried when it fell comes back aboard.
        let load = self.seats[seat]
            .courier_kept
            .take()
            .unwrap_or_else(|| Inventory::empty(rules::INVENTORY_SLOTS));
        self.inventory.insert(courier, load);
        self.owner.insert(courier, self.seats[seat].slot);
        self.abilities.insert(
            courier,
            crate::game::AbilityBook {
                slots: [
                    crate::game::ability::TAKE_STASH,
                    crate::game::ability::RETURN_ITEMS,
                    crate::game::ability::BURST,
                    crate::game::ability::DELIVER,
                    crate::game::ability::SHIELD,
                ]
                .into_iter()
                .map(|id| crate::game::AbilityState {
                    id,
                    level: 1,
                    cooldown: 0,
                })
                .collect(),
            },
        );
        self.errand.insert(courier, Errand::None);
        self.seats[seat].courier = Some(courier);
        self.seats[seat].courier_left = 0;
    }

    /// Brings back every courier whose wait is out, and starts the wait for
    /// every one that has just gone.
    pub fn tick_couriers(&mut self) {
        self.run_errands();
        for seat in 0..self.seats.len() {
            match self.seats[seat].courier {
                Some(courier) if !self.alive(courier) => {
                    self.seats[seat].courier = None;
                    self.seats[seat].courier_left = rules::COURIER_RESPAWN_TICKS;
                }
                Some(_) => continue,
                None => {}
            }
            if self.seats[seat].courier_left > 0 {
                self.seats[seat].courier_left -= 1;
                if self.seats[seat].courier_left == 0 {
                    self.stand_up_courier(seat);
                }
            }
        }
    }

    /// The seat a courier belongs to, if it belongs to one.
    fn seat_of_courier(&self, courier: Entity) -> Option<usize> {
        self.seats
            .iter()
            .position(|seat| seat.courier == Some(courier))
    }

    /// Makes a courier fly faster for a while.
    pub fn courier_burst(&mut self, courier: Entity) -> bool {
        if self.statuses.get(courier).is_some_and(|on_it| {
            on_it
                .active()
                .any(|status| matches!(status.kind, StatusKind::Hastened { .. }))
        }) {
            return false;
        }
        let mut on_it = self.statuses.remove(courier).unwrap_or_default();
        on_it.put(Status {
            kind: StatusKind::Hastened {
                pct: rules::COURIER_BURST_PCT,
            },
            ticks_left: rules::COURIER_BURST_TICKS,
        });
        self.statuses.insert(courier, on_it);
        true
    }

    /// Sends a courier home to its own fountain.
    pub fn courier_go_home(&mut self, courier: Entity) -> bool {
        if self.seat_of_courier(courier).is_none() {
            return false;
        }
        self.errand.insert(courier, Errand::GoingHome);
        true
    }

    /// Sends it for what waits in its owner's stash.
    pub fn courier_take_stash(&mut self, courier: Entity) -> bool {
        if self.seat_of_courier(courier).is_none() {
            return false;
        }
        self.errand.insert(courier, Errand::ToStash);
        true
    }

    /// Sends it to put back what it holds.
    pub fn courier_return_items(&mut self, courier: Entity) -> bool {
        if self.seat_of_courier(courier).is_none() {
            return false;
        }
        self.errand.insert(courier, Errand::PutBack);
        true
    }

    /// Puts a shield on it that nothing gets through.
    pub fn courier_shield(&mut self, courier: Entity) -> bool {
        if self.seat_of_courier(courier).is_none() {
            return false;
        }
        let mut on_it = self.statuses.remove(courier).unwrap_or_default();
        on_it.put(Status {
            kind: StatusKind::Shielded,
            ticks_left: rules::COURIER_SHIELD_TICKS,
        });
        self.statuses.insert(courier, on_it);
        true
    }

    /// Sends it to its owner with what it holds.
    pub fn courier_deliver(&mut self, courier: Entity) -> bool {
        if self.seat_of_courier(courier).is_none() {
            return false;
        }
        self.errand.insert(courier, Errand::ToOwner);
        true
    }

    /// Carries every errand one tick on.
    fn run_errands(&mut self) {
        for courier in self.entities.iter().collect::<Vec<_>>() {
            let Some(errand) = self.errand.get(courier).copied() else {
                continue;
            };
            let Some(seat) = self.seat_of_courier(courier) else {
                continue;
            };
            if !self.alive(courier) {
                continue;
            }
            let done = match errand {
                Errand::None => continue,
                Errand::ToStash => self.take_the_stash(seat, courier),
                Errand::PutBack => self.put_the_stash_back(seat, courier),
                Errand::ToOwner => self.deliver(seat, courier),
                Errand::GoingHome => self.go_home(seat, courier),
            };
            if done {
                self.errand.insert(courier, Errand::None);
            }
        }
    }

    /// Takes what waits in a seat stash, from the spot by the shop.
    ///
    /// Nothing to take is not a failure. Carrying something already, it takes
    /// that on to its owner rather than fetching nothing; carrying nothing
    /// either, it goes home and stays. What is taken is carried on at once,
    /// without being asked twice.
    fn take_the_stash(&mut self, seat: usize, courier: Entity) -> bool {
        if self.seats[seat].stash.held().count() == 0 {
            let carrying = self
                .inventory
                .get(courier)
                .is_some_and(|bag| bag.held().count() > 0);
            self.errand.insert(
                courier,
                if carrying {
                    Errand::ToOwner
                } else {
                    Errand::GoingHome
                },
            );
            return false;
        }
        if !self.at_the_stash(seat, courier) {
            return false;
        }
        let mut moved = false;
        for at in 0..self.seats[seat].stash.slots.len() {
            let Some(stack) = self.seats[seat].stash.slots[at] else {
                continue;
            };
            let Some(bag) = self.inventory.get_mut(courier) else {
                break;
            };
            let Some(free) = bag.slots.iter_mut().find(|slot| slot.is_none()) else {
                break;
            };
            *free = Some(stack);
            self.seats[seat].stash.slots[at] = None;
            moved = true;
        }
        if moved {
            // Having taken it, it is carrying it: that is one errand, not two.
            self.errand.insert(courier, Errand::ToOwner);
        }
        false
    }

    /// Puts back what a courier holds, from the spot by the shop.
    ///
    /// Holding nothing, it goes home instead.
    fn put_the_stash_back(&mut self, seat: usize, courier: Entity) -> bool {
        if self
            .inventory
            .get(courier)
            .is_none_or(|bag| bag.held().count() == 0)
        {
            self.errand.insert(courier, Errand::GoingHome);
            return false;
        }
        if !self.at_the_stash(seat, courier) {
            return false;
        }
        for at in 0..rules::INVENTORY_SLOTS {
            let Some(stack) = self
                .inventory
                .get(courier)
                .and_then(|bag| bag.slots.get(at).copied().flatten())
            else {
                continue;
            };
            let Some(free) = self.seats[seat]
                .stash
                .slots
                .iter_mut()
                .find(|slot| slot.is_none())
            else {
                break;
            };
            *free = Some(stack);
            if let Some(bag) = self.inventory.get_mut(courier)
                && let Some(held) = bag.slots.get_mut(at)
            {
                *held = None;
            }
        }
        self.errand.insert(courier, Errand::GoingHome);
        false
    }

    /// Whether a courier stands where the stash can be reached, walking to
    /// that spot if it does not.
    fn at_the_stash(&mut self, seat: usize, courier: Entity) -> bool {
        let shop = crate::game::fountain_pos(self.map, self.seats[seat].team);
        if self.transform.get(courier).is_some_and(|at| {
            at.pos
                .within(shop, rules::units(rules::COURIER_STASH_RANGE))
        }) {
            return true;
        }
        self.set_order(courier, UnitOrder::Move { pos: shop });
        false
    }

    /// Carries what it holds to its owner, hands it over, and takes aboard
    /// whatever the owner has marked for sale.
    ///
    /// Holding nothing and with nothing marked, it goes home. With its owner
    /// fallen, it turns round and puts what it holds back in the stash.
    fn deliver(&mut self, seat: usize, courier: Entity) -> bool {
        let empty = self
            .inventory
            .get(courier)
            .is_none_or(|bag| bag.held().count() == 0);
        let Some(owner) = self.seats[seat].unit.filter(|hero| self.alive(*hero)) else {
            self.errand.insert(
                courier,
                if empty {
                    Errand::GoingHome
                } else {
                    Errand::PutBack
                },
            );
            return false;
        };
        // A courier called with an empty bag still flies out while something
        // is marked: the call is the ask.
        if empty && !self.holds_marked(owner) {
            self.errand.insert(courier, Errand::GoingHome);
            return false;
        }
        let Some(to) = self.transform.get(owner).map(|at| at.pos) else {
            return false;
        };
        let near = self.transform.get(courier).is_some_and(|at| {
            at.pos
                .within(to, rules::units(rules::COURIER_DELIVER_RANGE))
        });
        if !near {
            self.set_order(courier, UnitOrder::Move { pos: to });
            return false;
        }
        self.hand_over(courier, owner);
        self.collect_marked(courier, owner);
        // What its owner had no room for is carried back to the stash rather
        // than flown home and sat on, and what was marked rides the same leg.
        let left = self
            .inventory
            .get(courier)
            .is_some_and(|bag| bag.held().count() > 0);
        self.errand.insert(
            courier,
            if left {
                Errand::PutBack
            } else {
                Errand::GoingHome
            },
        );
        false
    }

    /// Whether a unit's bag holds anything marked for sale.
    fn holds_marked(&self, unit: Entity) -> bool {
        self.inventory
            .get(unit)
            .is_some_and(|bag| bag.held().any(|stack| stack.for_sale))
    }

    /// Takes every stack a unit has marked for sale into a courier's free
    /// slots.
    fn collect_marked(&mut self, courier: Entity, owner: Entity) {
        let slots = self.inventory.get(owner).map_or(0, |bag| bag.slots.len());
        for at in 0..slots {
            let Some(stack) = self
                .inventory
                .get(owner)
                .and_then(|bag| bag.slots.get(at).copied().flatten())
            else {
                continue;
            };
            if !stack.for_sale {
                continue;
            }
            let Some(bag) = self.inventory.get_mut(courier) else {
                return;
            };
            let Some(free) = bag.slots.iter_mut().find(|held| held.is_none()) else {
                return;
            };
            *free = Some(stack);
            if let Some(bag) = self.inventory.get_mut(owner)
                && let Some(held) = bag.slots.get_mut(at)
            {
                *held = None;
            }
        }
    }

    /// Walks home and stands there.
    fn go_home(&mut self, seat: usize, courier: Entity) -> bool {
        let home = crate::game::fountain_pos(self.map, self.seats[seat].team);
        if self.transform.get(courier).is_some_and(|at| at.pos == home) {
            return true;
        }
        self.set_order(courier, UnitOrder::Move { pos: home });
        false
    }

    /// Moves what a courier holds into whatever room its owner has.
    fn hand_over(&mut self, courier: Entity, owner: Entity) {
        for at in 0..rules::INVENTORY_SLOTS {
            let Some(stack) = self
                .inventory
                .get(courier)
                .and_then(|bag| bag.slots.get(at).copied().flatten())
            else {
                continue;
            };
            let Some(bag) = self.inventory.get_mut(owner) else {
                return;
            };
            let Some(free) = bag.slots.iter_mut().find(|slot| slot.is_none()) else {
                return;
            };
            *free = Some(stack);
            if let Some(bag) = self.inventory.get_mut(courier)
                && let Some(held) = bag.slots.get_mut(at)
            {
                *held = None;
            }
        }
    }

    /// Where a side's couriers stand up, for anything that needs to know.
    pub fn courier_home(&self, team: Team) -> Vec2 {
        crate::game::fountain_pos(self.map, team)
    }
}
