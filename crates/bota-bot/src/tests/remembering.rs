//! The window of frames the model is shown.

use crate::{A_SECOND, AGES, DEEDS, Fed, HISTORY, INPUT, Learned, Mind, Model, NUMBERS, Shown};

/// A tick whose numbers all read the same, so a frame can be named by sight.
fn a_tick(at: u32) -> Shown {
    Shown {
        at,
        numbers: vec![at as f32; NUMBERS],
        allowed: vec![true; DEEDS],
    }
}

/// Which tick each of the frames it was last fed came from.
fn window(mind: &Learned) -> Vec<u32> {
    let fed = mind.what_was_fed();
    (0..HISTORY).map(|at| fed[at * NUMBERS] as u32).collect()
}

/// A mind that has been shown every tick from one up to the one given.
fn shown_every_tick_to(last: u32) -> Learned {
    let mut mind = Learned::new(Model::fresh(1).expect("a model"));
    mind.starting();
    for at in 1..=last {
        mind.choose(&a_tick(at));
    }
    mind
}

#[test]
fn the_ages_double_and_end_on_the_tick_being_decided() {
    assert_eq!(
        AGES,
        [
            16 * A_SECOND,
            8 * A_SECOND,
            4 * A_SECOND,
            2 * A_SECOND,
            A_SECOND,
            A_SECOND / 2,
            0
        ],
        "half a second back, then doubling to sixteen"
    );
    assert_eq!(
        AGES.last().copied(),
        Some(0),
        "the newest frame is the tick being decided, not one already past"
    );
    for pair in AGES.windows(2) {
        assert!(pair[0] > pair[1], "oldest first, and no age is repeated");
    }
    assert_eq!(INPUT, NUMBERS * HISTORY);
}

#[test]
fn a_filled_window_holds_exactly_the_ages_asked_for() {
    let now = 1000;
    let mind = shown_every_tick_to(now);
    assert_eq!(
        window(&mind),
        AGES.iter().map(|age| now - age).collect::<Vec<u32>>(),
        "every frame is the tick that many ticks back"
    );
}

#[test]
fn an_unfilled_window_leans_on_the_oldest_tick_seen_and_never_on_noughts() {
    // Three ticks in, everything older than the third is the first tick it
    // ever saw. Noughts would be numbers it will never be shown again.
    let mind = shown_every_tick_to(3);
    assert_eq!(window(&mind), vec![1, 1, 1, 1, 1, 1, 3]);

    let mind = shown_every_tick_to(A_SECOND + 1);
    let held = window(&mind);
    assert_eq!(
        held.last().copied(),
        Some(A_SECOND + 1),
        "the newest is now"
    );
    assert_eq!(
        held[4], 1,
        "a second back is the first tick, which is all it has"
    );
    assert_eq!(
        held[5],
        A_SECOND + 1 - A_SECOND / 2,
        "and half a second back is reached properly"
    );
}

#[test]
fn a_gap_shows_the_last_tick_seen_rather_than_sliding_the_window() {
    // The seat chooses nothing while there is nothing to choose — being dead,
    // most often. Ages are the match's ticks, so a gap must leave the other
    // frames where they are.
    let mut mind = shown_every_tick_to(600);
    for at in 900..=901 {
        mind.choose(&a_tick(at));
    }
    let held = window(&mind);
    assert_eq!(
        held.last().copied(),
        Some(901),
        "the newest is the tick now"
    );
    assert_eq!(
        held[5], 600,
        "half a second back lands at tick 886, inside the gap, so it reads as          the last tick actually seen before it"
    );
    assert_eq!(
        held[0],
        901 - 16 * A_SECOND,
        "and sixteen seconds back is still a real tick from before the gap"
    );
}

#[test]
fn what_is_remembered_is_dropped_once_it_ages_out() {
    // The window is bounded, or a long match would carry every tick of it.
    let mind = shown_every_tick_to(5000);
    let held = window(&mind);
    assert_eq!(held[0], 5000 - 16 * A_SECOND);
    assert_eq!(held.last().copied(), Some(5000));
}

#[test]
fn starting_a_match_forgets_the_one_before() {
    let mut mind = shown_every_tick_to(1000);
    mind.starting();
    mind.choose(&a_tick(1));
    assert_eq!(
        window(&mind),
        vec![1; HISTORY],
        "nothing of the last match is left to lean on"
    );
}
