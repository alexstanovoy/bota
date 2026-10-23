//! Every hero that may be picked, and what each one carries.

use bota_proto::{AbilityId, Attribute, Attributes, Fixed, HeroId};

use crate::heroes::{pudge, shadow_fiend};
use crate::{TalentTree, UnitDef, rules};

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
pub const HEROES: [HeroDef; 2] = [pudge::HERO_DEF, shadow_fiend::HERO_DEF];

/// What one hero is, or nothing if no such hero exists.
#[inline]
pub fn hero_def(id: HeroId) -> Option<&'static HeroDef> {
    HEROES.get(usize::from(id.0))
}

/// A hero's attributes at a level, before items, souls, buffs and talents.
#[inline]
pub fn attributes_at_level(unit: &UnitDef, level: u8) -> Attributes {
    let levels = i32::from(level.max(1)) - 1;
    unit.attributes + unit.per_level.attributes.scaled(Fixed::from_int(levels))
}

/// The least damage a hero's swing deals at a level before items and
/// buffs: its base, what the levels add and what its primary attribute
/// pays.
pub fn base_damage_least(unit: &UnitDef, level: u8) -> i32 {
    white_damage_least(unit, level, attributes_at_level(unit, level))
}

/// The least of the damage Dota draws white: the hero's base, what the
/// levels add, and what its primary attribute pays for every point it
/// holds, `has`, from items and buffs too. Only flat bonus damage is green.
pub fn white_damage_least(unit: &UnitDef, level: u8, has: Attributes) -> i32 {
    let levels = i32::from(level.max(1)) - 1;
    unit.damage + unit.per_level.damage * levels + primary_damage(has, unit.primary)
}

/// The attack damage a primary attribute pays: none for a unit without one.
pub fn primary_damage(has: Attributes, primary: Option<Attribute>) -> i32 {
    primary.map_or(0, |primary| {
        (has.of(primary) * Fixed::from_int(rules::DAMAGE_PER_PRIMARY)).to_int()
    })
}

/// The armor agility is worth.
pub fn agility_armor(has: Attributes) -> Fixed {
    rules::ARMOR_PER_AGILITY * has.agility
}

/// The attack speed agility is worth.
pub fn agility_attack_speed(has: Attributes) -> i32 {
    (has.agility * Fixed::from_int(rules::ATTACK_SPEED_PER_AGILITY)).to_int()
}
