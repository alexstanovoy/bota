//! A bot written against `bota_client_core`, playing a lockstep match
//! inside this process: two Shadow Fiends, each walking to the other's
//! fountain, until the match ends.
//!
//! ```text
//! cargo run --release -p bota_server --example local_match
//! ```

use bota_client_core::{
    AckPolicy, ClientSession, Ingested, IssuedOrder, MatchStatic, SessionOptions,
};
use bota_core::{MAP2_ID, map_of};
use bota_proto::{HeroId, Order, OutgoingOrder, Pick, ServerMsg, SlotId, Target, Team, TickMode};
use bota_server::game::{MatchConfig, OrderDelay};
use bota_server::local::LocalMatch;

fn main() {
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
        tick_rate: 30,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 150,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
        objectives: bota_core::map_of(MAP2_ID).default_objectives,
        runes: bota_core::map_of(MAP2_ID).default_runes,
    };
    let map = map_of(cfg.map);
    let (mut local, opening) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a valid match");
    let mut sessions = sessions(&opening);
    let mut pending = opening;
    loop {
        // Fold what each seat was sent, then decide at most one order per seat:
        // the example's heroes need no more.
        let mut winner = None;
        let mut orders = Vec::new();
        for (slot, (session, stream)) in sessions.iter_mut().zip(pending).enumerate() {
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
            // Walk to the enemy fountain, fighting what is met on the way;
            // the first tick is enough, the order stands.
            let order = IssuedOrder::hero(Order::Attack {
                target: Target::Pos(map.fountains[1 - slot]),
            });
            let first = view.tick <= 1;
            let sent = first.then(|| session.issue(order).expect("a snapshot was seen"));
            orders.push(sent);
        }
        if let Some(winner) = winner {
            println!("tick {}: match over, winner {winner:?}", local.tick());
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
