use bota_proto::{EventKind, Fixed, HeroId, ServerMsg, SlotId, Target, Team};

use crate::game::{EventVisibility, wire_id};

use super::fixtures::*;

#[test]
fn mango_reports_actual_whole_mana_once_per_consumed_charge() {
    for deficit in [
        Fixed::ONE,
        Fixed::from_ratio(199, 2),
        Fixed::from_int(100),
        Fixed::from_int(500),
    ] {
        for explicit_self in [false, true] {
            let (mut world, hero) = fixture();
            put(&mut world, hero, 0, mango(3));
            let maximum = world.stats.get(hero).unwrap().max_mana;
            world.mana.get_mut(hero).unwrap().mana = maximum - deficit;
            let health = world.health.get(hero).unwrap().hp;
            let target = if explicit_self {
                Target::Unit(wire_id(hero))
            } else {
                Target::None
            };
            let mut events = Vec::new();

            assert!(world.use_item(hero, 0, target, &mut events));

            let restored = deficit.min(Fixed::from_int(100));
            assert_eq!(
                world.mana.get(hero).unwrap().mana,
                maximum - deficit + restored
            );
            assert_eq!(world.health.get(hero).unwrap().hp, health);
            assert_eq!(held(&world, hero, 0).charges, 2);
            assert_eq!(events.len(), 1);
            assert_eq!(
                events[0].kind,
                EventKind::Healed {
                    source: Some(wire_id(hero)),
                    target: wire_id(hero),
                    amount: 0,
                    mana: restored.to_int(),
                }
            );
        }
    }
}

#[test]
fn subpoint_mango_restoration_consumes_without_a_zero_point_event() {
    for deficit in [
        Fixed::EPSILON,
        Fixed::from_ratio(1, 2),
        Fixed::ONE - Fixed::EPSILON,
    ] {
        let (mut world, hero) = fixture();
        put(&mut world, hero, 0, mango(1));
        let maximum = world.stats.get(hero).unwrap().max_mana;
        world.mana.get_mut(hero).unwrap().mana = maximum - deficit;
        let mut events = Vec::new();

        assert!(world.use_item(hero, 0, Target::None, &mut events));

        assert_eq!(world.mana.get(hero).unwrap().mana, maximum);
        assert!(world.inventory.get(hero).unwrap().slots[0].is_none());
        assert!(events.is_empty());
    }
}

#[test]
fn mango_mana_event_uses_upstream_visibility_and_round_trips_on_the_wire() {
    for observed in [false, true] {
        let (mut world, hero) = fixture();
        put(&mut world, hero, 0, mango(1));
        if observed {
            let at = world.transform.get(hero).unwrap().pos;
            world.spawn_hero(Team::Dire, at, SlotId(1), HeroId(2));
        }
        world.settle();
        let mut events = Vec::new();

        assert!(world.use_item(hero, 0, Target::None, &mut events));

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].visible_to,
            if observed {
                EventVisibility::Everyone
            } else {
                EventVisibility::OneTeam(Team::Radiant)
            }
        );
        let message = ServerMsg::Events {
            tick: world.tick,
            events: vec![events[0].kind.clone()],
        };
        let encoded = bota_proto::encode_frame_to_vec(&message).unwrap();
        let decoded: ServerMsg =
            bota_proto::decode_payload(&encoded[bota_proto::LEN_PREFIX..]).unwrap();
        assert_eq!(decoded, message);
    }
}

#[test]
fn mango_passive_regeneration_does_not_repeat_its_consumption_event() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    world.transform.get_mut(hero).unwrap().pos = AWAY;
    let mut events = Vec::new();
    assert!(world.use_item(hero, 0, Target::None, &mut events));
    assert_eq!(events.len(), 1);

    let later = world.step();

    assert!(
        !later
            .iter()
            .any(|event| matches!(event.kind, EventKind::Healed { .. }))
    );
    assert_eq!(held(&world, hero, 0).charges, 2);
}
