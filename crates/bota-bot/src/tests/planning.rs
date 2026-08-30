//! Reading a plan, and what it settles on.

use crate::{Lesson, Plan, Role, Selection};

/// A plan that says one thing and leaves the rest out.
const BARE: &str = "
sequence:
  - score: hold_the_lane
";

#[test]
fn a_stage_that_says_nothing_takes_the_lesson_clock_and_a_plain_crowd() {
    let terms = Plan::of(BARE).expect("a plan").terms().expect("stages");
    assert_eq!(terms.len(), 1);
    let term = &terms[0];
    assert_eq!(term.rung.lesson, Lesson::HoldTheLane);
    assert_eq!(
        term.rung.ticks,
        Lesson::HoldTheLane.ticks(),
        "the lesson's own clock, when the stage names none"
    );
    assert_eq!(term.tribe.folk, 10);
    assert_eq!(term.tribe.trials, 1);
    assert_eq!(term.tribe.lives, 30);
    assert_eq!(term.tribe.keep, 2, "a quarter of the crowd");
    assert_eq!(term.tribe.role, Role::Mid);
    assert_eq!(
        term.tribe.selection,
        Selection::Mirror,
        "mirror, unless asked"
    );
}

#[test]
fn a_swiss_stage_is_read_and_a_crowd_that_cannot_be_paired_is_refused() {
    let plan = Plan::of(
        "
sequence:
  - score: grow_rich
    selection: swiss
    population: 4
",
    )
    .expect("a plan");
    let terms = plan.terms().expect("stages");
    assert_eq!(terms[0].tribe.selection, Selection::Swiss);

    let odd = Plan::of(
        "
sequence:
  - score: grow_rich
    selection: swiss
    population: 5
",
    )
    .expect("a plan");
    let wrong = odd.terms().expect_err("an odd swiss crowd");
    assert!(wrong.contains("paired"), "{wrong}");

    let unheard = Plan::of(
        "
sequence:
  - score: grow_rich
    selection: ladder
",
    )
    .expect("a plan");
    let wrong = unheard.terms().expect_err("no such selection");
    assert!(wrong.contains("mirror or swiss"), "{wrong}");
}

#[test]
fn a_swiss_stage_that_names_matches_is_refused() {
    let plan = Plan::of(
        "
defaults:
  matches: 1
sequence:
  - score: grow_rich
    selection: swiss
    population: 4
  - score: grow_rich
    selection: swiss
    population: 4
    matches: 2
",
    )
    .expect("a plan");
    let wrong = plan.terms().expect_err("matches on a swiss stage");
    assert!(wrong.contains("stage 2"), "{wrong}");
    assert!(
        wrong.contains("does not take matches"),
        "and the defaults' matches are no offence: {wrong}"
    );
}

#[test]
fn a_stage_overrides_the_defaults_and_the_defaults_override_nothing() {
    let plan = Plan::of(
        "
defaults:
  population: 24
  generations: 5
  role: offlane
sequence:
  - score: stock_up
    ticks: 600
  - score: grow_rich
    generations: 2
    survivors: 1
    mutation: 0.5
    matches: 3
    lanes: 4
    seed: 77
    role: support
",
    )
    .expect("a plan");
    let terms = plan.terms().expect("stages");
    assert_eq!(
        terms.len(),
        2,
        "the stages stay in the order they were written"
    );

    let first = &terms[0];
    assert_eq!(first.rung.lesson, Lesson::StockUp);
    assert_eq!(first.rung.ticks, 600, "the stage's clock, not the lesson's");
    assert_eq!(first.tribe.folk, 24, "from the defaults");
    assert_eq!(first.tribe.lives, 5, "from the defaults");
    assert_eq!(
        first.tribe.keep, 6,
        "a quarter of the population it settled on"
    );
    assert_eq!(first.tribe.role, Role::Offlane);

    let second = &terms[1];
    assert_eq!(second.rung.lesson, Lesson::GrowRich);
    assert_eq!(second.tribe.folk, 24, "still from the defaults");
    assert_eq!(second.tribe.lives, 2, "the stage wins over the defaults");
    assert_eq!(second.tribe.keep, 1);
    assert_eq!(second.tribe.spread, 0.5);
    assert_eq!(second.tribe.trials, 3);
    assert_eq!(second.tribe.lanes, 4);
    assert_eq!(second.tribe.role, Role::Support);

    // The seed the stage wrote is what it draws from: the same plan with the
    // seed left out settles the same stage somewhere else.
    let unsaid = Plan::of(
        "
defaults:
  population: 24
  generations: 5
  role: offlane
sequence:
  - score: stock_up
    ticks: 600
  - score: grow_rich
    generations: 2
    survivors: 1
    mutation: 0.5
    matches: 3
    lanes: 4
    role: support
",
    )
    .expect("a plan")
    .terms()
    .expect("stages");
    assert_ne!(
        second.tribe.seed, unsaid[1].tribe.seed,
        "the seed written counts"
    );
}

#[test]
fn stages_of_one_seed_still_draw_apart() {
    let spelled = "
defaults:
  seed: 9
sequence:
  - score: grow_rich
  - score: grow_rich
";
    let terms = Plan::of(spelled).expect("a plan").terms().expect("stages");
    assert_ne!(
        terms[0].tribe.seed, terms[1].tribe.seed,
        "a stage that repeated another's seed would repeat its matches and its noise"
    );
    let again = Plan::of(spelled).expect("a plan").terms().expect("stages");
    assert_eq!(
        terms[0].tribe.seed, again[0].tribe.seed,
        "and a run still repeats to the number"
    );
    assert_eq!(terms[1].tribe.seed, again[1].tribe.seed);
}

#[test]
fn every_lesson_and_every_role_can_be_spelled() {
    for rung in &crate::LADDER {
        assert_eq!(
            Lesson::named(&rung.lesson.spelling()),
            Some(rung.lesson),
            "{} spells itself back",
            rung.name
        );
    }
    for number in 1..=5 {
        let role = Role::of(number).expect("a role");
        assert_eq!(Role::named(role.spelling()), Some(role));
    }
}

/// What a plan cannot get away with. Every one of these is hours of a run
/// thrown away if it is only noticed on the stage it sits in.
#[test]
fn nonsense_is_refused_when_the_plan_is_read() {
    let wrong = |text: &str| -> String {
        match Plan::of(text) {
            Err(said) => said,
            Ok(plan) => plan.terms().expect_err("this plan is nonsense"),
        }
    };
    assert!(wrong("sequence: []").contains("teaches nothing"));
    assert!(wrong("sequence:\n  - score: lane_keep\n").contains("no lesson is called lane_keep"));
    assert!(wrong("sequence:\n  - ticks: 300\n").contains("no score"));
    assert!(wrong("sequence:\n  - score: stock_up\n    ticks: 0\n").contains("nought ticks"));
    assert!(wrong("sequence:\n  - score: stock_up\n    population: 1\n").contains("chosen over"));
    assert!(wrong("sequence:\n  - score: stock_up\n    matches: 0\n").contains("no matches"));
    assert!(
        wrong("sequence:\n  - score: stock_up\n    generations: 0\n")
            .contains("nought generations")
    );
    assert!(
        wrong("sequence:\n  - score: stock_up\n    population: 4\n    survivors: 9\n")
            .contains("9 survivors")
    );
    assert!(wrong("sequence:\n  - score: stock_up\n    lanes: 0\n").contains("no matches at all"));
    assert!(wrong("sequence:\n  - score: stock_up\n    role: jungler\n").contains("no role"));
    assert!(
        wrong("sequence:\n  - score: stock_up\n    populaton: 10\n").contains("populaton"),
        "a field nobody reads is a setting silently thrown away"
    );
    assert!(
        wrong(
            "weights: w.safetensors
sequence:
  - score: stock_up
"
        )
        .contains("weights"),
        "where the weights live is the command's business, not the plan's"
    );
}

#[test]
fn a_stage_that_goes_wrong_says_which_one_it_was() {
    let said = Plan::of("sequence:\n  - score: stock_up\n  - score: nowhere\n")
        .expect("a plan")
        .terms()
        .expect_err("the second stage is nonsense");
    assert!(said.starts_with("stage 2:"), "{said}");
}

/// The plan shipped beside the crates, which is the ladder as it stands.
#[test]
fn the_plan_in_the_repository_settles() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../train.yaml");
    let plan = Plan::read(&path).expect("train.yaml");
    let terms = plan.terms().expect("stages");
    assert!(!terms.is_empty(), "a plan teaches something");
}
