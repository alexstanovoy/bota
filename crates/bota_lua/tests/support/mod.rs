use bota_client_core::{AckPolicy, ClientSession, Ingested, MatchStatic, SessionOptions};
use bota_core::{Catalogue, MAP2_ID, map_of};
use bota_proto::{HeroId, Pick, ServerMsg, SlotId, Team, TickMode};
use bota_server::{
    game::{MatchConfig, OrderDelay},
    local::LocalMatch,
};

pub fn opening(teams: &[Team]) -> (LocalMatch, Vec<Vec<ServerMsg>>) {
    opening_heroes(
        &teams
            .iter()
            .map(|&team| (team, HeroId::SHADOW_FIEND))
            .collect::<Vec<_>>(),
        Vec::new(),
    )
}

fn opening_heroes(
    heroes: &[(Team, HeroId)],
    spawn_modifiers: Vec<bota_server::game::SpawnModifier>,
) -> (LocalMatch, Vec<Vec<ServerMsg>>) {
    let map = map_of(MAP2_ID);
    let config = MatchConfig {
        match_id: 7,
        master_key: [7; 32],
        stream: 1,
        picks: heroes
            .iter()
            .enumerate()
            .map(|(slot, &(team, hero))| Pick {
                slot: SlotId(slot as u8),
                team,
                hero,
            })
            .collect(),
        map: MAP2_ID,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 150,
        cheats: false,
        spawn_modifiers,
        immortal_structures: false,
        immortal_heroes: false,
        starting_gold_lost: Vec::new(),
        objectives: map.default_objectives,
        runes: map.default_runes,
        catalogue: Catalogue::FULL,
        economy_events: Vec::new(),
    };
    let (local, opening) = LocalMatch::new(&config, &[OrderDelay::NONE]).expect("match");
    (local, opening)
}

#[allow(
    dead_code,
    reason = "shared fixture also used by the transport driver test"
)]
pub fn local(teams: &[Team]) -> (LocalMatch, Vec<ClientSession>) {
    let (local, opening) = opening(teams);
    (
        local,
        sessions(opening, &vec![HeroId::SHADOW_FIEND; teams.len()]),
    )
}

#[allow(
    dead_code,
    reason = "only the melee API contracts need a different hero"
)]
pub fn local_with_modifiers(
    heroes: &[(Team, HeroId)],
    modifiers: Vec<bota_server::game::SpawnModifier>,
) -> (LocalMatch, Vec<ClientSession>) {
    let (local, opening) = opening_heroes(heroes, modifiers);
    (
        local,
        sessions(
            opening,
            &heroes.iter().map(|&(_, hero)| hero).collect::<Vec<_>>(),
        ),
    )
}

fn sessions(opening: Vec<Vec<ServerMsg>>, heroes: &[HeroId]) -> Vec<ClientSession> {
    opening
        .into_iter()
        .enumerate()
        .map(|(slot, stream)| {
            let ServerMsg::MatchStart { info } = &stream[0] else {
                panic!("start")
            };
            let mut session = ClientSession::seat(
                MatchStatic::new(info).expect("statics"),
                SlotId(slot as u8),
                Some(heroes[slot]),
                SessionOptions {
                    ack: AckPolicy::Never,
                    ..SessionOptions::default()
                },
            )
            .expect("session");
            fold(&mut session, stream.into_iter().skip(1));
            session
        })
        .collect()
}

pub fn fold(
    session: &mut ClientSession,
    stream: impl IntoIterator<Item = ServerMsg>,
) -> Option<Team> {
    let mut won = None;
    for message in stream {
        if let Ingested::Over { winner, .. } = session.ingest(message, &mut ()).expect("stream") {
            won = Some(winner);
        }
    }
    won
}
