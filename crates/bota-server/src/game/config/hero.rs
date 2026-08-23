//! Every hero that may be picked, and what each one carries.

use bota_proto::{AbilityId, HeroId};

use crate::game::{HERO, PUDGE, SHADOW_FIEND, UnitDef, ability};

/// One hero: what it is made of and what it can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeroDef {
    /// What it is called.
    pub name: &'static str,
    /// The plain form of its body.
    pub unit: &'static UnitDef,
    /// The four slots it carries, in the order they are shown.
    pub abilities: [AbilityId; 4],
    /// Whether it keeps a soul of everything it brings down. What souls are
    /// worth is the stats system's business.
    pub souls: bool,
}

/// Every hero, indexed by [`HeroId`].
pub const HEROES: [HeroDef; 3] = [
    HeroDef {
        name: "Sylla",
        unit: &HERO,
        abilities: [
            ability::CRIT,
            ability::FRENZY,
            ability::BOUNCE,
            ability::VOLLEY,
        ],
        souls: false,
    },
    HeroDef {
        name: "Pudge",
        unit: &PUDGE,
        abilities: [
            ability::MEAT_HOOK,
            ability::ROT,
            ability::FLESH_HEAP,
            ability::DISMEMBER,
        ],
        souls: false,
    },
    HeroDef {
        name: "Shadow Fiend",
        unit: &SHADOW_FIEND,
        abilities: [
            ability::RAZE_NEAR,
            ability::RAZE_MID,
            ability::RAZE_FAR,
            ability::REQUIEM,
        ],
        souls: true,
    },
];

/// What one hero is, or nothing if no such hero exists.
pub fn hero_def(id: HeroId) -> Option<&'static HeroDef> {
    HEROES.get(usize::from(id.0))
}
