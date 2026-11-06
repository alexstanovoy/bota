//! The talents a hero may learn, tier by tier, and the hero levels that open
//! them; and the Attribute Bonus, which Dota keeps under the talent tree.

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
    /// [`TALENT_ARMOR`] more armor.
    Armor,
    /// [`TALENT_SPELL_LIFESTEAL_PCT`] more spell lifesteal.
    SpellLifesteal,
    /// The rot slows more. This and the Pudge talents below give what their bonuses in his
    /// imported values say, by the talent's key
    /// ([`AbilityValue::talented`](crate::AbilityValue::talented)).
    RotSlow,
    /// The hook deals more.
    HookDamage,
    /// The hook waits less.
    HookCooldown,
    /// A dismember holds longer and bites more often.
    DismemberDuration,
    /// The flesh heap's stacks and the meat shield's block are worth more.
    FleshHeapBonuses,
    /// A dismember bites harder.
    DismemberDamage,
}

impl Talent {
    /// Dota's internal name of the talent, the ability its tooltip and icon
    /// are found by.
    pub const fn key(self) -> &'static str {
        match self {
            Talent::FeastAttackSpeed => "special_bonus_unique_nevermore_4",
            Talent::RazeStackDamage => "special_bonus_unique_nevermore_7",
            Talent::FeastSoulsPerHero => {
                "special_bonus_unique_nevermore_frenzy_max_collection_count"
            }
            Talent::PresenceArmor => "special_bonus_unique_nevermore_3",
            Talent::RequiemFear => "special_bonus_unique_nevermore_6",
            Talent::NecroSoulCap => "special_bonus_unique_nevermore_1",
            Talent::RazeAttackDamage => "special_bonus_unique_nevermore_raze_procsattacks",
            Talent::FeastCastSpeed => "special_bonus_unique_nevermore_frenzy_castspeed",
            Talent::Armor => "special_bonus_armor_5",
            Talent::SpellLifesteal => "special_bonus_spell_lifesteal_8",
            Talent::RotSlow => "special_bonus_unique_pudge_4",
            Talent::HookDamage => "special_bonus_unique_pudge_7",
            Talent::HookCooldown => "special_bonus_unique_pudge_5",
            Talent::DismemberDuration => "special_bonus_unique_pudge_6",
            Talent::FleshHeapBonuses => "special_bonus_unique_pudge_1",
            Talent::DismemberDamage => "special_bonus_unique_pudge_3",
        }
    }
}

/// A hero's whole tree, indexed by [`TalentSlot`].
pub type TalentTree = [Talent; TalentSlot::COUNT as usize];

/// The hero level a talent opens at, given what is learned already: its
/// tier's level, or, once the other talent of its tier is taken,
/// [`rules::TALENT_SECOND_LEVELS`].
#[inline]
pub fn talent_floor(slot: TalentSlot, learned: Talents) -> u8 {
    let tier = usize::from(slot.tier());
    if learned.has(slot.other()) {
        rules::TALENT_SECOND_LEVELS[tier]
    } else {
        rules::TALENT_LEVELS[tier]
    }
}

/// The hero level the Attribute Bonus waits for to take one more level
/// above `level`.
#[inline]
pub const fn attribute_bonus_floor(level: u8) -> u8 {
    rules::ATTRIBUTE_BONUS_LEVELS_APART * (level + 1)
}

/// Armor the generic `special_bonus_armor_5` gives: its imported `value`.
pub const TALENT_ARMOR: i32 = crate::heroes_data::SPECIAL_BONUS_ARMOR_5
    .value("value")
    .first()
    .int();

/// Percent of spell damage the generic `special_bonus_spell_lifesteal_8` heals: its imported
/// `value`.
pub const TALENT_SPELL_LIFESTEAL_PCT: i32 = crate::heroes_data::SPECIAL_BONUS_SPELL_LIFESTEAL_8
    .value("value")
    .first()
    .int();

/// Whether a hero with a tree has learned the talent whose Dota key is `key`.
#[inline]
pub fn has_talent_key(tree: &TalentTree, learned: Talents, key: &str) -> bool {
    tree.iter()
        .enumerate()
        .any(|(at, held)| held.key() == key && learned.has(TalentSlot(at as u8)))
}

/// Whether a hero with a tree has learned a talent.
#[inline]
pub fn has_talent(tree: &TalentTree, learned: Talents, talent: Talent) -> bool {
    tree.iter()
        .enumerate()
        .any(|(at, held)| *held == talent && learned.has(TalentSlot(at as u8)))
}
