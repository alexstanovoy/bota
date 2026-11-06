//! The owner knows a point it aimed at, even when its sight does not reach it.

use bota_proto::{EventKind, ItemId, ItemSlot, Order, Target, Team, Vec2};

use crate::game::World;

use super::support::*;

#[test]
fn a_nonmoving_point_use_tells_its_owner_the_destination_in_fog() {
    for side in [Team::Radiant, Team::Dire] {
        let mut world = World::new();
        let from = Vec2::from_ints(1000, 1000);
        let at = Vec2::from_ints(1900, 1000);
        let hero = seat_hero(&mut world, 0, side, from);
        hand_item(&mut world, hero, ItemId::BLOOD_GRENADE, 1);
        world.settle();
        assert!(
            !world.can_see_point(side, at),
            "the aim is beyond night vision"
        );
        let mut events = advance_validated(
            &mut world,
            None,
            Order::Use {
                slot: ItemSlot(0),
                target: Target::Pos(at),
            },
        );
        for _ in 0..90 {
            if events.iter().any(|event| {
                matches!(
                    event.kind(),
                    EventKind::ItemUsed {
                        item: ItemId::BLOOD_GRENADE,
                        ..
                    }
                )
            }) {
                break;
            }
            events.extend(world.step());
        }
        assert_eq!(
            world.pos(hero),
            Some(from),
            "throwing does not move the caster"
        );
        assert!(!world.can_see_point(side, at));
        let told = crate::game_loop::visible_events(&events, Some(side));
        let destination = told
            .iter()
            .find_map(|event| match event {
                EventKind::ItemUsed {
                    item: ItemId::BLOOD_GRENADE,
                    at,
                    ..
                } => Some(*at),
                _ => None,
            })
            .expect("one's own use was told");
        assert_eq!(destination, Some(at), "the owner knows its own aimed point");
    }
}
