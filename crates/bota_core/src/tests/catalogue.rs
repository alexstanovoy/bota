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
    let changes: [Change; 34] = [
        ("cooldown group", |d| {
            d.cooldown_group = Some(ItemId::REFRESHER_SHARD)
        }),
        ("inventory cooldown", |d| d.cooldown_in_inventory = true),
        ("public loot", |d| d.public_loot = true),
        ("public placement", |d| d.public_placement = true),
        ("drop rule", |d| d.limits.droppable = false),
        ("sale rule", |d| d.limits.sellable = false),
        ("ground destruction", |d| d.limits.killable = false),
        ("backpack rule", |d| d.limits.backpack = false),
        ("pickup eligibility", |d| {
            d.pickup = crate::PickupEligibility::RealHero
        }),
        ("pickup lifetime", |d| d.pickup_lifetime = 1),
        ("reincarnation", |d| d.reincarnation = Some(1)),
        ("expiry recovery", |d| {
            d.expiry_recovery = Some(crate::Recovery {
                ticks: 1,
                health_pct: 20,
                mana_pct: 20,
            })
        }),
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

#[test]
fn adding_roshan_items_keeps_the_preexisting_catalogue_prefix_identical() {
    let previous = bota_proto::CataloguePrefix {
        items: 62,
        digest: 0xe0e7_501a_8b65_4c20,
    };
    let shared = crate::Catalogue::agreeing(previous)
        .expect("the prefix published before Roshan still negotiates");
    assert_eq!(shared.prefix(), previous);
}
