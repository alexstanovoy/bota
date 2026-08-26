//! Choosing a crowd by tournament.

use crate::{Card, Lesson, SWISS_ROUNDS, Selection, Tribe, pairs_of, settled, standings};

/// A card that paid the given mark at one lesson and nothing anywhere else.
fn paid(lesson: Lesson, mark: f32) -> Card {
    let mut card = Card::new();
    card.marks[lesson.at()] = mark;
    card
}

#[test]
fn standings_order_by_margin_and_ties_never_swap() {
    assert_eq!(standings(&[1.0, 3.0, 3.0, -2.0]), vec![1, 2, 0, 3]);
    assert_eq!(standings(&[]), Vec::<usize>::new());
}

#[test]
fn neighbours_in_the_standing_pair_and_an_odd_tail_sits_out() {
    assert_eq!(pairs_of(&[3, 1, 2, 0]), vec![(3, 1), (2, 0)]);
    assert_eq!(pairs_of(&[7, 8, 9]), vec![(7, 8)]);
    assert_eq!(pairs_of(&[]), Vec::<(usize, usize)>::new());
}

#[test]
fn a_pairs_margins_mirror_each_other_and_the_anchors_never_moves() {
    let lesson = Lesson::GrowRich;
    let mut margins = vec![0.0; 3];
    let (played, failed, words) = settled(
        vec![Ok((0, 1, paid(lesson, 5.0), paid(lesson, 2.0)))],
        lesson,
        true,
        &mut margins,
    );
    assert_eq!((played, failed), (1, 0));
    assert!(words.is_none());
    assert_eq!(
        margins,
        vec![3.0, -3.0, 0.0],
        "what one gains the other loses"
    );

    settled(
        vec![Ok((2, 0, paid(lesson, 4.0), paid(lesson, 1.0)))],
        lesson,
        false,
        &mut margins,
    );
    assert_eq!(
        margins,
        vec![3.0, -3.0, 3.0],
        "only the challenger moves on the anchor match"
    );
}

#[test]
fn a_failed_bout_moves_nothing_and_is_counted() {
    let mut margins = vec![0.0; 2];
    let (played, failed, words) = settled(
        vec![Err(std::io::Error::other("gave up"))],
        Lesson::GrowRich,
        true,
        &mut margins,
    );
    assert_eq!((played, failed), (1, 1));
    assert!(words.is_some());
    assert_eq!(margins, vec![0.0, 0.0]);
}

#[test]
fn round_seeds_move_with_the_generation_and_never_repeat_within_one() {
    let tribe = Tribe::new(4, 1);
    let seeds = tribe.round_seeds(1);
    assert_eq!(
        seeds.len(),
        SWISS_ROUNDS + 1,
        "a seed a round, and the anchor match's last"
    );
    assert_eq!(
        seeds,
        tribe.round_seeds(1),
        "the same generation draws the same seeds"
    );
    assert_ne!(seeds, tribe.round_seeds(2));
    for one in 0..seeds.len() {
        for other in one + 1..seeds.len() {
            assert_ne!(seeds[one], seeds[other], "no round repeats another's");
        }
    }
}

#[test]
fn a_selection_is_spelled_and_read_back() {
    for way in [Selection::Mirror, Selection::Swiss] {
        assert_eq!(Selection::named(way.spelling()), Some(way));
    }
    assert_eq!(Selection::named("ladder"), None);
}
