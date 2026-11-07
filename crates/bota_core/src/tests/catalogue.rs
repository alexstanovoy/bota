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
        ("wait key", |d| d.wait_key = Some(ItemId::REFRESHER_SHARD)),
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
        items: 88,
        // Independently measured on pristine landing main bc20dfbb, before Roshan, and moved since
        // by two rules of items of that prefix: the Magic Wand waits under the Magic Stick's
        // cooldown (`AbilitySharedCooldown` "magicwand", digested as its `wait_key`), and the
        // orbs' attack passives open their part of the digest with a tag of their own.
        digest: 0xb793_0c5b_328e_746b,
    };
    assert_eq!(
        crate::Catalogue::first(previous.items)
            .expect("prefix")
            .prefix(),
        previous
    );
    let shared = crate::Catalogue::agreeing(previous)
        .expect("the prefix published before Roshan still negotiates");
    assert_eq!(shared.prefix(), previous);
}

#[test]
fn ward_digest_distinguishes_reordered_or_repeated_banner_level_payloads() {
    let banner = *item_def(ItemId::ROSHANS_BANNER).expect("banner");
    static REPEATED: [crate::WardDef; 3] = [crate::BANNER_WARDS[2]; 3];
    static REORDERED: [crate::WardDef; 3] = [
        crate::BANNER_WARDS[2],
        crate::BANNER_WARDS[1],
        crate::BANNER_WARDS[0],
    ];
    for units in [&REPEATED[..], &REORDERED[..]] {
        let mut changed = banner;
        let UseSpec::Ward { ticks, .. } = banner.effect else {
            panic!("ward");
        };
        changed.effect = UseSpec::Ward { units, ticks };
        assert_ne!(item_digest(&changed), item_digest(&banner));
    }
    let observer = *item_def(ItemId::OBSERVER_WARD).expect("observer");
    let mut changed = observer;
    let UseSpec::Ward { ticks, .. } = observer.effect else {
        panic!("ward");
    };
    changed.effect = UseSpec::Ward {
        units: &crate::SENTRY_WARDS,
        ticks,
    };
    assert_ne!(
        item_digest(&changed),
        item_digest(&observer),
        "same-kind singleton selection still has identity"
    );
}

#[test]
fn a_ward_dependency_must_fit_inside_its_containing_catalogue_prefix() {
    let mut observer = *item_def(ItemId::OBSERVER_WARD).expect("observer");
    let UseSpec::Ward { ticks, .. } = observer.effect else {
        panic!("ward");
    };
    observer.effect = UseSpec::Ward {
        units: &crate::BANNER_WARDS,
        ticks,
    };
    assert!(!crate::item_table::ward_sources_precede_item(&observer));
    let mut sentry = *item_def(ItemId::SENTRY_WARD).expect("sentry");
    sentry.effect = UseSpec::Ward {
        units: &crate::OBSERVER_WARDS,
        ticks,
    };
    assert!(crate::item_table::ward_sources_precede_item(&sentry));
}

/// Every optional part of an item's digest opens with a tag of its own, so that two different
/// rows can never feed the digest the same words: no two parts share a tag (the Orb of Blight's
/// attack passive and a pick-up range once both opened with 0xA2).
#[test]
fn every_digest_tag_opens_one_part_alone() {
    let source = include_str!("../catalogue.rs");
    let mut tags: Vec<&str> = source
        .match_indices("u64(0x")
        .map(|(at, _)| {
            let tag = &source[at + 4..];
            &tag[..tag.find(')').expect("a closed call")]
        })
        .collect();
    assert!(tags.len() > 10, "the tags are found: {tags:?}");
    tags.sort_unstable();
    let shared: Vec<&&str> = tags
        .windows(2)
        .filter(|w| w[0] == w[1])
        .map(|w| &w[0])
        .collect();
    assert!(
        shared.is_empty(),
        "tags opening more than one part: {shared:?}"
    );
}
