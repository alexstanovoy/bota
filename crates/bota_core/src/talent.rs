//! The talents a hero may learn, tier by tier, and the hero levels that open
//! them.

use bota_proto::{TalentSlot, Talents};

use crate::rules;

/// One talent: what it changes. The numbers live in [`rules`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Talent {
    /// [`rules::TALENT_FEAST_ATTACK_SPEED`] more attack speed from a feast.
    FeastAttackSpeed,
    /// [`rules::TALENT_RAZE_STACK_DAMAGE`] more raze damage a stack.
    RazeStackDamage,
    /// [`rules::TALENT_FEAST_SOULS_PER_HERO`] more souls a feast takes from a
    /// hero.
    FeastSoulsPerHero,
    /// [`rules::TALENT_PRESENCE_ARMOR`] more armor taken by the presence.
    PresenceArmor,
    /// [`rules::TALENT_REQUIEM_LINE_TICKS`] more fear a requiem line, up to
    /// [`rules::TALENT_REQUIEM_HOLD_TICKS`] more in all.
    RequiemFear,
    /// [`rules::TALENT_NECRO_SOUL_CAP`] more souls held from kills.
    NecroSoulCap,
    /// A raze also deals the caster's attack damage, as physical damage.
    RazeAttackDamage,
    /// Cast points [`rules::TALENT_FEAST_CAST_POINT_PCT`] of their length
    /// while a feast runs.
    FeastCastSpeed,
}

/// A hero's whole tree, indexed by [`TalentSlot`].
pub type TalentTree = [Talent; TalentSlot::COUNT as usize];

/// Shadow Fiend's tree at Dota 7.41f, the left of each tier first.
pub const SHADOW_FIEND_TALENTS: TalentTree = [
    Talent::FeastAttackSpeed,
    Talent::RazeStackDamage,
    Talent::FeastSoulsPerHero,
    Talent::PresenceArmor,
    Talent::RequiemFear,
    Talent::NecroSoulCap,
    Talent::RazeAttackDamage,
    Talent::FeastCastSpeed,
];

/// The hero level a talent opens at, given what is learned already: its
/// tier's level, or, once the other talent of its tier is taken,
/// [`rules::TALENT_SECOND_LEVELS`].
pub fn talent_floor(slot: TalentSlot, learned: Talents) -> u8 {
    assert!(slot.0 < TalentSlot::COUNT);
    let tier = usize::from(slot.tier());
    if learned.has(slot.other()) {
        rules::TALENT_SECOND_LEVELS[tier]
    } else {
        rules::TALENT_LEVELS[tier]
    }
}
