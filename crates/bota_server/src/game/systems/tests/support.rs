//! Fixtures the systems unit tests share.

use std::fmt::Debug;

use bota_core::{MELEE_CREEP, ModifierKind, Ratio, UnitDef, rules};
use bota_proto::{Fixed, ItemId, ModifierSpec, SlotId};

use crate::game::{Chance, Entity, Event, ItemStack, Modifier, Modifiers, World};

/// What takes a strike and never falls: no armor, no mending, no swing back.
pub(super) const ANVIL: UnitDef = UnitDef {
    max_hp: 30_000,
    armor: 0,
    damage: 0,
    hp_regen: Fixed::ZERO,
    ..MELEE_CREEP
};

/// A haste that holds a body's attack speed at `speed` for good.
pub(super) fn swinging_at(speed: i32) -> Modifiers {
    Modifiers(vec![Modifier {
        kind: ModifierKind::Haste {
            speed: speed - rules::BASE_ATTACK_SPEED,
        },
        source: None,
        ticks_left: Some(u32::MAX / 2),
    }])
}

/// Puts a freshly bought item in one of a hero's slots.
pub(super) fn hold(world: &mut World, hero: Entity, slot: usize, item: ItemId) {
    let stack = ItemStack::bought(item, SlotId(0), 0).expect("the item is sold");
    world.inventory.get_mut(hero).expect("has a bag").slots[slot] = Some(stack);
}

/// The roll stream an entity has opened in one of the world's stream tables.
pub(super) fn opened(streams: &[Option<Chance>], of: Entity) -> Option<&Chance> {
    streams.get(of.index().0 as usize)?.as_ref()
}

/// Steps until `stream` opens (at most `ticks` steps). Returns what `told`
/// read off those steps and how many rolls of its first block the opening
/// roll left, itself included.
pub(super) fn until_opened<T>(
    world: &mut World,
    ticks: usize,
    told: impl Fn(&[Event]) -> Vec<T>,
    stream: impl Fn(&World) -> Option<&Chance>,
) -> (Vec<T>, usize) {
    let mut seen = Vec::new();
    for _ in 0..ticks {
        seen.extend(told(&world.step()));
        if let Some(chance) = stream(world) {
            let den = usize::from(chance.current().den());
            let first = match usize::from(chance.block_position()) {
                0 => 1,
                pos => den - (pos - 1),
            };
            return (seen, first);
        }
    }
    panic!("no roll within {ticks} ticks")
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

/// Asserts that past the first `first` outcomes come at least five full
/// blocks, each holding exactly `ratio.num()` hits.
pub(super) fn assert_exact_blocks<T: Debug>(
    seen: &[T],
    first: usize,
    ratio: Ratio,
    hit: impl Fn(&T) -> bool,
    case: &str,
) {
    let full = seen[first..].chunks_exact(usize::from(ratio.den()));
    assert!(full.len() >= 5, "{case}: five full blocks");
    for (nth, block) in full.enumerate() {
        let hits = block.iter().filter(|outcome| hit(outcome)).count();
        assert_eq!(
            hits,
            usize::from(ratio.num()),
            "{case}, block {nth}: {block:?}"
        );
    }
}

/// A nominal spec with one change on it.
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
