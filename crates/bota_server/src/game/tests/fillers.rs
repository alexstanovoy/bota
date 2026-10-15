//! The base fillers: seven statues a side that pay gold and nothing else,
//! struck only once a tier three has fallen.

use bota_core::rules;

use crate::game::{World, map_of};
use bota_proto::{MapId, Team, UnitKind, Vec2};

use super::support::*;

#[test]
fn fillers_wait_on_a_tier_three_and_pay_the_killer_when_they_fall() {
    let mut world = World::on_map(map_of(MapId(0)));
    for team in [Team::Radiant, Team::Dire] {
        let count = world
            .entities
            .iter()
            .filter(|e| {
                world.kind.get(*e) == Some(&UnitKind::Filler) && world.team.get(*e) == Some(&team)
            })
            .count();
        assert_eq!(count, 7, "seven fillers a side");
    }
    let at = rules::RADIANT_FILLERS[0];
    let filler = structure_at(&world, at);
    let stats = world.stats.get(filler).expect("settled");
    assert_eq!(
        (stats.max_hp, stats.armor),
        (
            bota_proto::Fixed::from_int(rules::FILLER_HP),
            bota_proto::Fixed::from_int(rules::FILLER_ARMOR)
        )
    );
    assert!(!open_at(&world, at), "guarded until a tier three falls");
    fell_at(&mut world, rules::RADIANT_TOWERS[2].2);
    assert!(open_at(&world, at), "any tier three opens every filler");
    assert!(open_at(&world, rules::RADIANT_FILLERS[6]));

    let killer = seat_hero(&mut world, 0, Team::Dire, Vec2::from_ints(3000, 5000));
    let gold = world.seats[0].gold;
    let mut events = Vec::new();
    let filler = structure_at(&world, at);
    world.bury(vec![(filler, Some(killer))], &mut events);
    assert_eq!(world.seats[0].gold - gold, rules::FILLER_BOUNTY);
    assert_eq!(paid(&events), Some(rules::FILLER_BOUNTY));
}
