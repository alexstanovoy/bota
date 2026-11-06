use bota_client_core::{AckPolicy, ClientSession, Ingested, MatchStatic, SessionOptions};
use bota_core::{Catalogue, MAP2_ID, map_of};
use bota_proto::{HeroId, Pick, ServerMsg, SlotId, Team, TickMode};
use bota_server::{game::{MatchConfig, OrderDelay}, local::LocalMatch};

pub fn opening(teams: &[Team]) -> (LocalMatch, Vec<Vec<ServerMsg>>) {
    let map = map_of(MAP2_ID);
    let config = MatchConfig {
        match_id: 7, master_key: [7; 32], stream: 1,
        picks: teams.iter().enumerate().map(|(slot, &team)| Pick { slot: SlotId(slot as u8), team, hero: HeroId::SHADOW_FIEND }).collect(),
        map: MAP2_ID, mode: TickMode::Lockstep, ack_timeout_ticks: 150, cheats: false,
        spawn_modifiers: Vec::new(), immortal_structures: false, immortal_heroes: false,
        starting_gold_lost: Vec::new(), objectives: map.default_objectives,
        runes: map.default_runes, catalogue: Catalogue::FULL, economy_events: Vec::new(),
    };
    let (local, opening) = LocalMatch::new(&config, &[OrderDelay::NONE]).expect("match");
    (local, opening)
}

#[allow(dead_code, reason = "shared fixture also used by the transport driver test")]
pub fn local(teams: &[Team]) -> (LocalMatch, Vec<ClientSession>) {
    let (local, opening) = opening(teams);
    let sessions = opening.into_iter().enumerate().map(|(slot, stream)| {
        let ServerMsg::MatchStart { info } = &stream[0] else { panic!("start") };
        let mut session = ClientSession::seat(MatchStatic::new(info).expect("statics"), SlotId(slot as u8), Some(HeroId::SHADOW_FIEND), SessionOptions { ack: AckPolicy::Never, ..SessionOptions::default() }).expect("session");
        fold(&mut session, stream.into_iter().skip(1));
        session
    }).collect();
    (local, sessions)
}

pub fn fold(session: &mut ClientSession, stream: impl IntoIterator<Item = ServerMsg>) -> Option<Team> {
    let mut won = None;
    for message in stream {
        if let Ingested::Over { winner, .. } = session.ingest(message, &mut ()).expect("stream") {
            won = Some(winner);
        }
    }
    won
}
