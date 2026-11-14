//! How the sources of a carried stat that does not all stack combine: by lineage, by the stacking
//! groups Dota's notes name, and across sources by each stat's own rule.

use bota_proto::ItemId;

use crate::item::{STACKING_STATS, Source, StackingStat};
use crate::{Carried, CarriedItem, carried_bonus};

/// The stacking rule of the stat `get` reads.
fn rule_of(get: fn(&Carried) -> i32) -> &'static StackingStat {
    let probe = Carried {
        cast_range_bonus: 1,
        cooldown_cut_bp: 2,
        ..Carried::default()
    };
    STACKING_STATS
        .iter()
        .find(|stat| (stat.get)(&probe) == get(&probe))
        .expect("a stacking stat")
}

fn carried(items: &[ItemId]) -> Carried {
    carried_bonus(
        items.iter().map(|&id| CarriedItem {
            id,
            passive: true,
            charges: 0,
            mode: None,
        }),
        false,
    )
}

/// Aether Lens and Dagon share one cast range bonus (Dota 7.41, Dagon: "Cast Range Bonus does
/// not stack with Aether Lens or multiple Dagons"): the lens is in that group, so two lenses carry
/// one bonus, while a source outside every group (a neutral enhancement's, Dota's Keen-eyed +125)
/// adds to it.
#[test]
fn the_aether_lens_shares_a_cast_range_group_and_other_sources_add() {
    let rule = rule_of(|carried| carried.cast_range_bonus);
    assert_eq!(rule.source(ItemId::AETHER_LENS, false), Source::Group(0));
    let lens = carried(&[ItemId::AETHER_LENS]).cast_range_bonus;
    assert_eq!(lens, 225);
    assert_eq!(
        carried(&[ItemId::AETHER_LENS, ItemId::AETHER_LENS]).cast_range_bonus,
        225
    );
    assert_eq!((rule.stack)(lens, 125), 350, "another source adds");
}

/// Cooldown reductions of different sources multiply (Dota 7.31: "Cooldown Reduction stacking is
/// now multiplicative"): Octarine Core's 25% and Dota's Feverish 15% leave 0.75 × 0.85 = 63.75%
/// of a cooldown, a cut of 36.25%; two Octarine Cores cut 25% once.
#[test]
fn cooldown_cuts_of_different_sources_multiply_and_one_lineage_counts_once() {
    let rule = rule_of(|carried| carried.cooldown_cut_bp);
    let core = carried(&[ItemId::OCTARINE_CORE]).cooldown_cut_bp;
    assert_eq!(core, 2_500);
    assert_eq!(
        carried(&[ItemId::OCTARINE_CORE, ItemId::OCTARINE_CORE]).cooldown_cut_bp,
        2_500
    );
    assert_eq!((rule.stack)(core, 1_500), 3_625);
}
