mod support;
use bota_client_core::{ClientSession, DamageSource, IssuedOrder};
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Cheat, EntityId, HeroId, Order, SlotId, Target, Team, Vec2};
use bota_server::local::LocalMatch;
fn advance(
    local: &mut LocalMatch,
    sessions: &mut [ClientSession],
    commands: &[Vec<bota_proto::OutgoingOrder>; 2],
) {
    let streams = local.step(&[&commands[0], &commands[1]]).unwrap();
    for (session, stream) in sessions.iter_mut().zip(streams) {
        assert!(
            !stream
                .iter()
                .any(|message| matches!(message, bota_proto::ServerMsg::OrderRejected { .. })),
            "orders accepted: {stream:?}"
        );
        support::fold(session, stream);
    }
}
fn check_own(session: &ClientSession, source: &[u8]) {
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .unwrap();
    lua.add_script(SlotId(0), ScriptKind::Takeover, "observed_damage", source)
        .unwrap();
    let turn = lua.think(session).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}
fn creep_encounter() -> (LocalMatch, Vec<ClientSession>) {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let line = bota_core::lane_polyline(sessions[0].map(), bota_core::rules::LANE_MID);
    let at = line[line.len() / 2];
    let mut commands = [Vec::new(), Vec::new()];
    for order in [
        Order::Cheat {
            cheat: Cheat::StartGame,
        },
        Order::Cheat {
            cheat: Cheat::Teleport { at },
        },
        Order::Cheat {
            cheat: Cheat::ClearModifiers {
                target: Target::None,
            },
        },
        Order::Move {
            target: Target::Pos(at),
        },
        Order::Cheat {
            cheat: Cheat::CreateCreeps {
                enemy: true,
                at: at + Vec2::from_ints(80, 0),
            },
        },
    ] {
        commands[0].push(sessions[0].issue(IssuedOrder::hero(order)).unwrap());
    }
    advance(&mut local, &mut sessions, &commands);
    (local, sessions)
}
#[test]
fn creep_damage_reaches_the_shared_history_and_the_lua_query_through_real_orders() {
    let (mut local, mut sessions) = creep_encounter();
    let own = sessions[0].own_hero().unwrap().id;
    check_own(&sessions[0],b"function Think() assert(GetBot():WasRecentlyDamagedByCreep(0)==false); assert(type(GetBot():WasRecentlyDamagedByCreep(10))=='boolean') end");
    for _ in 0..180 {
        if sessions[0]
            .entity(own)
            .unwrap()
            .recent_damage
            .last(DamageSource::Creep)
            .is_some()
        {
            break;
        }
        advance(&mut local, &mut sessions, &[Vec::new(), Vec::new()]);
    }
    let track = sessions[0].entity(own).unwrap();
    let last = track
        .recent_damage
        .last(DamageSource::Creep)
        .expect("creeps dealt observed damage");
    assert!(track.recent_damage.within(last, 1, DamageSource::Creep));
    check_own(&sessions[0],b"function Think() assert(GetBot():WasRecentlyDamagedByCreep(10)); assert(not GetBot():WasRecentlyDamagedByHero(GetBot(),10)) end");
}
fn hidden_caster(observer: usize) -> (LocalMatch, Vec<ClientSession>, EntityId, usize) {
    let enemy = 1 - observer;
    let heroes = if observer == 0 {
        [
            (Team::Radiant, HeroId::SHADOW_FIEND),
            (Team::Dire, HeroId::PUDGE),
        ]
    } else {
        [
            (Team::Radiant, HeroId::PUDGE),
            (Team::Dire, HeroId::SHADOW_FIEND),
        ]
    };
    let (mut local, mut sessions) = support::local_for_heroes_with_cheats(&heroes);
    let id = local.hero_of(enemy).unwrap();
    let at = bota_core::move_towards(
        sessions[enemy].map().fountains[enemy],
        Vec2::from_ints(9216, 9216),
        bota_proto::Fixed::from_int(2000),
    );
    for cheat in [
        Cheat::StartGame,
        Cheat::Levels { count: 5 },
        Cheat::Refresh,
        Cheat::Teleport { at },
        Cheat::ClearModifiers {
            target: Target::None,
        },
        Cheat::AllVision { on: true },
    ] {
        local
            .world_mut()
            .cheat(SlotId(enemy as u8), Some(id), cheat, &mut Vec::new());
    }
    advance(&mut local, &mut sessions, &[Vec::new(), Vec::new()]);
    local.world_mut().cheat(
        SlotId(enemy as u8),
        Some(id),
        Cheat::AllVision { on: false },
        &mut Vec::new(),
    );
    advance(&mut local, &mut sessions, &[Vec::new(), Vec::new()]);
    let enemy_id = sessions[enemy].own_hero().unwrap().id;
    assert!(
        !sessions[observer]
            .current()
            .unwrap()
            .units
            .iter()
            .any(|unit| unit.id == enemy_id)
    );
    let slot = sessions[enemy]
        .own_hero()
        .unwrap()
        .abilities
        .iter()
        .position(|ability| ability.id == bota_proto::AbilityId::ROT)
        .unwrap();
    (local, sessions, enemy_id, slot)
}
fn fork_hidden_damage(observer: usize) -> (Vec<ClientSession>, Vec<ClientSession>, EntityId) {
    let enemy = 1 - observer;
    let (local, sessions, enemy_id, slot) = hidden_caster(observer);
    let mut a = local.clone();
    let mut b = local;
    let mut sa = sessions.clone();
    let mut sb = sessions;
    let mut commands = [Vec::new(), Vec::new()];
    commands[enemy].push(
        sb[enemy]
            .issue(IssuedOrder::hero(Order::Learn {
                slot: bota_proto::AbilitySlot(slot as u8),
            }))
            .unwrap(),
    );
    commands[enemy].push(
        sb[enemy]
            .issue(IssuedOrder::hero(Order::Cast {
                slot: bota_proto::AbilitySlot(slot as u8),
                target: Target::None,
            }))
            .unwrap(),
    );
    advance(&mut a, &mut sa, &[Vec::new(), Vec::new()]);
    advance(&mut b, &mut sb, &commands);
    for _ in 0..60 {
        advance(&mut a, &mut sa, &[Vec::new(), Vec::new()]);
        advance(&mut b, &mut sb, &[Vec::new(), Vec::new()]);
    }
    (sa, sb, enemy_id)
}
#[test]
fn damage_only_the_other_seat_observes_cannot_change_shared_damage_memory() {
    for observer in 0..2 {
        let enemy = 1 - observer;
        let (sa, sb, enemy_id) = fork_hidden_damage(observer);
        assert_ne!(
            sa[enemy].entity(enemy_id).unwrap().last_damage_taken,
            sb[enemy].entity(enemy_id).unwrap().last_damage_taken,
            "the hidden self-damage really occurred"
        );
        let ha = &sa[observer].entity(enemy_id).unwrap().recent_damage;
        let hb = &sb[observer].entity(enemy_id).unwrap().recent_damage;
        assert_eq!(
            ha, hb,
            "shared memory depends only on the receiving seat stream"
        );
        assert_eq!(
            sa[observer].current().unwrap().units,
            sb[observer].current().unwrap().units
        );
    }
}
