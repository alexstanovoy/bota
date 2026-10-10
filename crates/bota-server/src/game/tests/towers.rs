//! Towers at Dota 7.41f: what they are made of, how fast they shoot, what
//! their fall pays and how often they let go.

use crate::game::{MELEE_CREEP, UnitDef, World, tower_def};
use bota_proto::{Fixed, Team, UnitKind, Vec2};

/// A creep that bears no armor and never falls.
const TARGET: UnitDef = UnitDef {
    max_hp: 30_000,
    damage: 0,
    armor: 0,
    hp_regen: Fixed::ZERO,
    move_speed: 0,
    ..MELEE_CREEP
};

#[test]
fn every_tier_stands_and_shoots_as_in_seven_forty_one() {
    // Liquipedia, Tower (Tier 1) to (Tier 4): health, armor and damage
    // range; every tier shoots at attack speed 110 on a 0.9 s base, so a
    // shot every 0.818 s, 36 or 37 of them in 30 seconds.
    let tiers = [
        (1, 1800, 12, (88, 92)),
        (2, 2500, 16, (170, 174)),
        (3, 2500, 16, (170, 174)),
        (4, 2600, 21, (170, 174)),
    ];
    for (tier, hp, armor, (low, high)) in tiers {
        let mut world = World::new();
        let tower = world.spawn_unit(tower_def(tier), Team::Radiant, Vec2::from_ints(1500, 1500));
        let on = world.spawn_unit(&TARGET, Team::Dire, Vec2::from_ints(1900, 1500));
        world.settle();
        world.fill_pools(on);
        let stats = world.stats.get(tower).expect("settled");
        assert_eq!(world.kind.get(tower), Some(&UnitKind::Tower));
        assert_eq!(
            (stats.max_hp, stats.armor, stats.attack_speed),
            (Fixed::from_int(hp), Fixed::from_int(armor), 110),
            "tier {tier}"
        );
        world.set_target(tower, on);
        let mut shots = Vec::new();
        let mut before = world.health.get(on).expect("standing").hp;
        let mut first = None;
        while first.is_none_or(|at| world.tick < at + 900) {
            world.step();
            let now = world.health.get(on).expect("standing").hp;
            if now < before {
                first.get_or_insert(world.tick);
                shots.push((before - now).to_int());
            }
            before = now;
            assert!(world.tick < 2_000, "tier {tier}: the tower shoots");
        }
        // The shot landing on the first tick counted opens the window.
        let in_window = shots.len() - 1;
        assert!(
            (36..=37).contains(&in_window),
            "tier {tier}: {in_window} shots in 30 seconds"
        );
        assert!(
            shots.iter().all(|shot| (low..=high).contains(shot)),
            "tier {tier}: {shots:?}"
        );
        assert!(
            shots.contains(&low) && shots.contains(&high),
            "tier {tier}: {shots:?}"
        );
    }
}
