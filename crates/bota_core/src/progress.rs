//! Progress inside an action, counted in beats.

use crate::rules;

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
/// [`BEATS_PER_TICK`].
#[inline]
pub const fn attack_gain(attack_speed: i32) -> u32 {
    let speed = if attack_speed < rules::MIN_ATTACK_SPEED {
        rules::MIN_ATTACK_SPEED
    } else if attack_speed > rules::MAX_ATTACK_SPEED {
        rules::MAX_ATTACK_SPEED
    } else {
        attack_speed
    };
    BEATS_PER_TICK * speed as u32 / rules::BASE_ATTACK_SPEED as u32
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
pub const fn phase_ticks_left(ms: u32, progress: u32, attack_speed: i32) -> u32 {
    let left = beats(ms).saturating_sub(progress);
    let gain = attack_gain(attack_speed);
    assert!(gain > 0);
    let ticks = left.div_ceil(gain);
    if ticks == 0 { 1 } else { ticks }
}

/// Ticks between the starts of two attacks of `attack_time` milliseconds at
/// an attack speed.
#[inline]
pub const fn attack_interval_ticks(attack_time: u32, attack_speed: i32) -> u32 {
    phase_ticks_left(attack_time, 0, attack_speed)
}
