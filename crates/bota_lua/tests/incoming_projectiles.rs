mod support;
use bota_client_core::{ClientSession, IssuedOrder};
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Cheat, HeroId, Order, OutgoingOrder, SlotId, Target, Team, Vec2};

fn step(
    local: &mut bota_server::local::LocalMatch,
    sessions: &mut [ClientSession],
    commands: &[&[OutgoingOrder]],
) {
    for (session, stream) in sessions.iter_mut().zip(local.step(commands).expect("step")) {
        support::fold(session, stream);
    }
}
fn reader(observer: usize, actor: usize) -> LuaTeam {
    let slot = SlotId(observer as u8);
    let mut team = LuaTeam::new(
        if observer == 0 {
            Team::Radiant
        } else {
            Team::Dire
        },
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
    team.set_gc_stress(bota_lua::GcStress::Full);
    let player = if actor == 0 { 0 } else { 5 };
    let source = format!(
        r#"
        local phase,caster=0,nil
        function Think()
            phase=phase+1
            local bot=GetBot()
            assert(not pcall(bot.GetIncomingTrackingProjectiles,{{}}))
            local shots=bot:GetIncomingTrackingProjectiles()
            assert(type(shots)=='table' and #shots>0)
            local shot=shots[1]
            assert(shot.location.x>=-216 and shot.location.x<=384)
            assert(shot.location.y==-216, "projectile y="..shot.location.y)
            assert(shot.is_attack==true and shot.is_dodgeable==true)
            assert(shot.ability==nil)
            if phase==1 then
                caster=shot.caster
                assert(caster and caster:GetPlayerID()=={player})
                assert(shot.playerid=={player})
                assert(type(caster:GetIncomingTrackingProjectiles())=='table')
            else
                assert(shot.caster==nil and shot.playerid==-1)
                assert(caster:GetIncomingTrackingProjectiles()==nil)
            end
            bot:Action_MoveToLocation(shot.location)
        end
    "#
    );
    team.add_script(
        slot,
        ScriptKind::Takeover,
        "incoming_projectiles",
        source.as_bytes(),
    )
    .expect("script");
    team
}
#[test]
fn incoming_attacks_follow_real_missiles_without_revealing_an_unseen_caster() {
    for observer in [0, 1] {
        let actor = 1 - observer;
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
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
                (observer, Vec2::from_ints(9550, 9000)),
            ] {
                let id = world.seats[index].unit.expect("hero");
                world.cheat(
                    SlotId(index as u8),
                    Some(id),
                    Cheat::Teleport { at },
                    &mut Vec::new(),
                );
            }
        }
        step(&mut local, &mut sessions, &[&[], &[]]);
        let stops: Vec<_> = sessions
            .iter_mut()
            .map(|s| {
                s.issue(IssuedOrder::hero(Order::Move {
                    target: Target::None,
                }))
                .expect("stop")
            })
            .collect();
        step(&mut local, &mut sessions, &[&[stops[0]], &[stops[1]]]);
        let target = sessions[observer].own_hero().expect("target").id;
        let attack = sessions[actor]
            .issue(IssuedOrder::hero(Order::Attack {
                target: Target::Unit(target),
            }))
            .expect("attack");
        let mut orders = [Vec::new(), Vec::new()];
        orders[actor].push(attack);
        step(&mut local, &mut sessions, &[&orders[0], &orders[1]]);
        let mut found = false;
        for _ in 0..90 {
            if sessions[observer]
                .current()
                .expect("view")
                .projectiles
                .iter()
                .any(|p| p.target == Some(target))
            {
                found = true;
                break;
            }
            step(&mut local, &mut sessions, &[&[], &[]]);
        }
        assert!(found, "real attack missile");
        let mut reader = reader(observer, actor);
        let turn = reader.think(&sessions[observer]).expect("seen think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(turn.bots[0].orders.len(), 1, "query-derived move");
        {
            let world = local.world_mut();
            world.cheat(
                SlotId(0),
                None,
                Cheat::AllVision { on: false },
                &mut Vec::new(),
            );
            let id = world.seats[actor].unit.expect("actor");
            let map = bota_core::map_of(bota_core::MAP2_ID);
            let at = bota_core::fountain_pos(
                map,
                if actor == 0 {
                    Team::Radiant
                } else {
                    Team::Dire
                },
            );
            world.cheat(
                SlotId(actor as u8),
                Some(id),
                Cheat::Teleport { at },
                &mut Vec::new(),
            );
        }
        step(&mut local, &mut sessions, &[&[], &[]]);
        let turn = reader
            .think(&sessions[observer])
            .expect("unseen caster think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(turn.bots[0].orders.len(), 1, "missile remains observed");
    }
}
