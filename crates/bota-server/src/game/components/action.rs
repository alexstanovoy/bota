//! What a body is doing right now.

use bota_proto::{AbilitySlot, ItemSlot, Target};

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
    /// The ability under way. Zero length for an attack and for one that is
    /// over at once. Breaking it off calls its `on_cancel`.
    During { progress: u32 },
    /// Backswing. Cancelled at no cost.
    After { progress: u32 },
}
