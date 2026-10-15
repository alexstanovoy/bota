//! What an entity can cast.

use bota_core::{Talent, TalentTree};
use bota_proto::{AbilityId, Talents};

/// One ability in a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbilityState {
    /// Which ability sits here.
    pub id: AbilityId,
    /// Level it stands at. Zero until it is learned.
    pub level: u8,
    /// Ticks until it may be cast again.
    pub cooldown: u32,
}

/// The abilities an entity holds, in slot order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AbilityBook {
    /// Every slot, in the order they are shown.
    pub slots: Vec<AbilityState>,
    /// The talents it may learn. Absent for one with none.
    pub tree: Option<&'static TalentTree>,
    /// The talents it has learned, by slot of its tree.
    pub talents: Talents,
}

impl AbilityBook {
    /// What sits in a slot, if anything does.
    pub fn slot(&self, index: usize) -> Option<&AbilityState> {
        self.slots.get(index)
    }

    /// Whether it has learned a talent.
    pub fn has_talent(&self, talent: Talent) -> bool {
        self.tree.is_some_and(|tree| {
            tree.iter().enumerate().any(|(at, held)| {
                *held == talent && self.talents.has(bota_proto::TalentSlot(at as u8))
            })
        })
    }

    /// Every ability that has been learned.
    pub fn learned(&self) -> impl Iterator<Item = &AbilityState> {
        self.slots.iter().filter(|a| a.level > 0)
    }
}
