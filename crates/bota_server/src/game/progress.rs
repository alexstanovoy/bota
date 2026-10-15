//! Progress inside an action, counted in beats.

use crate::game::rules;

/// Beats in one tick of animation at [`rules::BASE_ATTACK_SPEED`].
pub const BEATS_PER_TICK: u32 = 1000;

/// Beats in one millisecond of animation.
pub const BEATS_PER_MS: u32 = BEATS_PER_TICK * rules::TICKS_PER_SECOND / 1000;

const _: () = assert!((BEATS_PER_TICK * rules::TICKS_PER_SECOND).is_multiple_of(1000));

/// Beats a span of milliseconds lasts.
pub const fn beats(ms: u32) -> u32 {
    ms * BEATS_PER_MS
}

/// Beats one tick adds to an attack at a speed, clamped to
/// `MIN_ATTACK_SPEED..=MAX_ATTACK_SPEED`. [`rules::BASE_ATTACK_SPEED`] adds
/// [`BEATS_PER_TICK`].
pub fn attack_gain(attack_speed: i32) -> u32 {
    let speed = attack_speed.clamp(rules::MIN_ATTACK_SPEED, rules::MAX_ATTACK_SPEED);
    BEATS_PER_TICK * speed as u32 / rules::BASE_ATTACK_SPEED as u32
}

/// The overshoot past a mark, which the next phase starts at. Absent while
/// the mark is not reached.
pub const fn cross(progress: u32, mark: u32) -> Option<u32> {
    progress.checked_sub(mark)
}
