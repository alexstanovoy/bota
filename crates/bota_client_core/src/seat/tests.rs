use bota_proto::{
    HeroId, MapId, MatchInfo, ObjectiveSet, Pick, RuneSet, ServerMsg, SlotId, Team, TickMode, Vec2,
};

use super::Seat;
use crate::session::{IngestError, Ingested, SessionError, SessionOptions};

fn info(trees: Vec<Vec2>) -> MatchInfo {
    MatchInfo {
        match_id: 1,
        map: MapId(2),
        tick_rate: 30,
        pregame_ticks: 0,
        trees,
        terrain_cells: 128,
        terrain_rle: vec![(16384, 0x80)],
        opaque_cells: Vec::new(),
        mode: TickMode::Lockstep,
        picks: vec![Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero: HeroId::PUDGE,
        }],
        shop: Vec::new(),
        fountains: [Vec2::ZERO; 2],
        shop_range: 0,
        secret_shops: Vec::new(),
        universal_shop: false,
        cheats: false,
        objectives: ObjectiveSet::NONE,
        runes: RuneSet::NONE,
        catalogue: bota_core::Catalogue::FULL.prefix(),
    }
}

fn trees() -> Vec<Vec2> {
    vec![Vec2::from_ints(128, 256), Vec2::from_ints(512, 64)]
}

fn scenery(map: u16, trees: Vec<Vec2>) -> ServerMsg {
    ServerMsg::Scenery {
        map: MapId(map),
        trees,
    }
}

fn welcome() -> ServerMsg {
    ServerMsg::Welcome {
        player_id: bota_proto::PlayerId(1),
        slot: None,
        tick_rate: 30,
        mode: TickMode::Realtime,
    }
}

#[test]
fn scenery_reconnect_keeps_the_same_trees_and_starts_the_match() {
    let mut seat = Seat::watching(SessionOptions::default());
    for _ in 0..2 {
        seat.ingest(welcome(), &mut ())
            .expect("a fresh connection welcomed");
        seat.ingest(scenery(2, trees()), &mut ())
            .expect("the same scenery on a fresh connection");
    }
    assert!(matches!(
        seat.ingest(
            ServerMsg::MatchStart {
                info: info(trees())
            },
            &mut ()
        ),
        Ok(Ingested::Started { .. })
    ));
    assert_eq!(
        refusal(&mut seat, scenery(2, trees())),
        "server sent Scenery after MatchStart"
    );
}

#[test]
fn scenery_reconnect_refuses_changed_scenery_and_duplicate_on_one_connection() {
    for changed in [scenery(1, trees()), scenery(2, Vec::new())] {
        let mut seat = Seat::watching(SessionOptions::default());
        seat.ingest(welcome(), &mut ()).expect("welcomed");
        seat.ingest(scenery(2, trees()), &mut ())
            .expect("the first scenery");
        seat.ingest(welcome(), &mut ()).expect("welcomed again");
        assert_eq!(refusal(&mut seat, changed), "server told another scenery");
    }
    let mut seat = Seat::watching(SessionOptions::default());
    seat.ingest(scenery(2, trees()), &mut ())
        .expect("the first scenery");
    seat.ingest(welcome(), &mut ()).expect("welcomed again");
    seat.ingest(scenery(2, trees()), &mut ())
        .expect("the repeated scenery");
    assert_eq!(
        refusal(&mut seat, scenery(2, trees())),
        "server sent Scenery twice"
    );
}

/// What the seat refused a message with.
fn refusal(seat: &mut Seat, msg: ServerMsg) -> &'static str {
    match seat.ingest(msg, &mut ()) {
        Err(IngestError::Session(SessionError::Stream(why))) => why,
        other => panic!("the seat took it: {other:?}"),
    }
}

#[test]
fn a_match_that_states_the_trees_it_was_told_starts() {
    let mut seat = Seat::watching(SessionOptions::default());
    assert!(matches!(
        seat.ingest(scenery(2, trees()), &mut ()),
        Ok(Ingested::Passed(_))
    ));
    assert_eq!(
        seat.scenery().map(|(map, t)| (map, t.len())),
        Some((MapId(2), 2))
    );
    let started = seat.ingest(
        ServerMsg::MatchStart {
            info: info(trees()),
        },
        &mut (),
    );
    assert!(
        matches!(started, Ok(Ingested::Started { .. })),
        "{started:?}"
    );
}

#[test]
fn a_match_start_with_other_trees_than_the_scenery_told_is_refused() {
    let mut seat = Seat::watching(SessionOptions::default());
    seat.ingest(scenery(2, Vec::new()), &mut ())
        .expect("passed");
    let why = refusal(
        &mut seat,
        ServerMsg::MatchStart {
            info: info(trees()),
        },
    );
    assert_eq!(why, "server told another scenery");
}

#[test]
fn a_match_start_on_another_map_than_the_scenery_told_is_refused() {
    let mut seat = Seat::watching(SessionOptions::default());
    seat.ingest(scenery(1, trees()), &mut ()).expect("passed");
    let why = refusal(
        &mut seat,
        ServerMsg::MatchStart {
            info: info(trees()),
        },
    );
    assert_eq!(why, "server told another scenery");
}

#[test]
fn scenery_told_twice_before_the_start_is_refused() {
    let mut seat = Seat::watching(SessionOptions::default());
    seat.ingest(scenery(2, trees()), &mut ()).expect("passed");
    assert_eq!(
        refusal(&mut seat, scenery(2, trees())),
        "server sent Scenery twice"
    );
}

#[test]
fn scenery_after_the_match_start_is_a_stream_error() {
    let mut seat = Seat::watching(SessionOptions::default());
    seat.ingest(
        ServerMsg::MatchStart {
            info: info(trees()),
        },
        &mut (),
    )
    .expect("started");
    assert_eq!(
        refusal(&mut seat, scenery(2, trees())),
        "server sent Scenery after MatchStart"
    );
}
