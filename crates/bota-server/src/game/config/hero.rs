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
    /// The slots it carries, in the order they are shown. The ultimate sits
    /// last.
    pub abilities: &'static [AbilityId],
}

/// Every hero, indexed by [`HeroId`].
pub const HEROES: [HeroDef; 3] = [
    HeroDef {
        name: "Sylla",
        unit: &HERO,
        abilities: &[
            ability::CRIT,
            ability::FRENZY,
            ability::BOUNCE,
            ability::VOLLEY,
        ],
    },
    HeroDef {
        name: "Pudge",
        unit: &PUDGE,
        abilities: &[
            ability::MEAT_HOOK,
            ability::ROT,
            ability::FLESH_HEAP,
            ability::DISMEMBER,
        ],
    },
    HeroDef {
        name: "Shadow Fiend",
        unit: &SHADOW_FIEND,
        abilities: &[
            ability::RAZE_NEAR,
            ability::RAZE_MID,
            ability::RAZE_FAR,
            ability::NECROMASTERY,
            ability::PRESENCE,
            ability::REQUIEM,
        ],
    },
];

/// What one hero is, or nothing if no such hero exists.
pub fn hero_def(id: HeroId) -> Option<&'static HeroDef> {
    HEROES.get(usize::from(id.0))
}
