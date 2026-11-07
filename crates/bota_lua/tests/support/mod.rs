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

#[allow(dead_code, reason = "shared real-order encounter fixture")]
pub fn durable_creep_encounter(hero: HeroId) -> (LocalMatch, Vec<ClientSession>, bota_proto::EntityId) {
    use bota_proto::{Fixed, ModifierSpec, Order, Target};
    use bota_client_core::IssuedOrder;
    use bota_server::game::{ModifierDuration, SpawnCategory, SpawnModifier, SpawnSelector, SpawnTarget};
    let modifiers = vec![SpawnModifier {
        select: SpawnSelector {
            team: None,
            targets: vec![
                SpawnTarget::Category(SpawnCategory::LaneCreep),
                SpawnTarget::Category(SpawnCategory::Hero),
            ],
        },
        spec: ModifierSpec {
            max_hp: 40_000,
            ..ModifierSpec::NOMINAL
        },
        duration: ModifierDuration::MatchLong,
    }];
    let (mut local, mut sessions) = local_with_modifiers(
        &[(Team::Radiant, hero), (Team::Dire, HeroId::SHADOW_FIEND)],
        modifiers,
    );
    let tower = sessions[0]
        .map()
        .radiant_towers
        .iter()
        .find(|&&(lane, tier, _)| lane == bota_core::rules::LANE_MID && tier == 1)
        .expect("tower")
        .2;
    let position = bota_core::move_towards(
        tower,
        bota_proto::Vec2::from_ints(9216, 9216),
        Fixed::from_int(800),
    );
    for _ in 0..sessions[0].map().pregame_ticks + 1200 {
        let order = sessions[0]
            .issue(IssuedOrder::hero(Order::Move {
                target: Target::Pos(position),
            }))
            .expect("move");
        let streams = local.step(&[&[order], &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            fold(session, stream);
        }
        let own = sessions[0].own_hero().expect("hero");
        let target = sessions[0]
            .current()
            .expect("view")
            .units
            .iter()
            .filter(|unit| {
                unit.team == Team::Dire
                    && bota_core::kind_def(unit.kind).creep
                    && unit.pos.within(own.pos, Fixed::from_int(1600))
            })
            .min_by_key(|unit| (unit.pos.distance_squared(own.pos), unit.id))
            .map(|unit| unit.id);
        if let Some(target) = target {
            return (local, sessions, target);
        }
    }
    panic!("no enemy creep reached the lane");
}
