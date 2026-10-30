//! What a body is doing right now.

use bota_proto::{AbilitySlot, ItemSlot, Target};

use crate::engine::{Fnv, StateHash, state_hash};
use crate::game::Entity;

/// What a body is doing. One thing at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action {
    pub state: ActionState,
    /// Beats until a swing may begin. Zero: it may.
    pub attack_cooldown: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionState {
    Ready,
    /// The hit lands on whoever the swing began against.
    Attack {
        target: Entity,
        phase: ActionPhase,
    },
    /// The target is the order, resolved at the moment of effect.
    CastAbility {
        target: Target,
        slot: AbilitySlot,
        phase: ActionPhase,
    },
    UseItem {
        target: Target,
        slot: ItemSlot,
        phase: ActionPhase,
    },
}

/// Where an action stands. `progress` is beats since the phase began.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionPhase {
    /// Windup or cast point. Cancelled at no cost.
    Before { progress: u32 },
    /// The ability or item under way. Zero length for an attack and for one
    /// that is over at once. Breaking it off calls its `on_cancel`.
    During { progress: u32 },
    /// Backswing. Cancelled at no cost.
    After { progress: u32 },
}

state_hash!(Action {
    state,
    attack_cooldown
});

impl StateHash for ActionState {
    fn state_hash(&self, fnv: &mut Fnv) {
        match self {
            ActionState::Ready => fnv.u8(0),
            ActionState::Attack { target, phase } => {
                fnv.u8(1);
                target.state_hash(fnv);
                phase.state_hash(fnv);
            }
            ActionState::CastAbility {
                target,
                slot,
                phase,
            } => {
                fnv.u8(2);
                target.state_hash(fnv);
                slot.state_hash(fnv);
                phase.state_hash(fnv);
            }
            ActionState::UseItem {
                target,
                slot,
                phase,
            } => {
                fnv.u8(3);
                target.state_hash(fnv);
                slot.state_hash(fnv);
                phase.state_hash(fnv);
            }
        }
    }
}

impl StateHash for ActionPhase {
    fn state_hash(&self, fnv: &mut Fnv) {
        let (tag, progress) = match self {
            ActionPhase::Before { progress } => (0, progress),
            ActionPhase::During { progress } => (1, progress),
            ActionPhase::After { progress } => (2, progress),
        };
        fnv.u8(tag);
        progress.state_hash(fnv);
    }
}

#[cfg(test)]
mod specimens {
    use super::*;
    use crate::engine::Specimen;

    impl Specimen for ActionState {
        fn low() -> Self {
            ActionState::Ready
        }

        fn high() -> Self {
            ActionState::CastAbility {
                target: Specimen::high(),
                slot: Specimen::high(),
                phase: Specimen::high(),
            }
        }
    }

    impl Specimen for ActionPhase {
        fn low() -> Self {
            ActionPhase::Before { progress: 1 }
        }

        fn high() -> Self {
            ActionPhase::After { progress: 2 }
        }
    }
}
