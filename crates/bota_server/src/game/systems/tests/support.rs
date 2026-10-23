//! Fixtures the systems unit tests share.

use std::fmt::Debug;

use bota_core::{MELEE_CREEP, Odds, UnitDef};
use bota_proto::{Fixed, ItemId, ModifierSpec, SlotId};

use crate::game::{Def, Entity, Event, ItemStack, PseudoRandom, Rolls, World};

/// What takes a strike and never falls: no armor, no mending, no swing back.
pub(super) const ANVIL: UnitDef = UnitDef {
    max_hp: 30_000,
    armor: 0,
    damage: 0,
    hp_regen: Fixed::ZERO,
    ..MELEE_CREEP
};

/// Sets how fast a body swings, at `speed` where [`rules::BASE_ATTACK_SPEED`]
/// is its plain pace, by giving it a body of its own kind at that pace. The
/// body is leaked: tests make a handful.
pub(super) fn swing_at(world: &mut World, entity: Entity, speed: i32) {
    let plain = world.def.get(entity).expect("has a body").0;
    let quick: &'static UnitDef = Box::leak(Box::new(UnitDef {
        attack_speed: speed,
        ..*plain
    }));
    world.def.insert(entity, Def(quick));
}

/// Puts a freshly bought item in one of a hero's slots.
pub(super) fn hold(world: &mut World, hero: Entity, slot: usize, item: ItemId) {
    let stack = ItemStack::bought(item, SlotId(0), 0).expect("the item is sold");
    world.inventory.get_mut(hero).expect("has a bag").slots[slot] = Some(stack);
}

/// The pseudo-random sequence an entity has opened in one of the world's
/// tables of one sequence each.
pub(super) fn opened(streams: &[Option<PseudoRandom>], of: Entity) -> Option<&PseudoRandom> {
    streams.get(of.index().0 as usize)?.as_ref()
}

/// The pseudo-random sequences an entity has opened in one of the world's
/// tables of sequences by place.
pub(super) fn opened_rolls(streams: &[Option<Rolls>], of: Entity) -> Option<&Rolls> {
    streams.get(of.index().0 as usize)?.as_ref()
}

/// Steps (at most 800 times) until `seen` holds `wanted` entries of what
/// `told` reads off each step.
pub(super) fn step_until<T>(
    world: &mut World,
    seen: &mut Vec<T>,
    wanted: usize,
    told: impl Fn(&[Event]) -> Vec<T>,
) {
    for _ in 0..800 {
        if seen.len() >= wanted {
            return;
        }
        seen.extend(told(&world.step()));
    }
}

/// Asserts that outcomes rolled at `odds` follow Dota's pseudo-random
/// distribution: never a longer run without a hit than its step allows,
/// and over them all a rate within five points of the chance.
pub(super) fn assert_dota_runs<T: Debug>(
    seen: &[T],
    odds: Odds,
    hit: impl Fn(&T) -> bool,
    case: &str,
) {
    assert!(seen.len() >= 100, "{case}: a hundred outcomes or more");
    let step = odds.prd_step();
    let longest = ((1u64 << 32).div_ceil(step) - 1) as usize;
    let mut run = 0;
    for (nth, outcome) in seen.iter().enumerate() {
        run = if hit(outcome) { 0 } else { run + 1 };
        assert!(
            run <= longest,
            "{case}: {run} misses running at {nth}, {longest} at most"
        );
    }
    let hits = seen.iter().filter(|outcome| hit(outcome)).count() as u64;
    let rate = hits * u64::from(Odds::WHOLE) / seen.len() as u64;
    assert!(
        rate.abs_diff(u64::from(odds.get())) <= u64::from(Odds::WHOLE) / 20,
        "{case}: {hits} of {} at {odds:?}",
        seen.len()
    );
}

pub(super) fn spec(change: impl FnOnce(&mut ModifierSpec)) -> ModifierSpec {
    let mut spec = ModifierSpec::NOMINAL;
    change(&mut spec);
    spec
}

/// A spec with a non-neutral value in every field, inside the bounds.
pub(super) fn a_spec() -> ModifierSpec {
    ModifierSpec {
        magic_resist: 1_000,
        status_resist: 2_000,
        physical_damage: 11_000,
        magic_damage: 12_000,
        pure_damage: 13_000,
        cooldown_rate: 9_000,
        mana_cost_rate: 8_000,
        move_speed: 8_500,
        max_hp: 11_000,
        max_mana: 12_000,
        gold_income: 7_500,
    }
}
