use bota_client_core::{AckPolicy, ClientSession, Ingested, MatchStatic, SessionOptions};
use bota_core::{Catalogue, MAP2_ID, map_of};
use bota_proto::{HeroId, MapId, Pick, ServerMsg, SlotId, Team, TickMode};
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
        MAP2_ID,
    )
}

fn opening_heroes(
    heroes: &[(Team, HeroId)],
    spawn_modifiers: Vec<bota_server::game::SpawnModifier>,
    map_id: MapId,
) -> (LocalMatch, Vec<Vec<ServerMsg>>) {
    opening_match(heroes, spawn_modifiers, map_id, false)
}

fn opening_match(
    heroes: &[(Team, HeroId)],
    spawn_modifiers: Vec<bota_server::game::SpawnModifier>,
    map_id: MapId,
    cheats: bool,
) -> (LocalMatch, Vec<Vec<ServerMsg>>) {
    let map = map_of(map_id);
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
        map: map_id,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 150,
        cheats,
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
    let (local, opening) = opening_heroes(heroes, modifiers, MAP2_ID);
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

/// The fuel the interpreter takes for the host calling a native that does nothing. The fuel tests pin the host work a
/// callback charges above this, so that a change of the interpreter's own prices does not move them.
#[allow(
    dead_code,
    reason = "only the tests that pin a callback's host charge need the baseline"
)]
pub fn bare_native_call_fuel() -> u64 {
    let limits = luagee::Limits::DEFAULT;
    let mut lua = luagee::Lua::new(
        limits,
        luagee::Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("vm");
    let function = lua
        .function("bare_native", std::rc::Rc::new(|_, _| Ok(0)))
        .expect("callback");
    lua.begin_tick();
    lua.call(function, &[]).expect("bare call");
    limits.fuel_per_tick - lua.tick_fuel_left()
}

#[allow(dead_code, reason = "shared real-order encounter fixture")]
pub fn durable_creep_encounter(
    hero: HeroId,
) -> (LocalMatch, Vec<ClientSession>, bota_proto::EntityId) {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Fixed, ModifierSpec, Order, Target};
    use bota_server::game::{
        ModifierDuration, SpawnCategory, SpawnModifier, SpawnSelector, SpawnTarget,
    };
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

#[allow(
    dead_code,
    reason = "shared by courier integration and worker continuation contracts"
)]
pub fn courier_delivery_setup() -> (LocalMatch, Vec<ClientSession>) {
    use bota_client_core::IssuedOrder;
    use bota_proto::{ItemId, Order, Target, Vec2};
    let (mut local, mut sessions) = local(&[Team::Radiant, Team::Dire]);
    let walk = sessions[0]
        .issue(IssuedOrder::hero(Order::Move {
            target: Target::Pos(Vec2::from_ints(9216, 9216)),
        }))
        .expect("walk");
    for tick in 0..250 {
        let first = [walk];
        let orders = if tick == 0 { &first[..] } else { &[] };
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[orders, &[]]).expect("walk step"))
        {
            fold(session, stream);
        }
    }
    let buy = sessions[0]
        .issue(IssuedOrder::hero(Order::Buy {
            item: ItemId::IRON_BRANCH,
        }))
        .expect("buy");
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[buy], &[]]).expect("stash purchase"))
    {
        fold(session, stream);
    }
    assert!(
        sessions[0].current().expect("view").players[0]
            .stash
            .as_ref()
            .expect("own stash")
            .iter()
            .flatten()
            .any(|item| item.id == ItemId::IRON_BRANCH)
    );
    (local, sessions)
}

#[allow(
    dead_code,
    reason = "shared by courier integration and worker continuation contracts"
)]
pub fn courier_modules(fail_level_once: bool) -> bota_lua::ModuleBank {
    let mut modules = bota_lua::ModuleBank::new(bota_lua::Limits::DEFAULT);
    modules
        .insert(
            "mode_laning_generic",
            b"function GetDesire() return 1 end function Think() end",
        )
        .expect("mode");
    modules
        .insert(
            "item_purchase_nevermore",
            b"function ItemPurchaseThink() end",
        )
        .expect("purchase");
    let usage = if fail_level_once {
        b"function AbilityUsageThink() end function ItemUsageThink() end function BuybackUsageThink() end function AbilityLevelUpThink() if not failed then failed=true error('one deferred turn') end end".as_slice()
    } else {
        b"function AbilityUsageThink() end function ItemUsageThink() end function BuybackUsageThink() end function AbilityLevelUpThink() end".as_slice()
    };
    modules
        .insert("ability_item_usage_nevermore", usage)
        .expect("usage");
    modules
}

#[allow(dead_code, reason = "shared fixture used by match metadata queries")]
pub fn local_on_map(teams: &[Team], map: MapId) -> (LocalMatch, Vec<ClientSession>) {
    let heroes: Vec<_> = teams
        .iter()
        .map(|&team| (team, HeroId::SHADOW_FIEND))
        .collect();
    let (local, opening) = opening_heroes(&heroes, Vec::new(), map);
    (
        local,
        sessions(opening, &vec![HeroId::SHADOW_FIEND; teams.len()]),
    )
}

#[allow(
    dead_code,
    reason = "shared fixture of scenarios reached by cheat orders"
)]
pub fn local_with_cheats(teams: &[Team], map: MapId) -> (LocalMatch, Vec<ClientSession>) {
    let heroes: Vec<_> = teams
        .iter()
        .map(|&team| (team, HeroId::SHADOW_FIEND))
        .collect();
    let (local, opening) = opening_match(&heroes, Vec::new(), map, true);
    (
        local,
        sessions(opening, &vec![HeroId::SHADOW_FIEND; teams.len()]),
    )
}
