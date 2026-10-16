//! What Shadow Fiend's razes, souls, presence and requiem are worth, as pure
//! functions of the numbers a player may know. Which talents a hero has
//! learned and how many souls it holds are parameters.

use bota_proto::{Angle, Fixed, Vec2};

use crate::{heading_of, point_along, rules};

/// Where a raze lands: its own reach from the caster, along the line the
/// caster faces. `reach` indexes [`rules::RAZE_DISTANCE`].
#[inline]
pub fn raze_landing(from: Vec2, facing: Angle, reach: usize) -> Vec2 {
    point_along(
        from,
        from + heading_of(facing),
        Fixed::from_int(rules::RAZE_DISTANCE[reach]),
    )
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

/// The lines a requiem sends for the souls held, one to a soul.
pub fn requiem_lines(souls: u32) -> u32 {
    souls.min(rules::REQUIEM_LINES_MAX)
}

/// The lines a fallen caster's requiem sends for the souls left to him.
pub fn requiem_death_lines(souls: u32) -> u32 {
    let share = rules::REQUIEM_DEATH_LINES;
    (souls * u32::from(share.num()) / u32::from(share.den())).min(rules::REQUIEM_LINES_MAX)
}

/// What one line of a requiem burns at a level counted from zero, the last
/// level standing for any past it.
#[inline]
pub fn requiem_line_damage(level: usize) -> i32 {
    rules::REQUIEM_LINE_DAMAGE[level.min(rules::REQUIEM_LINE_DAMAGE.len() - 1)]
}

/// How long a requiem line's fear lasts, and the most a unit can be held
/// in it, in ticks; the level-25 talent lengthens both when `talented`.
pub fn requiem_fear_ticks(talented: bool) -> (u32, u32) {
    if talented {
        (
            rules::REQUIEM_LINE_TICKS + rules::TALENT_REQUIEM_LINE_TICKS,
            rules::REQUIEM_HOLD_MAX_TICKS + rules::TALENT_REQUIEM_HOLD_TICKS,
        )
    } else {
        (rules::REQUIEM_LINE_TICKS, rules::REQUIEM_HOLD_MAX_TICKS)
    }
}

/// How wide a line of the requiem catches once it has flown so far of its
/// distance: from [`rules::REQUIEM_LINE_WIDTH_START`] out to
/// [`rules::REQUIEM_LINE_WIDTH_END`], evenly along the way.
pub fn requiem_width(travelled: Fixed, distance: Fixed) -> Fixed {
    let start = Fixed::from_int(rules::REQUIEM_LINE_WIDTH_START);
    let growth = i64::from(rules::REQUIEM_LINE_WIDTH_END - rules::REQUIEM_LINE_WIDTH_START);
    if distance.raw <= 0 {
        return start;
    }
    let grown = growth * i64::from(travelled.raw) / i64::from(distance.raw);
    start + Fixed::from_int(grown as i32)
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
