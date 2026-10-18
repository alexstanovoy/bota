//! The buildings as the game's `npc_units.txt` (client 6957) has them:
//! health, regeneration, armor and sight, and the names a player reads.

use bota_core::{map_of, rules, unit_name};
use bota_proto::{Fixed, MapId, Team, UnitKind};

use crate::game::World;

#[test]
fn every_building_stands_as_the_unit_table_has_it() {
    let world = World::on_map(map_of(MapId(0)));
    let mut seen = 0;
    for entity in world.entities.iter() {
        let (Some(kind), Some(team)) = (
            world.kind.get(entity).copied(),
            world.team.get(entity).copied(),
        ) else {
            continue;
        };
        let stats = world.stats.get(entity).expect("settled");
        let hp = stats.max_hp.to_int();
        let regen = (stats.hp_regen.to_f32() * rules::TICKS_PER_SECOND as f32).round() as i32;
        let (armor, per_second, name) = match (kind, hp) {
            (UnitKind::Ancient, 4500) => (
                23,
                12,
                if team == Team::Radiant {
                    "Radiant's Ancient"
                } else {
                    "Dire's Ancient"
                },
            ),
            (UnitKind::Barracks, 2200) => (15, 5, "Melee Barracks"),
            (UnitKind::Barracks, 1300) => (9, 0, "Ranged Barracks"),
            (UnitKind::Filler, 1000) => (10, 0, "Building"),
            (UnitKind::Tower, _) | (UnitKind::Fountain, _) => continue,
            other => panic!("a building the table does not know: {other:?}"),
        };
        assert_eq!(stats.armor, Fixed::from_int(armor), "{name}");
        assert_eq!(regen, per_second, "{name}");
        assert_eq!(unit_name(kind, team, hp), name);
        seen += 1;
    }
    assert_eq!(seen, 2 + 12 + 14);
}
