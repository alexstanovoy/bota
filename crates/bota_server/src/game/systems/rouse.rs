//! What a hero going for a hero does to everybody who is not the one doing it.
//!
//! An attack order does it, whether the attack ever happens or not, and only
//! a hero's order does it. Only an order at an enemy hero calls creeps and
//! towers on, and only an order at *another* of your own calls them off; an
//! order at any other enemy moves nobody, and one a hero aims at itself is
//! nobody's business but its own. Every swing a hero begins at a hero calls
//! the lane creeps that see it on as well, more weakly: it holds nobody.

use bota_core::{kind_def, rules};
use bota_proto::{Team, UnitKind, Vec2};

use crate::game::{Entity, World};

/// Which way an order moves the bystanders, if it moves them at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    /// At an enemy hero: the creeps come for whoever gave it.
    On,
    /// At one of your own: they look again with that one put last.
    Off,
}

impl World {
    /// Wakes whatever an attack order reaches onto, or off, the one who gave
    /// it.
    pub fn rouse_bystanders(&mut self, orderer: Entity, mark: Entity) {
        if orderer == mark {
            return;
        }
        let Some(side) = self.team.get(orderer).copied() else {
            return;
        };
        let Some(call) = self.call_of(side, mark) else {
            return;
        };
        self.rouse_with(orderer, call);
    }

    /// Puts a call out to everything that can hear it.
    ///
    /// A call on reaches the lane creeps only when the orderer's own
    /// [`rules::ORDER_AGGRO_COOLDOWN_TICKS`] has run out, and starts it again
    /// whether or not a creep was near to hear it. Letting go waits on
    /// nothing, and towers keep their own wait.
    fn rouse_with(&mut self, orderer: Entity, call: Call) {
        if self.kind.get(orderer) != Some(&UnitKind::Hero) {
            return;
        }
        let (Some(side), Some(at)) = (
            self.team.get(orderer).copied(),
            self.transform.get(orderer).map(|t| t.pos),
        ) else {
            return;
        };
        let creeps_hear = match call {
            Call::On => self.start_draw_cooldown(orderer),
            Call::Off => true,
        };
        for entity in self.entities.iter().collect::<Vec<_>>() {
            if self.team.get(entity).copied() == Some(side) || !self.can_attack(entity) {
                continue;
            }
            if self.lane_ai.get(entity).is_some() {
                if creeps_hear {
                    self.rouse_creep(entity, orderer, at, call);
                }
            } else if self.kind.get(entity) == Some(&UnitKind::Tower) {
                self.rouse_tower(entity, orderer, call);
            }
        }
    }

    /// Starts a hero's wait before its attack orders may draw lane creeps
    /// again, unless one is already running. Answers whether it was free to
    /// draw them.
    fn start_draw_cooldown(&mut self, hero: Entity) -> bool {
        let Some(orders) = self.orders.get_mut(hero) else {
            return false;
        };
        if orders.cooldown > 0 {
            return false;
        }
        orders.cooldown = rules::ORDER_AGGRO_COOLDOWN_TICKS;
        true
    }

    /// Draws the lane creeps around a hero that begins a swing at a hero
    /// onto the swinger.
    ///
    /// Every lane creep not of the swinger's side within
    /// [`rules::HERO_AGGRO_RANGE`] that sees it and may be called on yet
    /// takes it as its target, unless an attack order holds the creep or the
    /// swinger's own order wait still runs. Nothing is held and no wait
    /// starts: the creep goes back to its own ranking the tick after.
    pub fn rouse_by_swing(&mut self, swinger: Entity, mark: Entity) {
        if self.kind.get(swinger) != Some(&UnitKind::Hero)
            || self.kind.get(mark) != Some(&UnitKind::Hero)
            || self.orders.get(swinger).is_some_and(|o| o.cooldown > 0)
        {
            return;
        }
        let (Some(side), Some(at)) = (
            self.team.get(swinger).copied(),
            self.transform.get(swinger).map(|t| t.pos),
        ) else {
            return;
        };
        let reach = rules::units(rules::HERO_AGGRO_RANGE);
        for creep in self.entities.iter().collect::<Vec<_>>() {
            let Some(mut ai) = self.lane_ai.get(creep).copied() else {
                continue;
            };
            let Some(theirs) = self.team.get(creep).copied() else {
                continue;
            };
            let near = self
                .transform
                .get(creep)
                .is_some_and(|t| t.pos.within(at, reach));
            if theirs == side
                || !near
                || self.tick < ai.keep_until
                || !self.can_attack(creep)
                || !self.can_see(theirs, swinger)
                || !self.aggroable_yet(creep)
            {
                continue;
            }
            ai.roused_by = Some(swinger);
            ai.roused_at_own = false;
            self.lane_ai.insert(creep, ai);
        }
    }

    /// Which way an order at this mark calls, if it calls at all.
    fn call_of(&self, side: Team, mark: Entity) -> Option<Call> {
        let their_side = self.team.get(mark).copied()?;
        if their_side == side {
            return Some(Call::Off);
        }
        if self.kind.get(mark) == Some(&UnitKind::Hero) {
            return Some(Call::On);
        }
        None
    }

    /// One creep's answer: the orderer must stand within
    /// [`rules::HERO_AGGRO_RANGE`] of it, seen or not.
    fn rouse_creep(&mut self, creep: Entity, orderer: Entity, at: Vec2, call: Call) {
        let reach = rules::units(rules::HERO_AGGRO_RANGE);
        if !self
            .transform
            .get(creep)
            .is_some_and(|t| t.pos.within(at, reach))
        {
            return;
        }
        // Being called on is what the early rule holds back; letting go is
        // never held back.
        if call == Call::On && !self.aggroable_yet(creep) {
            return;
        }
        self.provoke(creep, orderer, call == Call::Off);
    }

    /// One tower's answer, when the orderer is in its reach.
    ///
    /// Called on, it takes the orderer as its target unless an earlier call
    /// is still cooling down, and starts that wait again. Called off, it
    /// drops the orderer if that is its target, at most once every
    /// [`rules::TOWER_RELEASE_COOLDOWN_TICKS`].
    fn rouse_tower(&mut self, tower: Entity, orderer: Entity, call: Call) {
        if !self.in_reach(tower, orderer) {
            return;
        }
        let Some(orders) = self.orders.get(tower).copied() else {
            return;
        };
        match call {
            Call::On => {
                if orders.cooldown > 0 {
                    return;
                }
                self.set_target(tower, orderer);
                if let Some(orders) = self.orders.get_mut(tower) {
                    orders.cooldown = rules::ORDER_AGGRO_COOLDOWN_TICKS;
                }
            }
            Call::Off => {
                if orders.release_cooldown > 0 || self.target_of(tower) != Some(orderer) {
                    return;
                }
                self.target.remove(tower);
                if let Some(orders) = self.orders.get_mut(tower) {
                    orders.release_cooldown = rules::TOWER_RELEASE_COOLDOWN_TICKS;
                }
            }
        }
    }

    /// Whether a lane creep may be called on by an attack order yet.
    ///
    /// Free from [`rules::FREE_AGGRO_TICK`]. Before it, only a creep that
    /// already has an enemy lane creep or a neutral within its acquisition, or
    /// that stands near one of its own tier-one towers.
    fn aggroable_yet(&self, creep: Entity) -> bool {
        if self.tick >= rules::FREE_AGGRO_TICK {
            return true;
        }
        let (Some(side), Some(at), Some(reach)) = (
            self.team.get(creep).copied(),
            self.transform.get(creep).map(|t| t.pos),
            self.stats.get(creep).map(|stats| stats.acquisition),
        ) else {
            return false;
        };
        let busy = self.entities.iter().any(|other| {
            let Some(their_side) = self.team.get(other).copied() else {
                return false;
            };
            let creepish = self
                .kind
                .get(other)
                .copied()
                .is_some_and(|kind| kind_def(kind).lane_creep)
                && their_side != side;
            (creepish || their_side == Team::Neutral)
                && self.alive(other)
                && self
                    .transform
                    .get(other)
                    .is_some_and(|t| at.within(t.pos, reach))
        });
        if busy {
            return true;
        }
        let near_home = rules::units(rules::EARLY_AGGRO_TOWER_RANGE);
        self.entities.iter().any(|other| {
            self.kind.get(other) == Some(&UnitKind::Tower)
                && self.team.get(other).copied() == Some(side)
                && self.tier.get(other).is_some_and(|tier| tier.0 == 1)
                && self
                    .transform
                    .get(other)
                    .is_some_and(|t| at.within(t.pos, near_home))
        })
    }
}
