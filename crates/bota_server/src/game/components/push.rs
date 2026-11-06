//! Forced movement owned by the victim, including a channel's grip and an independent flight.
use crate::engine::{Fnv, StateHash, state_hash};
use crate::game::Entity;
use bota_proto::{DamageKind, EffectId, Fixed, Inflictor, Team, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Push {
    pub source: Entity,
    pub inflictor: Inflictor,
    pub team: Team,
    pub aim: Vec2,
    pub speed: Fixed,
    pub clear: Vec2,
    pub effect: Option<EffectId>,
    pub phase: PushPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushPhase {
    Ground,
    Grip {
        started: u32,
        level: u8,
        contacted: bool,
        combo: u8,
    },
    Arc {
        from: Vec2,
        started: u32,
        duration: u32,
        height: i32,
        tree_radius: i32,
        landing: Option<Landing>,
    },
}

/// Values captured at launch. Health-based growth reads the game clock on landing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landing {
    pub damage: i32,
    pub health_growth: Option<bota_core::DamageGrowth>,
    pub level: u8,
    pub kind: DamageKind,
    pub stun: u32,
    pub damage_amp_bp: i32,
}

impl Push {
    pub fn carries(self) -> bool {
        !matches!(self.phase, PushPhase::Ground)
    }
    pub fn height(self, tick: u32) -> Fixed {
        let PushPhase::Arc {
            started,
            duration,
            height,
            ..
        } = self.phase
        else {
            return Fixed::ZERO;
        };
        let span = i128::from(duration.max(1));
        let age = i128::from(tick.saturating_sub(started).min(duration));
        let raw = 4 * i128::from(height.max(0)) * i128::from(Fixed::ONE.raw) * age * (span - age)
            / (span * span);
        Fixed {
            raw: raw.clamp(0, i32::MAX.into()) as i32,
        }
    }
}
state_hash!(Push {
    source,
    inflictor,
    team,
    aim,
    speed,
    clear,
    effect,
    phase
});
state_hash!(Landing {
    damage,
    health_growth,
    level,
    kind,
    stun,
    damage_amp_bp
});
impl StateHash for bota_core::DamageGrowth {
    fn state_hash(&self, h: &mut Fnv) {
        self.clock.first_after_horn.state_hash(h);
        self.clock.period.state_hash(h);
        self.clock.cap.state_hash(h);
        self.per_step.state_hash(h);
    }
}
impl StateHash for PushPhase {
    fn state_hash(&self, h: &mut Fnv) {
        match self {
            Self::Ground => h.u8(0),
            Self::Grip {
                started,
                level,
                contacted,
                combo,
            } => {
                h.u8(1);
                started.state_hash(h);
                level.state_hash(h);
                contacted.state_hash(h);
                combo.state_hash(h);
            }
            Self::Arc {
                from,
                started,
                duration,
                height,
                tree_radius,
                landing,
            } => {
                h.u8(2);
                from.state_hash(h);
                started.state_hash(h);
                duration.state_hash(h);
                height.state_hash(h);
                tree_radius.state_hash(h);
                landing.state_hash(h);
            }
        }
    }
}

state_hash!(EffectId(0));
#[cfg(test)]
impl crate::engine::Specimen for bota_core::DamageGrowth {
    fn low() -> Self {
        Self {
            clock: bota_core::GrowthClock {
                first_after_horn: 0,
                period: 1,
                cap: 1,
            },
            per_step: [0; 4],
        }
    }
    fn high() -> Self {
        Self {
            clock: bota_core::GrowthClock {
                first_after_horn: 1,
                period: 2,
                cap: 2,
            },
            per_step: [1; 4],
        }
    }
}
#[cfg(test)]
impl crate::engine::Specimen for PushPhase {
    fn low() -> Self {
        Self::Ground
    }
    fn high() -> Self {
        Self::Arc {
            from: crate::engine::Specimen::high(),
            started: 1,
            duration: 2,
            height: 1,
            tree_radius: 1,
            landing: crate::engine::Specimen::high(),
        }
    }
}
