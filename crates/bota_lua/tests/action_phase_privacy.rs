mod support;
use bota_client_core::IssuedOrder;
use bota_proto::{
    AbilityId, AbilitySlot, Cheat, HeroId, ModifierSpec, Order, SlotId, Target, Team,
    UnitActionPhase, Vec2,
};

fn step(
    local: &mut bota_server::local::LocalMatch,
    sessions: &mut [bota_client_core::ClientSession],
    commands: &[&[bota_proto::OutgoingOrder]],
) {
    for (session, stream) in sessions.iter_mut().zip(local.step(commands).expect("step")) {
        support::fold(session, stream);
    }
}

/// Radiant sees Pudge channel on a hidden neutral; compare the same observed instant.
fn run(resist: i32, dire_at: i32) -> (Option<bota_proto::ActionPhaseView>, Vec<Vec<u8>>) {
    let (mut local, mut sessions) = support::local_with_modifiers(
        &[
            (Team::Radiant, HeroId::SHADOW_FIEND),
            (Team::Dire, HeroId::PUDGE),
        ],
        Vec::new(),
    );
    let neutral;
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
        let hero = world.seats[1].unit.expect("pudge");
        world.cheat(
            SlotId(1),
            Some(hero),
            Cheat::Levels { count: 5 },
            &mut Vec::new(),
        );
        world.cheat(SlotId(1), Some(hero), Cheat::Refresh, &mut Vec::new());
        neutral = world.spawn_neutral(
            bota_core::NeutralKind::Kobold,
            Vec2::from_ints(9300, 9300),
            0,
            0,
        );
        world.cheat(
            SlotId(1),
            Some(hero),
            Cheat::Teleport {
                at: Vec2::from_ints(9200, 9200),
            },
            &mut Vec::new(),
        );
        let dire = world.seats[0].unit.expect("obs");
        world.cheat(
            SlotId(0),
            Some(dire),
            Cheat::Teleport {
                at: Vec2::from_ints(dire_at, dire_at),
            },
            &mut Vec::new(),
        );
        if resist != 0 {
            let mut spec = ModifierSpec::NOMINAL;
            spec.status_resist = resist;
            world.cheat(
                SlotId(1),
                Some(hero),
                Cheat::ApplyModifier {
                    target: Target::Unit(bota_server::game::wire_id(neutral)),
                    spec,
                    ticks: 2000,
                },
                &mut Vec::new(),
            );
        }
    }
    step(&mut local, &mut sessions, &[&[], &[]]);
    let slot = sessions[1]
        .own_hero()
        .unwrap()
        .abilities
        .iter()
        .position(|a| a.id == AbilityId::DISMEMBER)
        .expect("dismember slot");
    let slot = AbilitySlot(slot as u8);
    let learn = sessions[1]
        .issue(IssuedOrder::hero(Order::Learn { slot }))
        .expect("learn");
    step(&mut local, &mut sessions, &[&[], &[learn]]);
    let nid = sessions[1]
        .current()
        .unwrap()
        .units
        .iter()
        .find(|u| u.neutral.is_some())
        .map(|u| u.id)
        .expect("neutral in view");
    let cast = sessions[1]
        .issue(IssuedOrder::hero(Order::Cast {
            slot,
            target: Target::Unit(nid),
        }))
        .expect("cast");
    step(&mut local, &mut sessions, &[&[], &[cast]]);
    let pudge = local.world().seats[1].unit.unwrap();
    let pid = bota_server::game::wire_id(pudge);
    for _ in 0..80 {
        let v = local.world().view(Team::Radiant);
        if let Some(p) = v.unit(pid).and_then(|u| u.action_phase)
            && p.phase == UnitActionPhase::Channel
        {
            // the target is hidden for the observer once cheat vision goes away
            local.world_mut().cheat(
                SlotId(0),
                None,
                Cheat::AllVision { on: false },
                &mut Vec::new(),
            );
            step(&mut local, &mut sessions, &[&[], &[]]);
            assert!(!local.world().shows(Team::Radiant, neutral));
            assert!(local.world().shows(Team::Radiant, pudge));
            let stream: Vec<Vec<u8>> = local.restate()[0]
                .iter()
                .map(|m| bota_proto::encode_frame_to_vec(m).unwrap())
                .collect();
            return (
                local
                    .world()
                    .view(Team::Radiant)
                    .unit(pid)
                    .and_then(|u| u.action_phase),
                stream,
            );
        }
        step(&mut local, &mut sessions, &[&[], &[]]);
    }
    panic!("no channel");
}

#[test]
fn probe_dismember_phase_must_not_depend_on_hidden_target_resistance() {
    let dire_at = 8000;
    let (a, sa) = run(0, dire_at);
    let (b, sb) = run(5000, dire_at);
    assert_eq!(
        a, b,
        "LEAK: Dismember phase counters differ with the hidden target's status resistance"
    );
    assert_eq!(sa, sb);
}
