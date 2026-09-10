//! What an attack order does to everybody who is not the one giving it.
//!
//! The order alone does it, whether the attack ever happens or not. Only an
//! order at an enemy hero calls creeps on, and only an order at *another* of
//! your own calls them off; an order at an enemy creep is a last hit and moves
//! nobody, and one a hero aims at itself is nobody's business but its own.

use bota_proto::{Team, UnitKind, Vec2};

use crate::game::rules;
use crate::game::{Entity, World, is_lane_creep};

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
        // Letting go is done by pointing at somebody else of your own. A hero
        // pointing at itself has told the creeps nothing.
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

    /// Wakes the creeps onto whoever aimed a spell at an enemy hero.
    ///
    /// A spell answers the same as a swing, less the letting go: at an enemy
    /// hero it calls them on, at an enemy creep it is a spell like any other
    /// and moves nobody, and at one of your own it is not their business at
    /// all.
    pub fn rouse_by_cast(&mut self, orderer: Entity, mark: Entity) {
        let Some(side) = self.team.get(orderer).copied() else {
            return;
        };
        if self.call_of(side, mark) != Some(Call::On) {
            return;
        }
        self.rouse_with(orderer, Call::On);
    }

    /// Puts a call out to everything that can hear it.
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
        for entity in self.entities.iter().collect::<Vec<_>>() {
            if self.team.get(entity).copied() == Some(side) || self.attacking.get(entity).is_none()
            {
                continue;
            }
            if self.lane_ai.get(entity).is_some() {
                self.rouse_creep(entity, orderer, at, call);
            } else if self.kind.get(entity) == Some(&UnitKind::Tower) {
                self.rouse_tower(entity, orderer, call);
            }
        }
    }

    /// Which way an order at this mark calls, if it calls at all.
    ///
    /// The mark is never the one who gave the order; [`rouse_bystanders`]
    /// turns that away before it gets here.
    ///
    /// [`rouse_bystanders`]: World::rouse_bystanders
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

    /// One creep's answer: it must be near enough to have seen the order given.
    fn rouse_creep(&mut self, creep: Entity, orderer: Entity, at: Vec2, call: Call) {
        let reach = self
            .stats
            .get(creep)
            .map_or(bota_proto::Fixed::ZERO, |stats| stats.acquisition);
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

    /// One tower's answer: it does not weigh the offender against anything.
    ///
    /// A dive draws it outright; a click at one of your own lets it go, and
    /// letting go answers at once however recently it was drawn.
    fn rouse_tower(&mut self, tower: Entity, orderer: Entity, call: Call) {
        if !self.in_reach(tower, orderer) {
            return;
        }
        match call {
            Call::On => {
                if self
                    .orders
                    .get(tower)
                    .is_some_and(|orders| orders.cooldown > 0)
                {
                    return;
                }
                self.set_target(tower, orderer);
            }
            Call::Off => {
                if self.target_of(tower) != Some(orderer) {
                    return;
                }
                self.target.remove(tower);
            }
        }
        if let Some(orders) = self.orders.get_mut(tower) {
            orders.cooldown = rules::ORDER_AGGRO_COOLDOWN_TICKS;
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
            let creepish =
                self.kind.get(other).copied().is_some_and(is_lane_creep) && their_side != side;
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
