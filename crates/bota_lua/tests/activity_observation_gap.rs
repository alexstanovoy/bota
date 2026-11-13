mod support;
use bota_client_core::{ClientSession, IssuedOrder};
use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ModuleBank, ScriptKind};
use bota_proto::{Cheat, HeroId, Order, SlotId, Target, Team, UnitActionPhase, Vec2};
use bota_server::{
    game::{ActionPhase, ActionState, Entity},
    local::LocalMatch,
};

fn team(index: usize) -> Team {
    if index == 0 {
        Team::Radiant
    } else {
        Team::Dire
    }
}
fn at(observer: usize, x: i32, y: i32) -> Vec2 {
    if observer == 0 {
        Vec2::from_ints(x, y)
    } else {
        Vec2::from_ints(
            bota_core::rules::MAP_SIZE - x - 3000,
            bota_core::rules::MAP_SIZE - y - 3000,
        )
    }
}
fn step(
    local: &mut LocalMatch,
    sessions: &mut [ClientSession],
    commands: &[&[bota_proto::OutgoingOrder]],
) {
    for (session, stream) in sessions.iter_mut().zip(local.step(commands).expect("step")) {
        support::fold(session, stream);
    }
}
fn hidden_attack(observer: usize) -> (LocalMatch, Vec<ClientSession>, Entity, Entity) {
    let actor = 1 - observer;
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let (neutral, alternative);
    {
        let world = local.world_mut();
        world.cheats = true;
        world.cheat(SlotId(0), None, Cheat::StartGame, &mut Vec::new());
        world.cheat(
            SlotId(0),
            None,
            Cheat::AllVision { on: true },
            &mut Vec::new(),
        );
        neutral = world.spawn_neutral(
            bota_core::NeutralKind::Kobold,
            at(observer, 9450, 9450),
            0,
            0,
        );
        alternative = world.spawn_neutral(
            bota_core::NeutralKind::Kobold,
            at(observer, 9500, 9500),
            0,
            0,
        );
        for (index, position) in [
            (observer, at(observer, 8000, 8000)),
            (actor, at(observer, 9100, 9100)),
        ] {
            let hero = world.seats[index].unit.expect("hero");
            world.cheat(
                SlotId(index as u8),
                Some(hero),
                Cheat::Teleport { at: position },
                &mut Vec::new(),
            );
        }
    }
    step(&mut local, &mut sessions, &[&[], &[]]);
    let neutral_id = sessions[actor]
        .current()
        .expect("actor view")
        .units
        .iter()
        .filter(|unit| unit.neutral == Some(bota_core::NeutralKind::Kobold.id()))
        .min_by_key(|unit| {
            let x = i64::from(unit.pos.x.raw) - i64::from(at(observer, 9450, 9450).x.raw);
            let y = i64::from(unit.pos.y.raw) - i64::from(at(observer, 9450, 9450).y.raw);
            x * x + y * y
        })
        .expect("visible neutral target")
        .id;
    let stop = sessions[observer]
        .issue(IssuedOrder::hero(Order::Move {
            target: Target::None,
        }))
        .expect("stop");
    let attack = sessions[actor]
        .issue(IssuedOrder::hero(Order::Attack {
            target: Target::Unit(neutral_id),
        }))
        .expect("attack");
    local.world_mut().cheat(
        SlotId(0),
        None,
        Cheat::AllVision { on: false },
        &mut Vec::new(),
    );
    let mut commands = [Vec::new(), Vec::new()];
    commands[observer].push(stop);
    commands[actor].push(attack);
    step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
    for _ in 0..24 {
        let body = local.world().seats[actor].unit.expect("actor");
        if matches!(local.world().action.get(body).map(|a|a.state),Some(ActionState::Attack{target,phase:ActionPhase::Before{..}}) if target==neutral)
        {
            assert!(
                local.world().shows(team(observer), body),
                "observer sees the attacker"
            );
            assert!(
                !local.world().shows(team(observer), neutral),
                "observer cannot see attacked neutral"
            );
            assert!(
                !local.world().shows(team(observer), alternative),
                "observer cannot see alternative neutral"
            );
            return (local, sessions, neutral, alternative);
        }
        step(&mut local, &mut sessions, &[&[], &[]]);
    }
    panic!("ordinary attack reaches a wind-up");
}
#[test]
fn a_visible_attack_phase_does_not_identify_which_hidden_neutral_is_attacked() {
    for observer in [0, 1] {
        let actor = 1 - observer;
        let (mut local, sessions, neutral, alternative) = hidden_attack(observer);
        let world = local.world();
        let body = world.seats[actor].unit.expect("body");
        let attacker = sessions[observer]
            .current()
            .expect("observer view")
            .units
            .iter()
            .find(|unit| unit.hero.is_some() && unit.team == team(actor))
            .expect("visible enemy body")
            .id;
        assert!(
            matches!(world.action.get(body).expect("action").state,ActionState::Attack{target,phase:ActionPhase::Before{..}} if target==neutral)
        );
        let view = sessions[observer].current().expect("observer view");
        assert!(view.unit(attacker).is_some(), "attacker remains visible");
        let raw_view = world.view(team(observer));
        assert!(
            raw_view.unit(bota_server::game::wire_id(neutral)).is_none(),
            "the attacked neutral is hidden: observer={observer}, actor={:?}, target={:?}, observer_body={:?}",
            world.transform.get(body).map(|t| t.pos),
            world.transform.get(neutral).map(|t| t.pos),
            sessions[observer].own_hero().expect("observer").pos
        );
        assert!(
            raw_view
                .unit(bota_server::game::wire_id(alternative))
                .is_none(),
            "the alternative neutral is hidden"
        );
        assert!(
            view.aggro
                .iter()
                .find(|row| row.unit == attacker)
                .is_none_or(|row| row.swing.is_none()),
            "target identity stays withheld"
        );
        let phase = view
            .unit(attacker)
            .expect("attacker")
            .action_phase
            .expect("visible phase");
        assert_eq!(phase.phase, UnitActionPhase::AttackWindup);
        assert!(phase.elapsed < u32::MAX);
        let before = world.hash();
        let full_before = world.view_full();
        assert_eq!(
            world.hash(),
            before,
            "projection never changes simulation state"
        );
        let mut changed_match = local.clone();
        let frame: Vec<u8> = local.restate()[observer]
            .iter()
            .flat_map(|message| {
                bota_proto::encode_frame_to_vec(message).expect("encode complete observer stream")
            })
            .collect();
        let world = changed_match.world_mut();
        let ActionState::Attack { target, .. } =
            &mut world.action.get_mut(body).expect("action").state
        else {
            panic!("attack");
        };
        *target = alternative;
        assert_ne!(
            world.hash(),
            before,
            "only the hidden attacked identity changes"
        );
        assert_ne!(
            world.view_full(),
            full_before,
            "omniscient stream distinguishes targets"
        );
        let after: Vec<u8> = changed_match.restate()[observer]
            .iter()
            .flat_map(|message| {
                bota_proto::encode_frame_to_vec(message)
                    .expect("encode changed hidden target stream")
            })
            .collect();
        assert_eq!(
            frame, after,
            "the complete observer snapshot stream must stay identical"
        );
    }
}
fn reader(observer: usize) -> LuaTeam {
    let actor = 1 - observer;
    let slot = SlotId(observer as u8);
    let mut reader = LuaTeam::new(
        team(observer),
        &[BotSeat {
            slot,
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("reader");
    reader.set_gc_stress(GcStress::Full);
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    modules
        .set_activity_profile(
            "npc_dota_hero_nevermore",
            bota_client_core::ActivityProfile {
                idle_ms: Some(1000),
                ..Default::default()
            },
        )
        .expect("fixture profile");
    reader.set_modules(modules).expect("modules");
    let player = if actor == 0 { 0 } else { 5 };
    let source = format!(
        r#"
        local calls,actor,previous=0,nil,nil
        function Think()
            calls=calls+1
            if actor==nil then
                for _,u in ipairs(GetUnitList(UNIT_LIST_ENEMY_HEROES)) do
                    if u:GetPlayerID()=={player} then actor=u end
                end
            end
            assert(actor)
            assert(not pcall(actor.GetAnimActivity,{{}}))
            assert(not pcall(actor.GetAnimCycle,{{}}))
            local activity,cycle=actor:GetAnimActivity(),actor:GetAnimCycle()
            if calls<3 then
                assert(activity==ACTIVITY_ATTACK,'a visible hidden-target wind-up must be an attack')
                assert(cycle>=0 and cycle<1)
                if previous then assert(cycle>previous,'the observed wind-up cycle advances') end
                previous=cycle
            elseif calls==3 then assert(activity==-1 and cycle==-1.0) else assert(activity==ACTIVITY_IDLE and cycle>=0 and cycle<1) end
            GetBot():Action_MoveToLocation(Vector(cycle*100,0,0))
        end
    "#
    );
    reader
        .add_script(
            slot,
            ScriptKind::Takeover,
            "hidden_target_activity",
            source.as_bytes(),
        )
        .expect("script");
    reader
}
fn think(reader: &mut LuaTeam, session: &ClientSession) {
    let turn = reader.think(session).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(
        turn.bots[0].orders.len(),
        1,
        "queries keep their derived order"
    );
}
#[test]
fn both_animation_getters_observe_a_hidden_target_windup_and_return_sentinels_after_sight_loss() {
    for observer in [0, 1] {
        let actor = 1 - observer;
        let (mut local, mut sessions, _, _) = hidden_attack(observer);
        let attacker = sessions[observer]
            .current()
            .expect("observer view")
            .units
            .iter()
            .find(|unit| unit.hero.is_some() && unit.team == team(actor))
            .expect("visible attacker")
            .id;
        let mut reader = reader(observer);
        let first = sessions[observer]
            .current()
            .expect("view")
            .unit(attacker)
            .expect("seen")
            .action_phase
            .expect("phase");
        think(&mut reader, &sessions[observer]);
        let mut advanced = false;
        for _ in 0..12 {
            step(&mut local, &mut sessions, &[&[], &[]]);
            let phase = sessions[observer]
                .current()
                .expect("view")
                .unit(attacker)
                .expect("seen")
                .action_phase
                .expect("phase");
            if phase.phase == UnitActionPhase::AttackWindup && phase.elapsed > first.elapsed {
                advanced = true;
                break;
            }
        }
        assert!(
            advanced,
            "real server wind-up progresses while its target is hidden"
        );
        think(&mut reader, &sessions[observer]);
        let observed_position = local
            .world()
            .transform
            .get(local.world().seats[actor].unit.expect("actor"))
            .expect("position")
            .pos;
        {
            let world = local.world_mut();
            let hero = world.seats[actor].unit.expect("actor");
            let at = bota_core::fountain_pos(bota_core::map_of(bota_core::MAP2_ID), team(actor));
            world.cheat(
                SlotId(actor as u8),
                Some(hero),
                Cheat::Teleport { at },
                &mut Vec::new(),
            );
        }
        step(&mut local, &mut sessions, &[&[], &[]]);
        assert!(
            sessions[observer]
                .current()
                .expect("view")
                .unit(attacker)
                .is_none(),
            "hidden body and phase absent"
        );
        think(&mut reader, &sessions[observer]);
        let _frozen = sessions[observer]
            .entity(attacker)
            .expect("remembered actor")
            .activity;
        {
            let world = local.world_mut();
            let body = world.seats[actor].unit.expect("actor");
            world.cheat(
                SlotId(actor as u8),
                Some(body),
                Cheat::Teleport {
                    at: observed_position,
                },
                &mut Vec::new(),
            );
        }
        let streams = local.step(&[&[], &[]]).expect("return to sight");
        let mut omitted = false;
        for (index, (session, mut stream)) in sessions.iter_mut().zip(streams).enumerate() {
            if index == observer {
                for message in &mut stream {
                    if let bota_proto::ServerMsg::Snapshot { view } = message {
                        for unit in &mut view.units {
                            if unit.hero.is_some() && unit.team == team(actor) {
                                unit.action_phase = None;
                                omitted = true;
                            }
                        }
                    }
                }
            }
            support::fold(session, stream);
        }
        assert!(
            omitted,
            "exercise a carried body without a phase observation"
        );
        think(&mut reader, &sessions[observer]);
    }
}
