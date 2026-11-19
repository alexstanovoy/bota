//! Progress inside an action, counted in beats.

use crate::rules;
use bota_proto::Fixed;

/// Beats in one tick of animation at [`rules::BASE_ATTACK_SPEED`].
pub const BEATS_PER_TICK: u32 = 1000;

/// Beats in one millisecond of animation.
pub const BEATS_PER_MS: u32 = BEATS_PER_TICK * rules::TICKS_PER_SECOND / 1000;

const _: () = assert!((BEATS_PER_TICK * rules::TICKS_PER_SECOND).is_multiple_of(1000));

/// Beats a span of milliseconds lasts.
#[inline]
pub const fn beats(ms: u32) -> u32 {
    ms * BEATS_PER_MS
}

/// Beats one tick adds to an attack at a speed, clamped to
/// `MIN_ATTACK_SPEED..=MAX_ATTACK_SPEED`. [`rules::BASE_ATTACK_SPEED`] adds
/// [`BEATS_PER_TICK`]; a fraction of attack speed adds its share, rounded down.
#[inline]
pub const fn attack_gain(attack_speed: Fixed) -> u32 {
    let speed = clamped_raw(attack_speed);
    (BEATS_PER_TICK as i64 * speed / ((rules::BASE_ATTACK_SPEED as i64) << Fixed::FRAC_BITS)) as u32
}

/// An attack speed clamped to `MIN_ATTACK_SPEED..=MAX_ATTACK_SPEED`, as its raw fixed-point
/// value.
const fn clamped_raw(attack_speed: Fixed) -> i64 {
    let least = Fixed::from_int(rules::MIN_ATTACK_SPEED).raw;
    let most = Fixed::from_int(rules::MAX_ATTACK_SPEED).raw;
    let raw = if attack_speed.raw < least {
        least
    } else if attack_speed.raw > most {
        most
    } else {
        attack_speed.raw
    };
    raw as i64
}

/// The attack speed the server times a viewed unit's attacks with: the whole number its view
/// shows and the fraction it carries beside it.
pub const fn exact_attack_speed(unit: &bota_proto::UnitView) -> Fixed {
    Fixed {
        raw: (unit.attack_speed << Fixed::FRAC_BITS) | unit.attack_speed_fraction as i32,
    }
}

/// The overshoot past a mark, which the next phase starts at. Absent while
/// the mark is not reached.
#[inline]
pub const fn cross(progress: u32, mark: u32) -> Option<u32> {
    progress.checked_sub(mark)
}

/// Whole ticks that a span of milliseconds takes at a tick rate, rounded up.
#[inline]
pub const fn ms_to_ticks_ceil(ms: u32, tick_rate: u32) -> u32 {
    (ms * tick_rate).div_ceil(1000)
}

/// Ticks left of an animation phase `ms` long once `progress` beats of it are
/// done, at an attack speed: at least one while it is not over.
#[inline]
pub const fn phase_ticks_left(ms: u32, progress: u32, attack_speed: Fixed) -> u32 {
    let left = beats(ms).saturating_sub(progress);
    let gain = attack_gain(attack_speed);
    assert!(gain > 0);
    let ticks = left.div_ceil(gain);
    if ticks == 0 { 1 } else { ticks }
}

/// Milliseconds of an attack animation at its current speed, clamped as the cycle is.
/// Shared by simulation projections and clients displaying the same duration.
pub fn attack_duration(ms: u32, attack_speed: Fixed) -> u32 {
    let base = (rules::BASE_ATTACK_SPEED as i64) << Fixed::FRAC_BITS;
    (i64::from(ms) * base / clamped_raw(attack_speed)) as u32
}
