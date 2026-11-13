mod support;
use bota_client_core::IssuedOrder;
use bota_proto::{Cheat, Order, SlotId, Target, Team, Vec2};
use bota_server::game::{ActionPhase, ActionState};

#[test]
fn a_visible_attackers_swing_does_not_publish_its_hidden_neutral_target() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
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
        neutral = world.spawn_neutral(
            bota_core::NeutralKind::Kobold,
            Vec2::from_ints(9450, 9450),
            0,
            0,
        );
        for (index, at) in [
            (0, Vec2::from_ints(8000, 8000)),
            (1, Vec2::from_ints(9100, 9100)),
        ] {
            let hero = world.seats[index].unit.expect("hero");
            world.cheat(
                SlotId(index as u8),
                Some(hero),
                Cheat::Teleport { at },
                &mut Vec::new(),
            );
        }
    }
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[], &[]]).expect("step"))
    {
        support::fold(session, stream);
    }
    let neutral_id = local
        .world()
        .view_full()
        .units
        .iter()
        .find(|u| u.neutral == Some(bota_core::NeutralKind::Kobold.id()))
        .expect("neutral")
        .id;
    let attacker = sessions[1].own_hero().expect("attacker").id;
    let stop = sessions[0]
        .issue(IssuedOrder::hero(Order::Move {
            target: Target::None,
        }))
        .expect("stop");
    let attack = sessions[1]
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
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[stop], &[attack]]).expect("attack step"))
    {
        support::fold(session, stream);
    }
    let world = local.world();
    let body = world.seats[1].unit.expect("body");
    assert!(
        matches!(world.action.get(body).expect("action").state,ActionState::Attack{target,phase:ActionPhase::Before{..}} if target==neutral)
    );
    let view = sessions[0].current().expect("observer view");
    assert!(view.unit(attacker).is_some(), "attacker remains visible");
    assert!(
        view.unit(neutral_id).is_none(),
        "target outside viewer sight"
    );
    assert!(
        view.aggro
            .iter()
            .find(|row| row.unit == attacker)
            .is_none_or(|row| row.swing.is_none()),
        "target identity stays withheld"
    );
}
