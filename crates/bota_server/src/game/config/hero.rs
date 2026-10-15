//! Every hero that may be picked, and what each one carries.

use bota_core::{PUDGE, SHADOW_FIEND, SHADOW_FIEND_TALENTS, TalentTree, UnitDef};
use bota_proto::{AbilityId, HeroId};

use crate::game::ability;

/// One hero: what it is made of and what it can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeroDef {
    /// What it is called.
    pub name: &'static str,
    /// The plain form of its body.
    pub unit: &'static UnitDef,
    /// The slots it carries, in the order they are shown. The ultimate sits
    /// last.
    pub abilities: &'static [AbilityId],
    /// The innate ability it carries from its first level in no slot, never
    /// learned. Absent for a hero with none.
    pub innate: Option<AbilityId>,
    /// The talents it may learn. Absent for a hero with none: only Shadow
    /// Fiend, the one hero at Dota's numbers, has a tree.
    pub talents: Option<&'static TalentTree>,
}

/// Every hero, indexed by [`HeroId`].
pub const HEROES: [HeroDef; 2] = [
    HeroDef {
        name: "Pudge",
        unit: &PUDGE,
        abilities: &[
            ability::MEAT_HOOK,
            ability::ROT,
            ability::FLESH_HEAP,
            ability::DISMEMBER,
        ],
        innate: None,
        talents: None,
    },
    HeroDef {
        name: "Shadow Fiend",
        unit: &SHADOW_FIEND,
        abilities: &[
            ability::RAZE_NEAR,
            ability::RAZE_MID,
            ability::RAZE_FAR,
            ability::FEAST_OF_SOULS,
            ability::PRESENCE,
            ability::REQUIEM,
        ],
        innate: Some(ability::NECROMASTERY),
        talents: Some(&SHADOW_FIEND_TALENTS),
    },
];

/// What one hero is, or nothing if no such hero exists.
pub fn hero_def(id: HeroId) -> Option<&'static HeroDef> {
    HEROES.get(usize::from(id.0))
}
