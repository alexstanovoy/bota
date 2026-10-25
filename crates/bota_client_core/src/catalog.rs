//! What the client knows about abilities, items, effects and heroes, as
//! data: the rules' numbers a front end reads off a unit.

use bota_core::{
    agility_armor, agility_attack_speed, attribute_regen, attributes_at_level, hero_def,
    intelligence_magic_resist_bp, talent_floor, white_damage_least,
};
use bota_proto::{EffectId, Fixed, HeroId, TalentSlot, Talents};

/// Whether a front end lists an effect among a unit's buffs and debuffs. Dota
/// shows how long a ward or a summoned unit has left on its health bar, not
/// as a buff; a retired or unknown id is listed nowhere.
pub fn effect_listed(id: u16) -> bool {
    bota_core::effect_def(EffectId(id)).is_some()
        && !matches!(
            EffectId(id),
            EffectId::TIMED_LIFE | EffectId::FOUNTAIN_PASSIVE
        )
}

/// Hero levels at which the talent in a slot opens: the first of its tier
/// to be taken, and the other once its tier has one.
pub fn talent_levels(slot: TalentSlot) -> (u8, u8) {
    let none = Talents::default();
    (
        talent_floor(slot, none),
        talent_floor(slot, none.with(slot.other())),
    )
}

/// Whether a hero has a talent tree at all.
pub fn has_talents(hero: u16) -> bool {
    hero_def(HeroId(hero)).is_some_and(|def| def.talents.is_some())
}

/// A unit's attack damage the way Dota's panel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRange {
    /// The least and the most a swing deals without bonuses: the base
    /// range plus the primary attribute.
    pub min: i32,
    pub max: i32,
    /// What items and effects add on top of that, shown apart.
    pub bonus: i32,
}

/// Splits the range a swing draws from, as the view tells it, into the base
/// range and the bonus on top.
pub fn damage_range(unit: &bota_proto::UnitView) -> DamageRange {
    // White, as Dota draws it: the hero's base range and what its primary
    // attribute pays for every point it holds, from items too; green: flat
    // bonus damage alone.
    let base_least = unit
        .hero
        .and_then(hero_def)
        .map(|hero| white_damage_least(hero.unit, unit.level, sane(unit.attributes)));
    // Saturating: the view is a peer's word.
    let bonus = base_least.map_or(0, |base| unit.attack_damage_min.saturating_sub(base).max(0));
    DamageRange {
        min: unit.attack_damage_min.saturating_sub(bonus),
        max: unit.attack_damage_max.saturating_sub(bonus),
        bonus,
    }
}

/// One stat as Dota's panel splits it: white, what the unit's kind and level
/// and every attribute point it holds are worth; green, what items, buffs,
/// auras and talents add on top (negative for a loss).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split<T> {
    pub base: T,
    pub bonus: T,
}

impl Split<i32> {
    /// Saturating: the view is a peer's word.
    fn whole(base: i32, total: i32) -> Split<i32> {
        Split {
            base,
            bonus: total.saturating_sub(base),
        }
    }
}

impl Split<Fixed> {
    /// Saturating, on the same grounds.
    fn fixed(base: Fixed, total: Fixed) -> Split<Fixed> {
        Split {
            base,
            bonus: Fixed {
                raw: total.raw.saturating_sub(base.raw),
            },
        }
    }
}

/// Most of an attribute a view may claim that the client works with: far
/// past any hero's, so that a peer's absurd number cannot overflow what is
/// worked out of it.
const MOST_ATTRIBUTE: i32 = 10_000;

/// A fraction from the view as a percent, held to the range of a percent
/// either way.
fn percent(fraction: Fixed) -> Fixed {
    fraction.clamp(-Fixed::ONE, Fixed::ONE) * Fixed::from_int(100)
}

/// A Q16.16 number, widened, to the nearest whole, held to `i32`.
fn rounded(raw: i64) -> i32 {
    let half = 1i64 << (Fixed::FRAC_BITS - 1);
    ((raw + half) >> Fixed::FRAC_BITS).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// A view's attributes held between none and [`MOST_ATTRIBUTE`].
fn sane(has: bota_proto::Attributes) -> bota_proto::Attributes {
    let held = |f: Fixed| f.clamp(Fixed::ZERO, Fixed::from_int(MOST_ATTRIBUTE));
    bota_proto::Attributes {
        strength: held(has.strength),
        agility: held(has.agility),
        intelligence: held(has.intelligence),
    }
}

/// A hero's stats split as Dota's panel and its stats tooltip split them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatSplit {
    pub armor: Split<Fixed>,
    pub attack_speed: Split<i32>,
    pub move_speed: Split<i32>,
    pub attack_range: Split<i32>,
    /// Health and mana mended a second.
    pub hp_regen: Split<Fixed>,
    pub mana_regen: Split<Fixed>,
    /// Magic resistance, percent: white the hero's base with what its
    /// intelligence adds to it, as Dota's Base Magic Resistance.
    pub magic_resist: Split<Fixed>,
    /// Status resistance and evasion, percent: a hero has none of its own,
    /// so all of either is green.
    pub status_resist: Split<Fixed>,
    pub evasion: Split<Fixed>,
    /// Strength, agility, intelligence in whole points: white what the
    /// hero's level gives, green what anything else does.
    pub attributes: [Split<i32>; 3],
}

/// Splits a hero's stats, as the view tells them, into Dota's white and
/// green. None for anything but a hero.
pub fn stat_split(unit: &bota_proto::UnitView) -> Option<StatSplit> {
    let kind = hero_def(unit.hero?)?.unit;
    let levels = i32::from(unit.level.max(1)) - 1;
    let has = sane(unit.attributes);
    let a_second = Fixed::from_int(bota_core::rules::TICKS_PER_SECOND as i32);
    let paid = attribute_regen(has);
    let level = base_attributes(unit)?;
    let total = [has.strength, has.agility, has.intelligence].map(Fixed::to_int);
    let whole = |f: Fixed| rounded(i64::from(f.raw));
    Some(StatSplit {
        armor: Split::fixed(Fixed::from_int(kind.armor) + agility_armor(has), unit.armor),
        attack_speed: Split::whole(
            kind.attack_speed + kind.per_level.attack_speed * levels + agility_attack_speed(has),
            unit.attack_speed,
        ),
        move_speed: Split::whole(kind.move_speed, whole(unit.move_speed)),
        attack_range: Split::whole(kind.attack_range, whole(unit.attack_range)),
        hp_regen: Split::fixed((kind.hp_regen + paid.hp) * a_second, unit.hp_regen),
        mana_regen: Split::fixed((kind.mana_regen + paid.mana) * a_second, unit.mana_regen),
        // Worked out as the server's view is, so that none is told green.
        magic_resist: Split::fixed(
            percent(Fixed::from_ratio(
                kind.magic_resist_pct * 100 + intelligence_magic_resist_bp(has),
                bota_core::rules::NOMINAL_BP,
            )),
            percent(unit.magic_resist),
        ),
        status_resist: Split::fixed(Fixed::ZERO, percent(unit.status_resist)),
        evasion: Split::fixed(Fixed::ZERO, percent(unit.evasion)),
        attributes: [0, 1, 2].map(|k| Split::whole(level[k].min(total[k]), total[k])),
    })
}

/// A hero's strength, agility and intelligence of its level alone, before
/// items, souls, buffs and talents: whole points, as Dota counts them.
pub fn base_attributes(unit: &bota_proto::UnitView) -> Option<[i32; 3]> {
    let has = attributes_at_level(hero_def(unit.hero?)?.unit, unit.level);
    Some([has.strength, has.agility, has.intelligence].map(Fixed::to_int))
}
