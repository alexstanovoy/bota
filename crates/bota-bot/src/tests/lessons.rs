//! Roles, the lanes they belong in, and what each lesson pays for.

use bota_proto::{SlotId, Team, UnitKind};

use crate::tests::{DIRE_HOME, RADIANT_HOME, a_tick, unit};
use crate::{Card, Carried, Field, Lane, Lesson, Moment, Role, Which, lane_of, score};

/// The field of a tick with the creeps given, for the role given.
fn field_of(view: &bota_proto::WorldView, role: Role) -> Field<'_> {
    Field::of(view, SlotId(0), role).expect("the seat is in the tick")
}

/// What one lesson pays for one tick, starting from nothing remembered.
///
/// Through the one door every lesson is reached by: hand it a moment, take one
/// number.
fn paid(view: &bota_proto::WorldView, lesson: Lesson, events: &[bota_proto::EventKind]) -> f32 {
    paid_on(view, lesson, events, (0, 0, 0), &mut Carried::default())
}

/// The same, saying what the scoreboard moved by and remembering across ticks.
fn paid_on(
    view: &bota_proto::WorldView,
    lesson: Lesson,
    events: &[bota_proto::EventKind],
    scored: (u16, u16, u16),
    carried: &mut Carried,
) -> f32 {
    let field = field_of(view, Role::Mid);
    let lane = lane_of(&field, Role::Mid);
    score(
        lesson,
        &Moment {
            field: &field,
            lane: lane.as_ref(),
            events,
            took: scored.0,
            killed: scored.1,
            died: scored.2,
        },
        carried,
    )
}

/// A tick with one of their creeps standing where it is put.
fn a_tick_with_their_creep(at: (i32, i32)) -> bota_proto::WorldView {
    a_tick(vec![unit(30, UnitKind::CreepMelee, Team::Dire, at, 400)], 0)
}

/// A tick with one of their towers standing where it is put.
fn a_tick_with_their_tower(at: (i32, i32)) -> bota_proto::WorldView {
    let mut view = a_tick(Vec::new(), 0);
    view.units
        .push(crate::tests::building(80, UnitKind::Tower, Team::Dire, at));
    view
}

/// A blow the seat's own hero landed on the unit named.
fn blow_on(view: &bota_proto::WorldView, target: u32) -> bota_proto::EventKind {
    bota_proto::EventKind::Damaged {
        source: field_of(view, Role::Mid).me.map(|me| me.id),
        target: crate::tests::id(target),
        amount: 40,
        kind: bota_proto::DamageKind::Physical,
        crit: false,
    }
}

/// One of their towers falling to the seat's own hero.
fn tower_falls(view: &bota_proto::WorldView) -> bota_proto::EventKind {
    bota_proto::EventKind::Died {
        unit: crate::tests::id(80),
        killer: field_of(view, Role::Mid).me.map(|me| me.id),
        denied: false,
    }
}

#[test]
fn the_safe_lane_of_one_side_is_the_hard_lane_of_the_other() {
    // The whole reason a role is told apart from a lane. Getting this backwards
    // would teach one side to stand in the other side's lane, and every mark
    // after it would be paid for the wrong habit.
    assert_eq!(Role::Carry.lane(Team::Radiant), Which::Bottom);
    assert_eq!(Role::Carry.lane(Team::Dire), Which::Top);
    assert_eq!(Role::Offlane.lane(Team::Radiant), Which::Top);
    assert_eq!(Role::Offlane.lane(Team::Dire), Which::Bottom);
    assert_eq!(
        Role::Support.lane(Team::Radiant),
        Role::Carry.lane(Team::Radiant)
    );
    assert_eq!(
        Role::Roamer.lane(Team::Dire),
        Role::Offlane.lane(Team::Dire)
    );
    for team in [Team::Radiant, Team::Dire] {
        assert_eq!(Role::Mid.lane(team), Which::Mid, "the middle is the middle");
    }
}

#[test]
fn every_role_is_a_number_and_the_same_number_back() {
    for number in 1..=5u8 {
        let role = Role::of(number).expect("one to five are roles");
        assert_eq!(role.number(), number);
        assert!(role.at() < crate::ROLES);
    }
    assert_eq!(Role::of(0), None, "there is no role nought");
    assert_eq!(Role::of(6), None, "and none past five");
}

#[test]
fn a_lane_runs_from_its_own_end() {
    let radiant = bota_proto::Vec2::from_ints(RADIANT_HOME.0, RADIANT_HOME.1);
    let dire = bota_proto::Vec2::from_ints(DIRE_HOME.0, DIRE_HOME.1);
    for which in [Which::Top, Which::Mid, Which::Bottom] {
        let ours = Lane::of(which, Team::Radiant, radiant, dire);
        let theirs = Lane::of(which, Team::Dire, radiant, dire);
        assert_eq!(ours.route.first(), Some(&radiant), "our end comes first");
        assert_eq!(theirs.route.first(), Some(&dire), "and theirs for them");
        assert!(
            (ours.length() - theirs.length()).abs() < 1.0,
            "the same lane is the same length from either end"
        );
        // Standing at its own fountain is nought along the lane and a long way
        // along it for the other side.
        assert!(ours.how_far_along(radiant) < 100.0);
        assert!(theirs.how_far_along(radiant) > ours.length() - 100.0);
    }
}

#[test]
fn the_lessons_are_a_ladder_of_lengths_and_each_names_its_own_file() {
    let ladder: Vec<Lesson> = (1..=crate::LESSONS as u8)
        .map(|at| Lesson::of(at).expect("one of them"))
        .collect();
    assert_eq!(Lesson::of(0), None, "they count from one");
    assert_eq!(
        Lesson::of(crate::LESSONS as u8 + 1),
        None,
        "and the ladder ends where it says it does"
    );
    assert_eq!(
        ladder.iter().map(|one| one.ticks()).collect::<Vec<_>>(),
        vec![
            300,
            900,
            1200,
            3000,
            7 * crate::MINUTE,
            20 * crate::MINUTE,
            30 * crate::MINUTE,
            35 * crate::MINUTE
        ]
    );
    for pair in ladder.windows(2) {
        assert!(
            pair[0].ticks() < pair[1].ticks(),
            "{} runs no longer than {}",
            pair[0].name(),
            pair[1].name()
        );
    }
    assert_eq!(
        Lesson::longest(),
        Lesson::GrowStrong,
        "and a match run to its clock has scored them all"
    );
    for rung in &crate::LADDER {
        assert!(
            rung.scored_in.starts_with("marks/") && rung.scored_in.ends_with(".rs"),
            "{} says which file scores it",
            rung.name
        );
    }
}

#[test]
fn every_lesson_counts_only_the_ticks_inside_its_own_window() {
    for rung in &crate::LADDER {
        assert!(rung.lesson.covers(0), "{} covers the horn", rung.name);
        assert!(
            rung.lesson.covers(rung.ticks - 1),
            "{} covers its last tick",
            rung.name
        );
        assert!(
            !rung.lesson.covers(rung.ticks),
            "{} stops at its own clock",
            rung.name
        );
    }
    let longest = Lesson::longest().ticks();
    assert_eq!(
        crate::LADDER
            .iter()
            .filter(|rung| rung.lesson.covers(longest - 1))
            .count(),
        1,
        "and only the longest is still counting at the end of a match"
    );
}

/// What every lesson paid for one tick, through a marker rather than through
/// one lesson's own function.
fn marked(marker: &mut crate::Marker, view: &bota_proto::WorldView) -> Card {
    let field = field_of(view, Role::Mid);
    let lane = lane_of(&field, Role::Mid);
    marker.tick(&Moment {
        field: &field,
        lane: lane.as_ref(),
        events: &[],
        took: 0,
        killed: 0,
        died: 0,
    })
}

#[test]
fn a_lesson_is_paid_over_the_clock_it_is_given_rather_than_its_rungs() {
    // A stage of a plan names its own clock, and that has to move what the
    // lesson is paid for and not only when the match stops. Otherwise a stage
    // asking for a longer clock runs the extra ticks for nothing and reports
    // the very same mark.
    let bare = crate::tests::a_tick_holding(&[], 600);
    let carrying = crate::tests::a_tick_holding(&[crate::QUELLING, crate::TANGO], 285);
    assert!(
        bare.tick > Lesson::StockUp.ticks(),
        "the tick these are scored on is past the rung's own clock"
    );

    let mut shut = crate::Marker::new();
    marked(&mut shut, &bare);
    assert_eq!(
        marked(&mut shut, &carrying).of(Lesson::StockUp),
        0.0,
        "the rung's clock ran out long before this tick"
    );

    let mut open = crate::Marker::paid_until(Lesson::StockUp, bare.tick + 1);
    marked(&mut open, &bare);
    let bought = marked(&mut open, &carrying).of(Lesson::StockUp);
    assert!(bought > 0.0, "a longer clock is paid over: {bought}");
}

#[test]
fn stocking_up_pays_for_the_gold_that_left() {
    let bare = crate::tests::a_tick_holding(&[], 600);
    let mut carried = Carried::default();
    assert_eq!(
        paid_on(&bare, Lesson::StockUp, &[], (0, 0, 0), &mut carried),
        0.0,
        "the first tick has nothing to compare against"
    );
    let carrying = crate::tests::a_tick_holding(&[crate::QUELLING, crate::TANGO], 285);
    let bought = paid_on(&carrying, Lesson::StockUp, &[], (0, 0, 0), &mut carried);
    assert!(bought > 0.0, "and buying pays: {bought}");
    let sold = paid_on(&bare, Lesson::StockUp, &[], (0, 0, 0), &mut carried);
    assert_eq!(sold, 0.0, "selling gold back is not spending it");
}

#[test]
fn finding_the_lane_pays_for_the_spot_and_not_for_the_line() {
    // A fountain sits on the line its lane runs along, so a lesson paid for the
    // line alone is answered by never leaving it.
    let at_home = paid(
        &crate::tests::a_tick_at((RADIANT_HOME.0, RADIANT_HOME.1), Vec::new(), 0),
        Lesson::FindTheLane,
        &[],
    );
    let spot = {
        let view = a_tick(Vec::new(), 0);
        let field = field_of(&view, Role::Mid);
        lane_of(&field, Role::Mid)
            .expect("the fountains are in sight")
            .where_they_meet()
    };
    let out_there = paid(
        &crate::tests::a_tick_at((spot.x.to_int(), spot.y.to_int()), Vec::new(), 0),
        Lesson::FindTheLane,
        &[],
    );
    assert!(
        out_there > at_home * 4.0,
        "the meeting spot beats the fountain: {out_there} against {at_home}"
    );
}

#[test]
fn there_is_no_distance_at_which_going_home_is_worth_nothing() {
    // A flat floor would leave a bot that had wandered off with the same marks
    // wherever it went, and nothing in the numbers pointing back.
    let marks_at = |me: (i32, i32)| {
        paid(
            &crate::tests::a_tick_at(me, Vec::new(), 0),
            Lesson::FindTheLane,
            &[],
        )
    };
    let out_there = marks_at((2000, 14000));
    let nearer = marks_at((4000, 12000));
    assert!(out_there > 0.0, "however far off, it is not nothing");
    assert!(
        nearer > out_there,
        "and a step home is worth something: {nearer} against {out_there}"
    );
}

#[test]
fn a_step_towards_the_spot_pays_from_the_first_one() {
    // The whole of what a random walk stumbles into is a fraction of a mark, so
    // a lesson that only paid for arriving would pay the same for setting off
    // the right way as the wrong way.
    let walk = |from: (i32, i32), to: (i32, i32)| {
        let mut carried = Carried::default();
        let there = crate::tests::a_tick_at(from, Vec::new(), 0);
        let first = paid_on(&there, Lesson::FindTheLane, &[], (0, 0, 0), &mut carried);
        let then = crate::tests::a_tick_at(to, Vec::new(), 0);
        paid_on(&then, Lesson::FindTheLane, &[], (0, 0, 0), &mut carried) - first
    };
    let onwards = walk((7000, 7200), (7020, 7220));
    let backwards = walk((7000, 7200), (6980, 7180));
    assert!(onwards > 0.0, "a step the right way pays: {onwards}");
    assert!(backwards < 0.0, "and one the wrong way costs: {backwards}");
}

#[test]
fn no_marks_are_paid_for_the_spot_changing_under_it() {
    // The nearest creep is a different creep from one tick to the next, and the
    // ground that opens up when it changes is nobody's walking.
    let view = a_tick(Vec::new(), 0);
    let mut carried = Carried {
        was_off: Some(90_000.0),
        ..Carried::default()
    };
    let jumped = paid_on(&view, Lesson::FindTheLane, &[], (0, 0, 0), &mut carried);
    assert!(
        jumped < 0.1,
        "a spot that leapt across the map pays no more than a walk could: {jumped}"
    );
}

#[test]
fn holding_the_lane_wants_the_meeting_spot_before_its_wave_exists() {
    // Three ticks in four of it run before the first wave walks out. With
    // nowhere to be for those, what is left pays the same for the fountain as
    // for the lane, and that is most of the lesson.
    let bare = paid(&a_tick(Vec::new(), 0), Lesson::HoldTheLane, &[]);
    let at_home = paid(
        &crate::tests::a_tick_at((RADIANT_HOME.0, RADIANT_HOME.1), Vec::new(), 0),
        Lesson::HoldTheLane,
        &[],
    );
    assert!(
        bare > at_home,
        "standing out beats standing at home even with no wave: {bare} against {at_home}"
    );
    let with_a_wave = a_tick(
        vec![unit(
            30,
            UnitKind::CreepMelee,
            Team::Radiant,
            (7200, 7400),
            500,
        )],
        0,
    );
    assert!(
        paid(&with_a_wave, Lesson::HoldTheLane, &[]) > 0.0,
        "and once one is out it is paid for standing with it"
    );
}

#[test]
fn holding_the_wrong_lane_is_worth_less_than_holding_the_right_one() {
    let mine = unit(30, UnitKind::CreepMelee, Team::Radiant, (7200, 7400), 500);
    let view = a_tick(vec![mine], 0);
    let field = field_of(&view, Role::Mid);
    let mid = lane_of(&field, Role::Mid).expect("the fountains are in sight");
    let safe = lane_of(&field, Role::Carry).expect("the fountains are in sight");
    let on = |lane: &Lane| {
        score(
            Lesson::HoldTheLane,
            &Moment {
                field: &field,
                lane: Some(lane),
                events: &[],
                took: 0,
                killed: 0,
                died: 0,
            },
            &mut Carried::default(),
        )
    };
    assert!(
        on(&mid) > on(&safe),
        "a seat standing in the middle is doing the middle's job, not the safe lane's"
    );
}

#[test]
fn a_creep_taken_is_worth_ten_blows_that_only_land() {
    let view = a_tick_with_their_creep((7600, 7800));
    let none = paid(&view, Lesson::MeetTheWave, &[]);
    let one_blow = paid(&view, Lesson::MeetTheWave, &[blow_on(&view, 30)]);
    let one_creep = paid_on(
        &view,
        Lesson::MeetTheWave,
        &[],
        (1, 0, 0),
        &mut Carried::default(),
    );
    assert!(one_blow > none, "a blow that lands is worth something");
    assert!(
        ((one_creep - none) - (one_blow - none) * 10.0).abs() < 1e-3,
        "and a creep taken is worth ten of them"
    );
}

#[test]
fn cutting_down_its_own_wave_is_worth_nothing() {
    // Paid for the swing alone, a lesson that wants blows is answered by
    // hitting whatever is nearest, and what is nearest is usually its own.
    let theirs = unit(40, UnitKind::CreepMelee, Team::Dire, (7600, 7800), 400);
    let mine = unit(41, UnitKind::CreepMelee, Team::Radiant, (7300, 7500), 400);
    let view = a_tick(vec![theirs, mine], 0);
    let none = paid(&view, Lesson::MeetTheWave, &[]);
    let on_theirs = paid(&view, Lesson::MeetTheWave, &[blow_on(&view, 40)]);
    let on_ours = paid(&view, Lesson::MeetTheWave, &[blow_on(&view, 41)]);
    assert!(on_theirs > none, "a blow on the other side pays");
    assert_eq!(on_ours, none, "one on its own pays nothing");
}

#[test]
fn blows_of_somebody_else_are_not_the_bots_doing() {
    let view = a_tick_with_their_creep((7600, 7800));
    let theirs = bota_proto::EventKind::Damaged {
        source: Some(crate::tests::id(50)),
        target: crate::tests::id(30),
        amount: 40,
        kind: bota_proto::DamageKind::Physical,
        crit: false,
    };
    assert_eq!(
        paid(&view, Lesson::MeetTheWave, &[theirs]),
        paid(&view, Lesson::MeetTheWave, &[]),
        "somebody else's blow is somebody else's"
    );
}

#[test]
fn working_the_lane_is_scored_exactly_as_meeting_the_wave() {
    // The longer match asks for the same habit held for longer, so the two are
    // one formula rather than a copy that could drift.
    let view = a_tick_with_their_creep((7600, 7800));
    let blow = blow_on(&view, 30);
    assert_eq!(
        paid(&view, Lesson::WorkTheLane, std::slice::from_ref(&blow)),
        paid(&view, Lesson::MeetTheWave, std::slice::from_ref(&blow)),
    );
}

#[test]
fn a_tower_taken_sooner_is_worth_more_than_the_same_tower_taken_later() {
    let felled = |at: u32| {
        let mut view = a_tick_with_their_tower((8000, 8200));
        view.tick = at;
        let died = tower_falls(&view);
        let mut carried = Carried::default();
        let marks = paid_on(
            &view,
            Lesson::TakeTheTowers,
            std::slice::from_ref(&died),
            (0, 0, 0),
            &mut carried,
        );
        (marks, carried.towers_down)
    };
    let (early, down) = felled(600);
    let (late, _) = felled(20 * crate::MINUTE);
    assert_eq!(down, 1, "and it counts as one down");
    assert!(
        early > late * 2.0,
        "sooner is worth more: {early} against {late}"
    );
    assert!(late > 0.0, "however late, it is still worth something");
}

#[test]
fn every_tower_after_the_first_is_worth_more_than_the_one_before() {
    let view = a_tick_with_their_tower((8000, 8200));
    let died = tower_falls(&view);
    let after = |already| {
        let mut carried = Carried {
            towers_down: already,
            ..Carried::default()
        };
        paid_on(
            &view,
            Lesson::TakeTheTowers,
            std::slice::from_ref(&died),
            (0, 0, 0),
            &mut carried,
        )
    };
    let (first, second, third) = (after(0), after(1), after(2));
    assert!(
        second > first,
        "the second beats the first: {second} against {first}"
    );
    assert!(
        ((second - first) - (third - second)).abs() < 1e-3,
        "and each one after it is worth one more of the first"
    );
}

#[test]
fn hitting_a_tower_pays_before_any_of_them_has_fallen() {
    // Multiplied by the towers already taken, as the plain reading of the
    // formula would have it, the whole of the damage before the first one is
    // worth nothing at all — and nothing is what a model has to go on until it
    // stumbles into felling one.
    let view = a_tick_with_their_tower((8000, 8200));
    let none = paid(&view, Lesson::TakeTheTowers, &[]);
    let hit = paid(&view, Lesson::TakeTheTowers, &[blow_on(&view, 80)]);
    assert!(
        hit > none,
        "the first hit on a tower pays: {hit} against {none}"
    );
}

#[test]
fn a_kill_pays_and_a_death_costs() {
    let view = a_tick_with_their_tower((8000, 8200));
    let quiet = paid(&view, Lesson::TakeTheTowers, &[]);
    let scored = |killed, died| {
        paid_on(
            &view,
            Lesson::TakeTheTowers,
            &[],
            (0, killed, died),
            &mut Carried::default(),
        ) - quiet
    };
    assert!(scored(1, 0) > 0.0, "killing one of them pays");
    assert!(scored(0, 1) < 0.0, "and dying costs");
    assert!(
        scored(1, 0) > -scored(0, 1),
        "a kill is worth more than a death costs, or the lesson teaches hiding"
    );
}

#[test]
fn keeping_whole_pays_only_out_where_it_can_be_lost() {
    // Paid wherever it stood, the surest way to full health, full mana and no
    // deaths at all is never to leave the fountain.
    let at = |me: (i32, i32)| {
        let mut view = crate::tests::a_tick_at(me, Vec::new(), 0);
        view.units.retain(|unit| unit.kind != UnitKind::Tower);
        paid(&view, Lesson::TakeTheTowers, &[])
    };
    let at_home = at((RADIANT_HOME.0, RADIANT_HOME.1));
    let out_there = at((9000, 9200));
    assert_eq!(at_home, 0.0, "its own fountain is worth nothing");
    assert!(
        out_there > 0.0,
        "and being whole out there is worth something: {out_there}"
    );
}

#[test]
fn a_hero_at_full_is_whole_and_one_at_nothing_is_not() {
    let whole_at = |hp: i32| {
        let mut view = a_tick(Vec::new(), 0);
        for body in &mut view.units {
            if body.kind == UnitKind::Hero && body.owner == Some(SlotId(0)) {
                body.max_hp = 600;
                body.hp = hp;
                body.max_mana = 300;
                body.mana = 300;
            }
        }
        crate::wholeness(&field_of(&view, Role::Mid))
    };
    assert_eq!(whole_at(600), 1.0, "full health and full mana is all of it");
    assert_eq!(whole_at(300), 0.75, "half the health is half of that half");
    assert_eq!(whole_at(0), 0.5, "and none of it leaves the mana");
}

#[test]
fn a_lesson_pays_for_its_own_and_for_nothing_else() {
    // Every lesson's marks are its own, so the six numbers of a card can be
    // read against each other and against themselves a week later. They used to
    // keep a quarter of the habit before them, and at the last rung that
    // quarter was one mark against three hundred — which no selection can see,
    // and every bred model had forgotten how to shop by the end of the ladder.
    let view = a_tick_with_their_creep((7600, 7800));
    let blow = blow_on(&view, 30);
    for rung in &crate::LADDER {
        let quiet = paid(&view, rung.lesson, &[]);
        let struck = paid(&view, rung.lesson, std::slice::from_ref(&blow));
        assert_eq!(
            struck > quiet,
            matches!(rung.lesson, Lesson::MeetTheWave | Lesson::WorkTheLane),
            "{} pays for a blow on their creep only if that is what it is for",
            rung.name
        );
    }
    let bare = crate::tests::a_tick_holding(&[], 600);
    let carrying = crate::tests::a_tick_holding(&[crate::BOOTS], 100);
    for rung in &crate::LADDER {
        let mut carried = Carried::default();
        paid_on(&bare, rung.lesson, &[], (0, 0, 0), &mut carried);
        let after = paid_on(&carrying, rung.lesson, &[], (0, 0, 0), &mut carried);
        let none = paid_on(&bare, rung.lesson, &[], (0, 0, 0), &mut Carried::default());
        assert_eq!(
            after > none,
            matches!(rung.lesson, Lesson::StockUp | Lesson::GrowStrong),
            "{} pays for shopping only if that is what it is for",
            rung.name
        );
    }
}

#[test]
fn a_card_holds_one_number_a_lesson() {
    let mut card = Card::new();
    assert_eq!(card.marks.len(), crate::LESSONS, "one a lesson");
    card.marks[Lesson::StockUp.at()] = 4.0;
    let mut other = Card::new();
    other.marks[Lesson::StockUp.at()] = 2.0;
    card.add(&other);
    assert_eq!(card.of(Lesson::StockUp), 6.0, "adding puts them together");
    assert_eq!(
        card.over(3).of(Lesson::StockUp),
        2.0,
        "and dividing spreads them over the matches"
    );
    assert_eq!(
        card.lines().len(),
        crate::LESSONS,
        "and it prints one a line"
    );
}

#[test]
fn growing_rich_counts_the_purse_and_the_goods_and_falls_as_well_as_rises() {
    // Net worth, not the purse: spending moves gold from one side of it to the
    // other and must leave the number where it was, or the lesson pays for
    // hoarding.
    let mut carried = Carried::default();
    let purse = crate::tests::a_tick_holding(&[], 600);
    assert_eq!(
        paid_on(&purse, Lesson::GrowRich, &[], (0, 0, 0), &mut carried),
        0.0,
        "the first tick has nothing to compare against"
    );
    let spent = crate::tests::a_tick_holding(&[crate::BOOTS], 100);
    let after = paid_on(&spent, Lesson::GrowRich, &[], (0, 0, 0), &mut carried);
    assert_eq!(
        after, 0.0,
        "five hundred of gold turned into five hundred of boots"
    );

    let richer = crate::tests::a_tick_holding(&[crate::BOOTS], 400);
    let earned = paid_on(&richer, Lesson::GrowRich, &[], (0, 0, 0), &mut carried);
    assert_eq!(earned, 300.0, "and three hundred earned is three hundred");

    let poorer = crate::tests::a_tick_holding(&[], 400);
    let lost = paid_on(&poorer, Lesson::GrowRich, &[], (0, 0, 0), &mut carried);
    assert_eq!(lost, -500.0, "losing the boots costs what they were worth");
}

#[test]
fn growing_strong_pays_a_gold_half_on_earning_it_and_half_on_spending_it() {
    let mut carried = Carried::default();
    let purse = crate::tests::a_tick_holding(&[], 600);
    assert_eq!(
        paid_on(&purse, Lesson::GrowStrong, &[], (0, 0, 0), &mut carried),
        0.0,
        "the first tick has nothing to compare against"
    );

    let spent = crate::tests::a_tick_holding(&[crate::BOOTS], 100);
    assert_eq!(
        paid_on(&spent, Lesson::GrowStrong, &[], (0, 0, 0), &mut carried),
        250.0,
        "boots pay the half the gold was not paid when it was earned"
    );

    let richer = crate::tests::a_tick_holding(&[crate::BOOTS], 400);
    assert_eq!(
        paid_on(&richer, Lesson::GrowStrong, &[], (0, 0, 0), &mut carried),
        150.0,
        "three hundred earned and not yet spent is worth half of it"
    );

    let poorer = crate::tests::a_tick_holding(&[], 400);
    assert_eq!(
        paid_on(&poorer, Lesson::GrowStrong, &[], (0, 0, 0), &mut carried),
        -500.0,
        "losing the boots costs the whole of what they were worth"
    );
}

#[test]
fn hoarding_is_worth_half_of_wearing_and_growing_rich_cannot_tell_them_apart() {
    // The whole of what this lesson is for. Two hands end on the same net
    // worth off the same five hundred; one of them is wearing it.
    let start = crate::tests::a_tick_holding(&[], 100);
    let hoarded = crate::tests::a_tick_holding(&[], 600);
    let worn = crate::tests::a_tick_holding(&[crate::BOOTS], 100);
    let over = |lesson: Lesson, ended: &bota_proto::WorldView| {
        let mut carried = Carried::default();
        paid_on(&start, lesson, &[], (0, 0, 0), &mut carried);
        paid_on(ended, lesson, &[], (0, 0, 0), &mut carried)
    };
    assert_eq!(
        over(Lesson::GrowRich, &hoarded),
        over(Lesson::GrowRich, &worn),
        "growing rich is paid the same either way"
    );
    assert_eq!(over(Lesson::GrowStrong, &hoarded), 250.0);
    assert_eq!(
        over(Lesson::GrowStrong, &worn),
        500.0,
        "and growing strong pays the one that spent it twice as much"
    );
}
