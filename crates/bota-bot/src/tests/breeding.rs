//! The crowd, and what breeding it does to it.

use crate::{Card, Lesson, Model, Tribe, adapted_spread, crowd_from, next_crowd, placings};

/// Cards worth the marks given at the lesson the placings are read at.
fn cards(marks: &[f32]) -> Vec<Card> {
    marks
        .iter()
        .map(|worth| {
            let mut card = Card::new();
            card.marks[LESSON.at()] = *worth;
            card
        })
        .collect()
}

/// The lesson the placings tests read.
const LESSON: Lesson = Lesson::WorkTheLane;

/// A crowd of short made-up bodies, so the shape can be tested without a model.
fn crowd(many: usize) -> Vec<Vec<f32>> {
    (0..many)
        .map(|at| vec![at as f32, -(at as f32), 1.0])
        .collect()
}

#[test]
fn the_best_places_first_and_ties_never_swap() {
    assert_eq!(placings(&cards(&[1.0, 5.0, 3.0]), LESSON), vec![1, 2, 0]);
    assert_eq!(
        placings(&cards(&[2.0, 2.0, 2.0]), LESSON),
        vec![0, 1, 2],
        "worth the same, so nothing moves"
    );
    assert_eq!(
        placings(&cards(&[f32::NAN, 1.0]), LESSON),
        vec![0, 1],
        "a match that came to nothing does not shuffle the crowd"
    );
}

#[test]
fn a_generation_keeps_its_size_and_its_survivors_untouched() {
    let tribe = Tribe {
        keep: 2,
        ..Tribe::new(6, 1)
    };
    let crowd = crowd(6);
    let placed = placings(&cards(&[0.0, 9.0, 1.0, 0.0, 5.0, 0.0]), LESSON);
    let next = next_crowd(&tribe, &crowd, &placed, 1);
    assert_eq!(next.len(), 6, "the crowd is the size it was");
    assert_eq!(next[0], crowd[1], "the best carries over unchanged");
    assert_eq!(next[1], crowd[4], "and so does the one behind it");
    for child in &next[2..] {
        assert!(
            *child != crowd[1] && *child != crowd[4],
            "every child is moved off its parent"
        );
    }
}

#[test]
fn children_are_handed_round_the_survivors() {
    // Heaped on the best one, a crowd becomes one model and its copies before
    // a lesson has finished asking anything of it.
    let tribe = Tribe {
        keep: 2,
        spread: 0.0,
        ..Tribe::new(6, 1)
    };
    let crowd = crowd(6);
    let placed = placings(&cards(&[9.0, 5.0, 0.0, 0.0, 0.0, 0.0]), LESSON);
    let next = next_crowd(&tribe, &crowd, &placed, 1);
    // With nothing added, a child is its parent, so parentage is readable.
    assert_eq!(next[2], crowd[0], "the third takes after the best");
    assert_eq!(next[3], crowd[1], "the fourth after the one behind it");
    assert_eq!(next[4], crowd[0], "and round again");
}

#[test]
fn the_matches_judged_on_move_and_the_ones_reported_on_do_not() {
    let tribe = Tribe::new(4, 3);
    assert_eq!(tribe.trials_of(1).len(), 3, "one match a trial");
    assert_ne!(
        tribe.trials_of(1),
        tribe.trials_of(2),
        "a crowd judged on the same matches every time is selected for them"
    );
    assert_eq!(
        tribe.trials_of(7),
        tribe.trials_of(7),
        "but a generation is the same matches however often it is asked"
    );
    assert_eq!(
        tribe.reported_on(),
        Tribe::new(9, 9).reported_on(),
        "what is reported on does not move, whatever the crowd"
    );
    for judged in 1..40 {
        for seed in tribe.trials_of(judged) {
            assert!(
                !tribe.reported_on().contains(&seed),
                "nothing is trained on that is later reported on"
            );
        }
    }
}

#[test]
fn the_spread_widens_over_a_fifth_and_narrows_under_it() {
    // Two survivors and ten children, laid out as next_crowd lays them: the
    // child at `at` was bred off the survivor at `(at - 2) % 2`.
    let keep = 2;
    let worths = |wins: usize| -> Vec<f32> {
        let mut all = vec![1.0, 1.0];
        all.extend((0..10).map(|at| if at < wins { 2.0 } else { 0.0 }));
        all
    };
    assert!(
        adapted_spread(0.01, &worths(3), keep) > 0.01,
        "over a fifth winning reaches further"
    );
    assert!(
        adapted_spread(0.01, &worths(1), keep) < 0.01,
        "under a fifth winning reaches shorter"
    );
    assert_eq!(
        adapted_spread(0.01, &worths(2), keep),
        0.01,
        "at exactly a fifth it stands"
    );
    assert_eq!(
        adapted_spread(0.01, &[1.0, 1.0], keep),
        0.01,
        "a crowd with no children leaves it alone"
    );
}

#[test]
fn numbers_poured_out_of_a_model_go_back_in_where_they_came_from() {
    let model = Model::fresh(3).expect("a model");
    let was = model.pour().expect("its numbers");
    assert_eq!(was.len(), model.weight_count(), "every one of them");
    let mut moved = was.clone();
    for number in &mut moved {
        *number += 0.5;
    }
    model.soak(&moved).expect("put back");
    let now = model.pour().expect("its numbers again");
    assert_eq!(now, moved, "what went in is what comes out");
    assert!(
        model.soak(&was[..was.len() - 1]).is_err(),
        "and too few numbers is refused rather than half applied"
    );
}

#[test]
fn a_crowd_from_kept_weights_keeps_them_first_and_moves_the_rest_off() {
    // What continuing a run means: the kept body plays as it is, and the
    // search resumes around it rather than from noise.
    let tribe = Tribe {
        spread: 0.1,
        ..Tribe::new(5, 1)
    };
    let body = vec![1.0, 2.0, 3.0];
    let crowd = crowd_from(&tribe, body.clone());
    assert_eq!(crowd.len(), tribe.folk, "the crowd is the size asked for");
    assert_eq!(crowd[0], body, "what was kept carries over unchanged");
    for child in &crowd[1..] {
        assert!(*child != body, "every child is moved off it");
    }
}
