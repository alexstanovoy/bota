//! A bot written against `bota_client_core`, playing a lockstep match
//! inside this process: two Shadow Fiends, each walking to the middle of the
//! map and fighting there, until the match ends.
//!
//! ```text
//! cargo run --release -p bota_server --example local_match
//! cargo run --release -p bota_server --example local_match -- --replay match.brp
//! ```
//!
//! With `--replay FILE` the match is also written as a replay, the one a
//! server records of the same match, for a viewer to open.

use bota_client_core::{
    AckPolicy, ClientSession, Ingested, IssuedOrder, MatchStatic, SessionOptions,
};
use bota_core::{MAP2_ID, map_of};
use bota_proto::{
    HeroId, Order, OutgoingOrder, Pick, ReplayRecord, ServerMsg, SlotId, Target, Team, TickMode,
    Vec2,
};
use bota_server::game::{MatchConfig, OrderDelay};
use bota_server::local::LocalMatch;
use bota_server::replay::ReplayWriter;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut replay = None;
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--replay", Some(path)) => replay = Some(path),
            _ => {
                eprintln!("usage: local_match [--replay FILE]");
                std::process::exit(2);
            }
        }
    }
    let pick = |slot, team| Pick {
        slot: SlotId(slot),
        team,
        hero: HeroId::SHADOW_FIEND,
    };
    let cfg = MatchConfig {
        starting_gold_lost: Vec::new(),
        match_id: 1,
        master_key: [7; 32],
        stream: 1,
        picks: vec![pick(0, Team::Radiant), pick(1, Team::Dire)],
        map: MAP2_ID,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 150,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
        objectives: bota_core::map_of(MAP2_ID).default_objectives,
        runes: bota_core::map_of(MAP2_ID).default_runes,
        catalogue: bota_core::Catalogue::FULL,
    };
    let map = map_of(cfg.map);
    let [a, b] = map.fountains;
    let middle = Vec2::from_ints(
        (a.x.to_int() + b.x.to_int()) / 2,
        (a.y.to_int() + b.y.to_int()) / 2,
    );
    let (mut local, opening) = if replay.is_some() {
        LocalMatch::new_recorded(&cfg, &[OrderDelay::NONE])
    } else {
        LocalMatch::new(&cfg, &[OrderDelay::NONE])
    }
    .expect("a valid match");
    let mut writer = replay.map(|path| {
        let mut writer = ReplayWriter::create(path.as_ref(), None).expect("a replay file to write");
        writer.record(&ReplayRecord::Msg(opening[0][0].clone()));
        writer
    });
    let mut sessions = sessions(&opening);
    let mut pending = opening;
    loop {
        // Fold what each seat was sent, then decide at most one order per seat:
        // the example's heroes need no more.
        let mut winner = None;
        let mut orders = Vec::new();
        for (session, stream) in sessions.iter_mut().zip(pending) {
            for message in stream {
                if let ServerMsg::MatchStart { .. } = message {
                    continue; // the session was built from it
                }
                if let Ingested::Over { winner: won, .. } =
                    session.ingest(message, &mut ()).expect("folds")
                {
                    winner = Some(won);
                }
            }
            let Some(view) = session.current() else {
                orders.push(None);
                continue;
            };
            // Walk to the middle of the map between the fountains, fighting what
            // is met on the way. The order stands until the hero dies, so it is
            // given again every twenty seconds: a hero that fell and came back
            // walks again.
            let order = IssuedOrder::hero(Order::Attack {
                target: Target::Pos(middle),
            });
            let due = view.tick <= 1 || view.tick % 600 == 1;
            let sent = due.then(|| session.issue(order).expect("a snapshot was seen"));
            orders.push(sent);
        }
        if let Some(writer) = writer.as_mut() {
            local
                .take_records()
                .iter()
                .for_each(|record| writer.record(record));
        }
        if let Some(winner) = winner {
            println!("tick {}: match over, winner {winner:?}", local.tick());
            writer.into_iter().for_each(|writer| {
                writer.finish();
            });
            return;
        }
        let given: Vec<&[OutgoingOrder]> = orders.iter().map(Option::as_slice).collect();
        pending = local.step(&given).expect("a step");
    }
}

/// One session per seat, built from its stream's `MatchStart`, to fold what
/// that seat is sent, in order.
fn sessions(opening: &[Vec<ServerMsg>]) -> Vec<ClientSession> {
    let mut sessions = Vec::new();
    for (slot, stream) in opening.iter().enumerate() {
        let ServerMsg::MatchStart { info } = &stream[0] else {
            unreachable!("a stream starts with MatchStart");
        };
        // The caller steps the match itself, so nothing is acknowledged.
        let options = SessionOptions {
            ack: AckPolicy::Never,
            ..SessionOptions::default()
        };
        let statics = MatchStatic::new(info).expect("valid terms");
        let session = ClientSession::seat(
            statics,
            SlotId(slot as u8),
            Some(HeroId::SHADOW_FIEND),
            options,
        )
        .expect("a seat");
        sessions.push(session);
    }
    sessions
}
