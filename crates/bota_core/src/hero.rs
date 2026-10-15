//! Every hero that may be picked, and what each one carries.

use bota_proto::{AbilityId, Attributes, Fixed, HeroId};

use crate::{PUDGE, SHADOW_FIEND, SHADOW_FIEND_TALENTS, TalentTree, UnitDef, rules};

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
            AbilityId::MEAT_HOOK,
            AbilityId::ROT,
            AbilityId::FLESH_HEAP,
            AbilityId::DISMEMBER,
        ],
        innate: None,
        talents: None,
    },
    HeroDef {
        name: "Shadow Fiend",
        unit: &SHADOW_FIEND,
        abilities: &[
            AbilityId::RAZE_NEAR,
            AbilityId::RAZE_MID,
            AbilityId::RAZE_FAR,
            AbilityId::FEAST_OF_SOULS,
            AbilityId::PRESENCE,
            AbilityId::REQUIEM,
        ],
        innate: Some(AbilityId::NECROMASTERY),
        talents: Some(&SHADOW_FIEND_TALENTS),
    },
];

/// What one hero is, or nothing if no such hero exists.
pub fn hero_def(id: HeroId) -> Option<&'static HeroDef> {
    HEROES.get(usize::from(id.0))
}

/// A hero's attributes at a level, before items, souls, buffs and talents.
pub fn attributes_at_level(unit: &UnitDef, level: u8) -> Attributes {
    let levels = i32::from(level.max(1)) - 1;
    unit.attributes + unit.per_level.attributes.scaled(Fixed::from_int(levels))
}

/// The least damage a hero's swing deals at a level before items and
/// buffs: its base, what the levels add and what its primary attribute
/// pays.
pub fn base_damage_least(unit: &UnitDef, level: u8) -> i32 {
    let levels = i32::from(level.max(1)) - 1;
    let grown = unit.damage + unit.per_level.damage * levels;
    match unit.primary {
        Some(primary) => {
            let pays = attributes_at_level(unit, level).of(primary)
                * Fixed::from_int(rules::DAMAGE_PER_PRIMARY);
            grown + pays.to_int()
        }
        None => grown,
    }
}
