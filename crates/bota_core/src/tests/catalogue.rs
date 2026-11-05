//! What an item is to a match is all of its row: two builds that differ on any number or rule of
//! an item must not meet on it.

use bota_proto::{Aim, ItemId};

use crate::catalogue::item_digest;
use crate::{AllyUse, Behavior, ItemDef, Spends, UseSpec, item_def};

/// A field of a row, by its name, and a change of it.
type Change = (&'static str, fn(&mut ItemDef));

#[test]
fn every_rule_of_an_item_moves_its_digest() {
    let bottle = *item_def(ItemId::BOTTLE).expect("the Bottle is in the catalogue");
    let changes: [Change; 22] = [
        ("retired", |d| d.retired = !d.retired),
        ("cast_range", |d| d.cast_range += 1),
        ("spends", |d| {
            d.spends = if d.spends == Spends::All {
                Spends::One
            } else {
                Spends::All
            };
        }),
        ("point", |d| d.point += 1),
        ("backswing", |d| d.backswing += 1),
        ("duration", |d| d.duration += 1),
        ("mana_deficit", |d| d.mana_deficit = !d.mana_deficit),
        ("immediate", |d| d.immediate = !d.immediate),
        ("ignores_channel", |d| {
            d.ignores_channel = !d.ignores_channel
        }),
        ("unbuyable", |d| d.unbuyable = !d.unbuyable),
        ("kept_empty", |d| d.kept_empty = !d.kept_empty),
        ("lifetime", |d| d.lifetime += 1),
        ("aim", |d| d.aim = Some(Aim::Point)),
        ("behavior", |d| d.behavior = d.behavior.and(Behavior::AOE)),
        ("values", |d| d.values = &[]),
        ("ally_target", |d| d.ally_target = !d.ally_target),
        ("ally", |d| d.ally = Some(AllyUse::Effect { percent: 50 })),
        ("effect kind", |d| d.effect = UseSpec::None),
        ("effect health", |d| drink(d).health += 1),
        ("effect mana", |d| drink(d).mana += 1),
        ("effect ticks", |d| drink(d).ticks += 1),
        ("effect range", |d| drink(d).range += 1),
    ];
    assert_eq!(bottle.aim, Some(Aim::Own));
    assert!(!bottle.behavior.has(Behavior::AOE));
    assert!(!bottle.values.is_empty());
    assert_eq!(bottle.ally, None);
    for (field, change) in changes {
        let mut changed = bottle;
        change(&mut changed);
        assert_ne!(item_digest(&changed), item_digest(&bottle), "{field}");
    }
}

/// The Bottle's drink, which the effect cases change.
fn drink(def: &mut ItemDef) -> &mut crate::Drink {
    match &mut def.effect {
        UseSpec::Bottle(drink) => drink,
        _ => panic!("the Bottle drinks"),
    }
}
