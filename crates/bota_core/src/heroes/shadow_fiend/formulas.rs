//! What Shadow Fiend's razes, souls, presence and requiem are worth, as pure
//! functions of the numbers a player may know. Which talents a hero has
//! learned and how many souls it holds are parameters.

use bota_proto::{Angle, EffectId, Fixed, Talents, UnitView, Vec2};

use super::SHADOW_FIEND_TALENTS;
use crate::{Talent, has_talent, point_ahead, rules};

/// Where a raze lands: its own reach from the caster, along the line the
/// caster faces. `reach` indexes [`rules::RAZE_DISTANCE`].
#[inline]
pub fn raze_landing(from: Vec2, facing: Angle, reach: usize) -> Vec2 {
    point_ahead(from, facing, Fixed::from_int(rules::RAZE_DISTANCE[reach]))
}

/// The burn of a raze at a level, with every soul held (to the most it
/// counts) adding [`rules::RAZE_DAMAGE_PER_SOUL`].
#[inline]
pub fn raze_damage(level: usize, souls: u32) -> i32 {
    rules::RAZE_DAMAGE[level]
        + rules::RAZE_DAMAGE_PER_SOUL * souls.min(rules::SOULS_HELD_MAX) as i32
}

/// What each stack of a raze burns at a level, the level-10 talent
/// included when `talented`.
#[inline]
pub fn raze_stack_damage(level: usize, talented: bool) -> i32 {
    rules::RAZE_STACK_DAMAGE[level]
        + if talented {
            rules::TALENT_RAZE_STACK_DAMAGE
        } else {
            0
        }
}

/// What a raze at a level burns with `souls` held on a target that carries
/// `stacks` of the caster's earlier ones, before any resistance.
#[inline]
pub fn raze_burn(level: usize, souls: u32, stacks: i32, talented: bool) -> i32 {
    raze_damage(level, souls)
        .saturating_add(raze_stack_damage(level, talented).saturating_mul(stacks))
}

/// Attack damage the souls held are worth at a hero level: tenths of a point
/// a soul that grow every [`rules::SOUL_DAMAGE_LEVELS`] levels, the sum
/// rounded down.
#[inline]
pub fn soul_damage(souls: u32, hero_level: i32) -> i32 {
    assert!(hero_level >= 1);
    let souls = souls.min(rules::SOULS_HELD_MAX) as i32;
    let tenths = rules::SOUL_DAMAGE_TENTHS
        + rules::SOUL_DAMAGE_TENTHS_GAIN * (hero_level / rules::SOUL_DAMAGE_LEVELS);
    souls * tenths / 10
}

/// How many souls a hero that gathers them may gather from kills.
#[inline]
pub fn soul_cap(talented: bool) -> u32 {
    if talented {
        rules::NECRO_SOUL_CAP + rules::TALENT_NECRO_SOUL_CAP
    } else {
        rules::NECRO_SOUL_CAP
    }
}

/// How much armor the presence tears at a level (from one), the talent
/// included when `talented`.
#[inline]
pub fn presence_armor(level: u8, talented: bool) -> Fixed {
    rules::PRESENCE_ARMOR[usize::from(level - 1)]
        + if talented {
            rules::TALENT_PRESENCE_ARMOR
        } else {
            Fixed::ZERO
        }
}

/// How much magic resistance, in percent, the presence tears at a level (from one), the talent
/// included when `talented` (7.42).
#[inline]
pub fn presence_magic_pct(level: u8, talented: bool) -> i32 {
    rules::PRESENCE_MAGIC_PCT[usize::from(level - 1)]
        + if talented {
            rules::TALENT_PRESENCE_MAGIC_PCT
        } else {
            0
        }
}

/// The lines a requiem sends for the souls held, one to a soul.
pub fn requiem_lines(souls: u32) -> u32 {
    souls.min(rules::REQUIEM_LINES_MAX)
}

/// The lines a fallen caster's requiem sends for the souls left to him.
pub fn requiem_death_lines(souls: u32) -> u32 {
    (souls * rules::REQUIEM_DEATH_LINES_PCT / 100).min(rules::REQUIEM_LINES_MAX)
}

/// What one line of a requiem burns at a level counted from zero, the last
/// level standing for any past it.
#[inline]
pub fn requiem_line_damage(level: usize) -> i32 {
    rules::REQUIEM_LINE_DAMAGE[level.min(rules::REQUIEM_LINE_DAMAGE.len() - 1)]
}

/// How a requiem of a level (counted from zero) holds what its lines cross in fear and slow,
/// in ticks: the first line its hold by level and its share, each further line its share
/// (7.42: "a base Fear and Slow duration of 1.2/1.5/1.8s and no longer has a max duration",
/// "duration per soul ... 0.1s"); the level-20 talent lengthens the hold when `talented`. No
/// more than every line crossing it can hold a unit.
pub fn requiem_fear_ticks(level: usize, talented: bool) -> RequiemHold {
    let base = rules::REQUIEM_HOLD_TICKS[level.min(rules::REQUIEM_HOLD_TICKS.len() - 1)]
        + if talented {
            rules::TALENT_REQUIEM_HOLD_TICKS
        } else {
            0
        };
    let line = rules::REQUIEM_TICKS_PER_LINE;
    RequiemHold {
        first: base + line,
        line,
        most: base + line * rules::REQUIEM_LINES_MAX,
    }
}

/// How a requiem's lines hold what they cross: [`requiem_fear_ticks`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequiemHold {
    /// Ticks the first line holds a unit.
    pub first: u32,
    /// Ticks each further line adds.
    pub line: u32,
    /// The most the lines of one requiem can hold it.
    pub most: u32,
}

/// How wide a line of the requiem catches once it has flown so far of its
/// distance: from [`rules::REQUIEM_LINE_WIDTH_START`] out to
/// [`rules::REQUIEM_LINE_WIDTH_END`], evenly along the way, both widened by
/// the caster's AoE bonus (`requiem_line_width_start`,
/// `requiem_line_width_end`: `affected_by_aoe_increase`).
pub fn requiem_width(travelled: Fixed, distance: Fixed, aoe_bonus: i32) -> Fixed {
    let start = Fixed::from_int(crate::widened(rules::REQUIEM_LINE_WIDTH_START, aoe_bonus));
    let growth = i64::from(rules::REQUIEM_LINE_WIDTH_END - rules::REQUIEM_LINE_WIDTH_START);
    if distance.raw <= 0 {
        return start;
    }
    let grown = growth * i64::from(travelled.raw) / i64::from(distance.raw);
    start + Fixed::from_int(grown as i32)
}

/// Whether a feast shortens `caster`'s cast points: it feasts and `talents`
/// hold the level-25 talent.
pub fn feast_quickened(caster: &UnitView, talents: Talents) -> bool {
    caster
        .effects
        .iter()
        .any(|effect| effect.id == EffectId::FEASTING)
        && has_talent(&SHADOW_FIEND_TALENTS, talents, Talent::FeastCastSpeed)
}

/// Milliseconds a cast point lasts: its own length, or
/// [`rules::TALENT_FEAST_CAST_POINT_PCT`] of it while a feast runs on a caster
/// that has learned the level-25 talent.
pub fn cast_point_ms(point: u32, quickened_by_feast: bool) -> u32 {
    if quickened_by_feast {
        point * rules::TALENT_FEAST_CAST_POINT_PCT / 100
    } else {
        point
    }
}
