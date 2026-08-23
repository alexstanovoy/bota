//! The match played here against the same match played over a socket.

use crate::{Lesson, Nothing, Role, Yard};

/// A yard that plays where it is told, with everything else the same.
fn yard(builtin: bool) -> Yard {
    Yard {
        builtin,
        ..Yard::default()
    }
}

/// One seat's chair, for a match that stops at the end of a lesson.
fn chair(name: &str, lesson: Lesson) -> crate::Chair {
    crate::Chair {
        addr: String::new(),
        name: name.to_string(),
        hero: bota_proto::HeroId(0),
        limit: Some(lesson.ticks()),
        role: Role::Mid,
        lesson,
        until: None,
    }
}

#[test]
fn a_match_played_here_repeats_itself_to_the_mark() {
    // Two seats meeting at every tick is lockstep with the sockets taken out,
    // and lockstep that depended on which thread woke first would be no use
    // for teaching anything.
    let lesson = Lesson::HoldTheLane;
    let played = |seed| {
        let (one, two) = (chair("one", lesson), chair("two", lesson));
        yard(true)
            .play_a_match(seed, &mut Nothing, &mut Nothing, &one, &two)
            .expect("a match")
    };
    let first = played(7);
    let again = played(7);
    assert_eq!(
        first.0.card, again.0.card,
        "the same seed is the same match"
    );
    assert_eq!(first.1.card, again.1.card);
    assert_eq!(first.0.ticks, again.0.ticks, "and it runs to the same tick");
}

#[test]
fn a_seat_that_gets_up_does_not_hold_the_tick() {
    // A match whose seats stop at different ticks must still end. Over the
    // wire the straggler timeout sees to that; here an empty chair is simply
    // not one a tick waits on.
    let long = chair("one", Lesson::HoldTheLane);
    let short = crate::Chair {
        limit: Some(300),
        ..chair("two", Lesson::HoldTheLane)
    };
    let (first, second) = yard(true)
        .play_a_match(11, &mut Nothing, &mut Nothing, &long, &short)
        .expect("a match that ends");
    assert!(first.ticks >= second.ticks, "the one that stayed saw more");
    assert!(
        second.ticks >= 300,
        "and the one that left saw what it came for"
    );
}
