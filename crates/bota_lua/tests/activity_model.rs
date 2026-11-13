mod support;
use bota_client_core::{Activity, ActivityPhase, ActivityProfile, IssuedOrder};
use bota_proto::{AbilitySlot, Cheat, Order, SlotId, Target, Team, Vec2};

fn step(
    local: &mut bota_server::local::LocalMatch,
    sessions: &mut [bota_client_core::ClientSession],
    commands: &[&[bota_proto::OutgoingOrder]],
) {
    for (session, stream) in sessions.iter_mut().zip(local.step(commands).expect("step")) {
        support::fold(session, stream);
    }
}
fn profile() -> ActivityProfile {
    ActivityProfile {
        idle_ms: Some(1000),
        run_ms: Some(1000),
        flail_ms: Some(1000),
        stunned_ms: Some(1000),
        channel_ms: [Some(1000); 6],
        alternate_attacks: false,
        ..ActivityProfile::default()
    }
}
// This fixture supplies its own one-second clips; no installed content is needed.
fn query(
    session: &bota_client_core::ClientSession,
    observer: usize,
    actor: usize,
    expected: i32,
) -> i32 {
    let side = if observer == 0 {
        Team::Radiant
    } else {
        Team::Dire
    };
    let slot = SlotId(observer as u8);
    let mut team = bota_lua::LuaTeam::new(
        side,
        &[bota_lua::BotSeat {
            slot,
            hero: bota_proto::HeroId::SHADOW_FIEND,
        }],
        bota_lua::Limits::DEFAULT,
        bota_lua::Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("reader");
    team.set_gc_stress(bota_lua::GcStress::Full);
    let mut modules = bota_lua::ModuleBank::new(bota_lua::Limits::DEFAULT);
    modules
        .set_activity_profile("npc_dota_hero_nevermore", profile())
        .expect("profile");
    team.set_modules(modules).expect("modules");
    let player = if actor == 0 { 0 } else { 5 };
    let source = format!(
        r#"
        function Think()
            local actor=nil
            for _,u in ipairs(GetUnitList(UNIT_LIST_ALL)) do
                if u:IsHero() and u:GetPlayerID()=={player} then actor=u end
            end
            assert(actor)
            assert(not pcall(actor.GetAnimActivity,{{}}))
            assert(not pcall(actor.GetAnimCycle,{{}}))
            assert(actor:GetAnimActivity()=={expected}, "expected {expected}, got "..actor:GetAnimActivity())
            local cycle=actor:GetAnimCycle()
            if {expected}==-1 then assert(cycle==-1) else assert(cycle>=0 and cycle<1) end
            GetBot():Action_MoveToLocation(Vector(1000+cycle*1000,1000,0))
        end
    "#
    );
    team.add_script(
        slot,
        bota_lua::ScriptKind::Takeover,
        "activity_query",
        source.as_bytes(),
    )
    .expect("script");
    let turn = team.think(session).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
    let Order::Move {
        target: Target::Pos(at),
    } = turn.bots[0].orders[0].order
    else {
        panic!("query-derived move");
    };
    at.x.raw
}
fn fog_reader(side: Team, slot: SlotId, actor: usize) -> bota_lua::LuaTeam {
    let mut team = bota_lua::LuaTeam::new(
        side,
        &[bota_lua::BotSeat {
            slot,
            hero: bota_proto::HeroId::SHADOW_FIEND,
        }],
        bota_lua::Limits::DEFAULT,
        bota_lua::Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("fog reader");
    team.set_gc_stress(bota_lua::GcStress::Full);
    let mut modules = bota_lua::ModuleBank::new(bota_lua::Limits::DEFAULT);
    modules
        .set_activity_profile("npc_dota_hero_nevermore", profile())
        .expect("profile");
    team.set_modules(modules).expect("modules");
    let player = if actor == 0 { 0 } else { 5 };
    let source = format!(
        r#"
        local actor=nil
        function Think()
            if actor==nil then
                for _,u in ipairs(GetUnitList(UNIT_LIST_ALL)) do
                    if u:IsHero() and u:GetPlayerID()=={player} then actor=u end
                end
                assert(actor and actor:GetAnimActivity()>=1500)
            else
                assert(actor:GetAnimActivity()==-1)
                assert(actor:GetAnimCycle()==-1.0)
            end
        end
    "#
    );
    team.add_script(
        slot,
        bota_lua::ScriptKind::Takeover,
        "activity_fog",
        source.as_bytes(),
    )
    .expect("script");
    team
}
#[test]
fn observed_activity_tracks_real_idle_cast_and_attack_without_advancing_in_fog() {
    for observer in [0, 1] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let actor = 1 - observer;
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
            for (index, at) in [
                (actor, Vec2::from_ints(9000, 9000)),
                (observer, Vec2::from_ints(9100, 9000)),
            ] {
                let unit = world.seats[index].unit.expect("hero");
                world.cheat(
                    SlotId(index as u8),
                    Some(unit),
                    Cheat::Teleport { at },
                    &mut Vec::new(),
                );
            }
        }
        step(&mut local, &mut sessions, &[&[], &[]]);
        let stops: Vec<_> = sessions
            .iter_mut()
            .map(|session| {
                session
                    .issue(IssuedOrder::hero(Order::Move {
                        target: Target::None,
                    }))
                    .expect("stop")
            })
            .collect();
        step(&mut local, &mut sessions, &[&[stops[0]], &[stops[1]]]);
        let actor_id = sessions[actor].own_hero().expect("actor").id;
        let target = sessions[observer].own_hero().expect("target").id;
        let idle = sessions[observer]
            .entity(actor_id)
            .expect("seen actor")
            .activity
            .sample(profile())
            .expect("activity");
        assert_eq!(idle.activity, Activity::Idle);
        let before = idle.cycle.expect("known loop");
        let idle_query = query(&sessions[observer], observer, actor, 1500);
        for _ in 0..5 {
            step(&mut local, &mut sessions, &[&[], &[]]);
        }
        let idle = sessions[observer]
            .entity(actor_id)
            .expect("seen actor")
            .activity
            .sample(profile())
            .expect("activity");
        assert_eq!(idle.activity, Activity::Idle);
        assert!(idle.cycle.expect("cycle") > before);
        assert!(query(&sessions[observer], observer, actor, 1500) > idle_query);
        let learn = sessions[actor]
            .issue(IssuedOrder::hero(Order::Learn {
                slot: AbilitySlot(0),
            }))
            .expect("learn");
        let mut commands = [Vec::new(), Vec::new()];
        commands[actor].push(learn);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        let cast = sessions[actor]
            .issue(IssuedOrder::hero(Order::Cast {
                slot: AbilitySlot(0),
                target: Target::None,
            }))
            .expect("cast");
        let mut commands = [Vec::new(), Vec::new()];
        commands[actor].push(cast);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        let cast = sessions[observer]
            .entity(actor_id)
            .expect("seen actor")
            .activity
            .sample(profile())
            .expect("activity");
        assert!(matches!(
            cast.activity,
            Activity::Cast {
                slot: Some(AbilitySlot(0)),
                ..
            }
        ));
        assert_eq!(cast.phase, ActivityPhase::Windup);
        let before = cast.cycle.expect("cast cycle");
        let cast_query = query(&sessions[observer], observer, actor, 1510);
        step(&mut local, &mut sessions, &[&[], &[]]);
        let cast = sessions[observer]
            .entity(actor_id)
            .expect("seen actor")
            .activity
            .sample(profile())
            .expect("activity");
        assert!(cast.cycle.expect("cast cycle") > before);
        assert!(query(&sessions[observer], observer, actor, 1510) > cast_query);
        let attack = sessions[actor]
            .issue(IssuedOrder::hero(Order::Attack {
                target: Target::Unit(target),
            }))
            .expect("attack");
        let mut commands = [Vec::new(), Vec::new()];
        commands[actor].push(attack);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        let mut winding = None;
        for _ in 0..90 {
            let seen = sessions[observer]
                .entity(actor_id)
                .expect("actor")
                .activity
                .sample(profile())
                .expect("activity");
            if matches!(seen.activity, Activity::Attack { .. })
                && seen.phase == ActivityPhase::Windup
            {
                winding = Some(seen);
                break;
            }
            step(&mut local, &mut sessions, &[&[], &[]]);
        }
        let winding = winding.expect("visible windup");
        let attack_query = query(&sessions[observer], observer, actor, 1503);
        let mut advanced = false;
        for _ in 0..12 {
            step(&mut local, &mut sessions, &[&[], &[]]);
            let seen = sessions[observer]
                .entity(actor_id)
                .expect("actor")
                .activity
                .sample(profile())
                .expect("activity");
            assert!(
                seen.cycle
                    .is_none_or(|cycle| cycle.raw >= 0 && cycle.raw < 65536)
            );
            if seen.phase == ActivityPhase::Windup && seen.cycle > winding.cycle {
                assert!(query(&sessions[observer], observer, actor, 1503) > attack_query);
                advanced = true;
                break;
            }
        }
        assert!(advanced, "visible enemy windup cycle advances");
        let mut backswing = false;
        for _ in 0..60 {
            step(&mut local, &mut sessions, &[&[], &[]]);
            let body = sessions[observer]
                .entity(actor_id)
                .expect("visible attacker");
            if body
                .unit
                .action_phase
                .is_some_and(|phase| phase.phase == bota_proto::UnitActionPhase::AttackBackswing)
            {
                let sample = body.activity.sample(profile()).expect("backswing model");
                assert_eq!(sample.phase, ActivityPhase::Recovery);
                assert!(matches!(sample.activity, Activity::Attack { .. }));
                assert!(
                    sample
                        .cycle
                        .is_some_and(|cycle| cycle.raw >= 0 && cycle.raw < 65536)
                );
                backswing = true;
                break;
            }
        }
        assert!(
            backswing,
            "the authoritative attack backswing reaches the shared model"
        );
        let mut fog = fog_reader(
            if observer == 0 {
                Team::Radiant
            } else {
                Team::Dire
            },
            SlotId(observer as u8),
            actor,
        );
        assert!(
            fog.think(&sessions[observer])
                .expect("visible think")
                .failures
                .is_empty()
        );
        {
            let world = local.world_mut();
            world.cheat(
                SlotId(0),
                None,
                Cheat::AllVision { on: false },
                &mut Vec::new(),
            );
            for (index, at) in [
                (
                    actor,
                    bota_core::fountain_pos(
                        bota_core::map_of(bota_core::MAP2_ID),
                        if actor == 0 {
                            Team::Radiant
                        } else {
                            Team::Dire
                        },
                    ),
                ),
                (
                    observer,
                    bota_core::fountain_pos(
                        bota_core::map_of(bota_core::MAP2_ID),
                        if observer == 0 {
                            Team::Radiant
                        } else {
                            Team::Dire
                        },
                    ),
                ),
            ] {
                let unit = world.seats[index].unit.expect("hero");
                world.cheat(
                    SlotId(index as u8),
                    Some(unit),
                    Cheat::Teleport { at },
                    &mut Vec::new(),
                );
            }
        }
        step(&mut local, &mut sessions, &[&[], &[]]);
        let track = sessions[observer].entity(actor_id).expect("remembered");
        assert!(!track.visible);
        assert!(
            fog.think(&sessions[observer])
                .expect("hidden think")
                .failures
                .is_empty()
        );
        let frozen = track.activity;
        for _ in 0..8 {
            step(&mut local, &mut sessions, &[&[], &[]]);
        }
        assert_eq!(
            sessions[observer]
                .entity(actor_id)
                .expect("remembered")
                .activity,
            frozen
        );
    }
}

#[test]
fn real_cast_points_backswings_and_item_channels_publish_their_own_progress() {
    use bota_proto::{ItemId, ItemSlot, UnitActionPhase as Kind};
    for actor in [0, 1] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        {
            let world = local.world_mut();
            world.cheats = true;
            world.cheat(SlotId(0), None, Cheat::StartGame, &mut Vec::new());
            let hero = world.seats[actor].unit.expect("actor");
            world.cheat(
                SlotId(actor as u8),
                Some(hero),
                Cheat::Levels { count: 5 },
                &mut Vec::new(),
            );
            world.cheat(
                SlotId(actor as u8),
                Some(hero),
                Cheat::Refresh,
                &mut Vec::new(),
            );
            world.cheat(
                SlotId(0),
                None,
                Cheat::AllVision { on: true },
                &mut Vec::new(),
            );
            world.cheat(
                SlotId(actor as u8),
                Some(hero),
                Cheat::Teleport {
                    at: Vec2::from_ints(9000, 9000),
                },
                &mut Vec::new(),
            );
        }
        step(&mut local, &mut sessions, &[&[], &[]]);
        let learn = sessions[actor]
            .issue(IssuedOrder::hero(Order::Learn {
                slot: AbilitySlot(5),
            }))
            .expect("learn");
        let mut commands = [Vec::new(), Vec::new()];
        commands[actor].push(learn);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        let cast = sessions[actor]
            .issue(IssuedOrder::hero(Order::Cast {
                slot: AbilitySlot(5),
                target: Target::None,
            }))
            .expect("cast");
        let mut commands = [Vec::new(), Vec::new()];
        commands[actor].push(cast);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        let unit = sessions[actor].own_hero().expect("caster");
        let point = unit.action_phase.expect("phase");
        assert_eq!(point.phase, Kind::CastPoint);
        assert!(unit.cast.is_some());
        let mut recovered = false;
        for _ in 0..90 {
            step(&mut local, &mut sessions, &[&[], &[]]);
            let phase = sessions[actor]
                .own_hero()
                .expect("caster")
                .action_phase
                .expect("phase");
            if phase.phase == Kind::CastBackswing {
                assert!(phase.elapsed < u32::MAX);
                let observed = sessions[actor]
                    .entity(sessions[actor].own_hero().expect("hero").id)
                    .expect("track")
                    .activity
                    .sample(profile())
                    .expect("activity");
                assert!(
                    matches!(
                        observed.activity,
                        Activity::Cast {
                            ability: None,
                            slot: None
                        }
                    ),
                    "a recovery without an ability observation must not reuse the previous spell"
                );
                recovered = true;
                break;
            }
        }
        assert!(
            recovered,
            "cast recovery is projected separately from its cast point"
        );
        {
            let world = local.world_mut();
            let body = world.seats[actor].unit.expect("hero");

            world.cheat(
                SlotId(actor as u8),
                Some(body),
                Cheat::Item {
                    item: ItemId::TOWN_PORTAL_SCROLL,
                },
                &mut Vec::new(),
            );
            world.cheat(
                SlotId(actor as u8),
                Some(body),
                Cheat::Refresh,
                &mut Vec::new(),
            );
        }
        step(&mut local, &mut sessions, &[&[], &[]]);
        assert!(
            sessions[actor]
                .own_hero()
                .expect("hero")
                .town_portal
                .is_some(),
            "scroll in dedicated slot"
        );
        let use_scroll = sessions[actor]
            .issue(IssuedOrder::hero(Order::Use {
                slot: ItemSlot::TOWN_PORTAL,
                target: Target::None,
            }))
            .expect("use scroll");
        let mut commands = [Vec::new(), Vec::new()];
        commands[actor].push(use_scroll);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        let mut channel = None;
        for _ in 0..30 {
            let phase = sessions[actor]
                .own_hero()
                .expect("hero")
                .action_phase
                .expect("phase");
            if phase.phase == Kind::Channel {
                channel = Some(phase);
                break;
            }
            step(&mut local, &mut sessions, &[&[], &[]]);
        }
        let channel = channel.expect("real item channel");
        assert!(channel.elapsed < 1000);
        let hero = sessions[actor].own_hero().expect("hero");
        assert!(
            hero.channel.is_none(),
            "an item channel has no ability identity"
        );
        let activity = sessions[actor]
            .entity(hero.id)
            .expect("tracked caster")
            .activity
            .sample(profile())
            .expect("channel activity");
        assert!(
            matches!(
                activity.activity,
                Activity::Channel {
                    ability: None,
                    slot: None
                }
            ),
            "an item channel must discard the previous spell identity: {:?}",
            activity.activity
        );
        query(&sessions[actor], actor, actor, 1520);
        step(&mut local, &mut sessions, &[&[], &[]]);
        let next = sessions[actor]
            .own_hero()
            .expect("hero")
            .action_phase
            .expect("phase");
        assert_eq!(next.phase, Kind::Channel);
        assert!(next.elapsed > channel.elapsed);
    }
}

#[test]
fn missing_activity_profiles_return_sentinels_without_discarding_orders() {
    use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: bota_proto::HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [1, 2],
            random: 3,
        },
    )
    .expect("team");
    lua.add_script(SlotId(0),ScriptKind::Takeover,"no_profiles",b"function Think() local b=GetBot(); assert(b:GetAnimActivity()==-1); assert(b:GetAnimCycle()==-1); b:Action_MoveToLocation(Vector(0,0,0)) end").expect("script");
    let turn = lua.think(&sessions[0]).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
}
