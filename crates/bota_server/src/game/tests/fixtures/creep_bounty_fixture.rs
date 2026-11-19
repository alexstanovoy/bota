//! Pins the gold range and experience assigned by the previous server at every creep upgrade.
//! To regenerate, add this test module to `stats.rs` in a source archive of the base commit.
//! Set `BOTA_BOUNTY_FIXTURE_BASE` to the commit ID read with `git get-tar-commit-id`, then
//! run `capture_creep_bounty_fixture` with `--ignored --nocapture`. Keep the lines prefixed
//! with `FIXTURE ` and strip that prefix.

use bota_core::{UnitDef, rules};
use bota_proto::{Team, Vec2};

use crate::game::World;

pub(super) fn creep_bounty_cases() -> impl Iterator<Item = (String, &'static UnitDef, u32)> {
    let last_lane_step = u32::MAX / (rules::WAVE_PERIOD_TICKS * rules::WAVE_UPGRADE_PERIOD);
    let lane = bota_core::FIXED_UNIT_DEFS
        .into_iter()
        .filter(|def| bota_core::lane_creep_def(def.kind).is_some())
        .map(move |def| {
            (
                format!("{:?}/{:?}", def.kind, def.tier),
                def,
                last_lane_step,
            )
        });
    let neutral = bota_core::NEUTRALS.iter().map(|neutral| {
        let earliest = rules::Schedule::of(0);
        let last_step = neutral
            .growth
            .map_or(0, |growth| growth.clock.count(u32::MAX, earliest));
        (neutral.unit.to_owned(), &neutral.def, last_step)
    });
    lane.chain(neutral)
}

#[test]
#[ignore = "fixture generator for the previous rules build"]
fn capture_creep_bounty_fixture() {
    assert_eq!(
        bota_core::RULES_IDENTITY,
        0x2ab3_82ef_214f_2eba,
        "base rules identity"
    );
    let commit = std::env::var("BOTA_BOUNTY_FIXTURE_BASE").expect("archived commit ID");
    assert!(commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let base = &commit[..8];
    println!("FIXTURE # bota {base}: server gold min/max and experience by creep and upgrade step");
    println!("FIXTURE # Kind | last step | step zero | odd-step increment | even-step increment");
    for (name, def, last_step) in creep_bounty_cases() {
        let mut world = World::new();
        let at = Vec2::from_ints(5000, 5000);
        let neutral = bota_core::NEUTRALS.iter().find(|entry| entry.unit == name);
        let entity = match neutral {
            Some(entry) => world.spawn_neutral(entry.kind, at, 0, 0),
            None => world.spawn_creep(def, Team::Dire, at, 0, 0),
        };
        let sample = |world: &World| {
            world.bounty.get(entity).map_or([0; 3], |bounty| {
                [bounty.gold.min, bounty.gold.max, bounty.xp]
            })
        };
        let starting = sample(&world);
        let mut prior = starting;
        let mut increments = [[0; 3]; 2];
        for step in 1..=last_step {
            if neutral.is_some() {
                world.set_neutral_upgrades(entity, step);
            } else {
                world.set_upgrades(entity, def, step);
            }
            let current = sample(&world);
            let delta: [i32; 3] = std::array::from_fn(|index| current[index] - prior[index]);
            let parity = ((step - 1) % 2) as usize;
            if step <= 2 {
                increments[parity] = delta;
            }
            assert_eq!(delta, increments[parity], "{name} step {step}");
            prior = current;
        }
        let values = [starting, increments[0], increments[1]]
            .into_iter()
            .flatten()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        println!("FIXTURE {name} {last_step} {values}");
    }
}
