//! What a death pays and what may be put out by its own side.

use bota_proto::{Attribute, Fixed, ItemId, SlotId, UnitKind};

use crate::{HeldItem, ITEMS, ItemDef, item_def, kind_def, rules};

/// Whether one of a side's own units is worn down far enough for that side
/// to put it out: below its kind's
/// [`deny_below_pct`](crate::KindDef::deny_below_pct) of what it can hold.
#[inline]
pub fn deniable(kind: UnitKind, hp: Fixed, max_hp: Fixed) -> bool {
    let Some(share) = kind_def(kind).deny_below_pct else {
        return false;
    };
    i64::from(hp.raw) * 100 < i64::from(max_hp.raw) * i64::from(share)
}

/// How long a seat waits before its hero comes back, by its level and by
/// whether neutral creeps brought it down: never less than
/// [`rules::NEUTRAL_KILL_RESPAWN_SECONDS`] then, as in Dota.
#[inline]
pub fn respawn_wait(level: u8, by_neutrals: bool) -> u32 {
    let at = usize::from(level.max(1) - 1).min(rules::RESPAWN_SECONDS.len() - 1);
    let seconds = if by_neutrals {
        rules::RESPAWN_SECONDS[at].max(rules::NEUTRAL_KILL_RESPAWN_SECONDS)
    } else {
        rules::RESPAWN_SECONDS[at]
    };
    seconds * rules::TICKS_PER_SECOND
}

/// Passive gold paid in all by `ticks` past the horn: each stretch of
/// [`rules::PASSIVE_GOLD_TENTHS_A_MINUTE`] at its own rate, whole gold only.
/// A tick pays what this grows by over it, so the pay is a function of the
/// clock alone and no remainder is carried.
pub fn passive_gold_by(ticks: u32) -> i32 {
    let minute = i64::from(60 * rules::TICKS_PER_SECOND);
    let steps = &rules::PASSIVE_GOLD_TENTHS_A_MINUTE;
    let mut tenth_ticks = 0;
    for (at, &(from, tenths)) in steps.iter().enumerate() {
        let start = i64::from(from) * minute;
        let end = steps
            .get(at + 1)
            .map_or(i64::MAX, |next| i64::from(next.0) * minute);
        let span = i64::from(ticks).min(end) - start;
        if span <= 0 {
            break;
        }
        tenth_ticks += tenths * span;
    }
    i32::try_from(tenth_ticks / (10 * minute)).expect("passive gold fits an i32")
}

/// What bringing down a hero pays in experience: a base, a share of what
/// the fallen had earned up to [`rules::HERO_KILL_XP_COUNTED_MAX`], and a
/// bonus for the streak its death ends.
#[inline]
pub fn hero_kill_xp(victim_xp: i32, streak: i32, level: i32) -> i32 {
    let streak = streak.min(rules::STREAK_XP_CAP);
    let streak_bonus = if streak < rules::STREAK_XP_FROM {
        0
    } else {
        (5 * streak * streak - 10 * streak + 40) * level / 4
    };
    let counted = victim_xp.min(rules::HERO_KILL_XP_COUNTED_MAX);
    rules::HERO_KILL_XP_BASE + counted * rules::HERO_KILL_XP_SHARE_PCT / 100 + streak_bonus
}

/// What the killer of a hero of `level` on a streak of `streak` is paid
/// besides assist gold: Dota's base, a price a level, the gold for ending
/// the streak, and [`rules::FIRST_BLOOD_GOLD`] on the match's first hero
/// kill.
#[inline]
pub fn hero_bounty(level: i32, streak: i32, first_blood: bool) -> i32 {
    let ended = streak.min(rules::STREAK_XP_CAP) - rules::STREAK_GOLD_FROM;
    let streak_gold = usize::try_from(ended).map_or(0, |at| rules::STREAK_GOLD[at]);
    let first = if first_blood {
        rules::FIRST_BLOOD_GOLD
    } else {
        0
    };
    rules::HERO_KILL_GOLD_BASE + rules::HERO_KILL_GOLD_PER_LEVEL * level + streak_gold + first
}

/// Assist gold per eligible hero: 7.40's base share, multiplied by the victim
/// team's net worth over the earning team's net worth [LIQ Gold, Assists].
/// Wide intermediates retain the ratio for a trailing team; results fit the
/// protocol's gold integer. When the earning team has no worth, retain the neutral factor.
pub fn assist_gold(fallen_worth: i32, earners: usize, earning_side: i64, fallen_side: i64) -> i32 {
    if earners == 0 {
        return 0;
    }
    let earners = i128::try_from(earners).expect("seats are few");
    let (earning_side, fallen_side) = if earning_side <= 0 {
        (1, 1)
    } else {
        (earning_side, fallen_side.max(0))
    };
    let numerator = i128::from(fallen_side);
    // Scale before splitting: truncating the unscaled share first can lose
    // a whole gold after the trailing team's factor is applied.
    let shared = (i128::from(rules::ASSIST_GOLD_SHARED_MILLI)
        + i128::from(rules::ASSIST_GOLD_NET_WORTH_MILLI) * i128::from(fallen_worth.max(0)))
        * numerator
        / earners;
    let gold = (i128::from(rules::ASSIST_GOLD_FLAT_MILLI) * numerator + shared)
        / i128::from(earning_side.max(1))
        / 1000;
    gold.clamp(0, i128::from(i32::MAX)) as i32
}

/// The experience a lane creep denied by its own side pays the other side.
#[inline]
pub fn denied_xp(bounty_xp: i32) -> i32 {
    bounty_xp * rules::DENIED_XP_PCT / 100
}

/// What the shop sells under `id`: the item, unless it is retired or cannot be bought. The one
/// statement of what is sold, which the server buys by, every client lists by, and the wire's
/// shop is derived from.
#[inline]
pub fn sold(id: ItemId) -> Option<&'static ItemDef> {
    item_def(id).filter(|def| !def.unbuyable)
}

/// Whether a shelf can supply a quantity; no shelf means unlimited stock.
#[must_use]
pub fn can_take_from_stock(left: Option<u8>, count: usize) -> bool {
    left.is_none_or(|left| count <= usize::from(left))
}

/// Each occurrence of a purchased component consumes one from its own shelf.
#[must_use]
pub fn purchase_stock_available(parts: &[ItemId], left: impl Fn(ItemId) -> Option<u8>) -> bool {
    parts.iter().all(|part| {
        can_take_from_stock(
            left(*part),
            parts.iter().filter(|other| *other == part).count(),
        )
    })
}

/// Every item the shop sells, in id order.
pub fn sold_items() -> impl Iterator<Item = ItemId> {
    (0..ITEMS.len() as u16)
        .map(ItemId)
        .filter(|id| sold(*id).is_some())
}

/// Which stacks count toward a purchase, as the items to pass to [`missing_parts`]: the buyer's
/// own, not marked for sale and not locked against combining (a locked part is no part of a
/// build, so the build is bought whole), among all `held` that the buyer's purchase counts. As in Dota, a
/// purchase counts everything the player owns that it can reach: the hero's bag (its kit while it
/// lies dead), the stash and the courier, and never the town portal slot. The caller says which
/// containers by what it passes; the rule of what counts is this one.
pub fn purchase_pool<'a, P: HeldItem + 'a>(
    buyer: SlotId,
    held: impl IntoIterator<Item = &'a P>,
) -> Vec<ItemId> {
    held.into_iter()
        .filter(|stack| stack.owner() == buyer && !stack.for_sale() && !stack.locked())
        .map(HeldItem::id)
        .collect()
}

/// What buying `item` still takes, as the items to buy, given what the buyer
/// holds that counts ([`purchase_pool`] says which stacks do).
///
/// An item without parts is itself the answer, so asking for a second one buys
/// a second one. One that is built answers with the parts it lacks, each of
/// those asked the same question in turn; a part already held is spent on the
/// build and so is bought no second time.
///
/// Of an item's several recipes the one that costs least given what is held
/// is bought, and among equals the one set to the buyer's `primary`
/// attribute, then the first: Dota's quick-buy "evaluating which of the 3
/// recipes would be the cheapest", its components "based on the hero's
/// primary attribute" (Liquipedia, *Power Treads*). A part's own parts are
/// its first recipe's.
pub fn missing_parts(item: ItemId, held: &[ItemId], primary: Option<Attribute>) -> Vec<ItemId> {
    purchase_requirements(item, held, primary).wanted
}

/// The existing recipe evaluator's missing components and unconsumed ownership pool.
#[must_use]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurchaseRequirements {
    pub wanted: Vec<ItemId>,
    pub remaining_owned: Vec<ItemId>,
}

pub fn purchase_requirements(
    item: ItemId,
    held: &[ItemId],
    primary: Option<Attribute>,
) -> PurchaseRequirements {
    let recipes_of = |id: ItemId| item_def(id).into_iter().flat_map(ItemDef::recipes);
    let first_parts = |id: ItemId| recipes_of(id).next().map_or(&[][..], |recipe| recipe.parts);
    let cost_of = |id: ItemId| item_def(id).map_or(0, |def| def.cost);
    let mut best: Option<(i32, bool, PurchaseRequirements)> = None;
    for recipe in recipes_of(item) {
        let mut spare = held.to_vec();
        let mut wanted = Vec::new();
        for part in recipe.parts {
            parts_beyond(*part, &mut spare, &mut wanted, &first_parts);
        }
        let cost = wanted
            .iter()
            .fold(0i32, |sum, part| sum.saturating_add(cost_of(*part)));
        let own = recipe.mode.is_some() && recipe.mode == primary;
        let better = best
            .as_ref()
            .is_none_or(|(least, was_own, _)| cost < *least || (cost == *least && own && !was_own));
        if better {
            best = Some((
                cost,
                own,
                PurchaseRequirements {
                    wanted,
                    remaining_owned: spare,
                },
            ));
        }
    }
    best.map_or_else(
        || PurchaseRequirements {
            wanted: vec![item],
            remaining_owned: held.to_vec(),
        },
        |(_, _, result)| result,
    )
}

/// Lays out what one part still costs, spending `held` as it goes.
fn parts_beyond<'a>(
    item: ItemId,
    held: &mut Vec<ItemId>,
    wanted: &mut Vec<ItemId>,
    parts_of: &impl Fn(ItemId) -> &'a [ItemId],
) {
    if let Some(at) = held.iter().position(|id| *id == item) {
        held.remove(at);
        return;
    }
    match parts_of(item) {
        [] => wanted.push(item),
        parts => {
            for part in parts {
                parts_beyond(*part, held, wanted, parts_of);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn assist_gold_zero_earning_worth_uses_a_neutral_multiplier() {
        assert_eq!(super::assist_gold(4000, 2, 0, 4000), 114);
        assert_eq!(super::assist_gold(4000, 2, -1000, 4000), 114);
        assert_eq!(super::assist_gold(i32::MAX, 1, 0, i64::MAX), 79_456_959);
    }

    #[test]
    fn assist_gold_rounds_only_after_scaling_the_complete_share() {
        assert_eq!(super::assist_gold(2000, 3, 1000, 3000), 169);
    }

    #[test]
    fn assist_gold_ratio_uses_wide_arithmetic_and_a_bounded_result() {
        assert_eq!(super::assist_gold(i32::MAX, 1, 1, i64::MAX), i32::MAX);
        assert_eq!(super::assist_gold(4000, 0, 0, 0), 0);
        assert_eq!(super::assist_gold(4000, 2, 0, 0), 114);
    }

    #[test]
    fn assist_gold_uses_the_victim_to_earner_team_worth_ratio() {
        assert_eq!(super::assist_gold(4000, 2, 2000, 4000), 228);
        assert_eq!(super::assist_gold(4000, 2, 1000, 4000), 456);
        assert_eq!(super::assist_gold(4000, 2, 4000, 2000), 57);
    }

    use bota_proto::ItemId;

    use crate::missing_parts;

    #[test]
    fn a_held_part_is_spent_once_on_the_build() {
        let held = [ItemId::BOOTS];
        assert_eq!(
            missing_parts(ItemId::PHASE_BOOTS, &held, None),
            [ItemId::CHAINMAIL, ItemId::BLADES_OF_ATTACK]
        );
        assert_eq!(missing_parts(ItemId::BOOTS, &held, None), [ItemId::BOOTS]);
    }

    #[test]
    fn only_the_parts_not_held_are_bought() {
        let held = [ItemId::CIRCLET];
        let wanted = missing_parts(ItemId::BRACER, &held, None);
        assert_eq!(wanted, [ItemId::GAUNTLETS, ItemId::RECIPE_BRACER]);
    }
}

#[cfg(test)]
mod purchase_stock_tests {
    use super::*;
    #[test]
    fn f07_review_stock_counts_each_occurrence_and_unlimited_shelves() {
        assert!(can_take_from_stock(None, usize::MAX));
        assert!(can_take_from_stock(Some(0), 0));
        assert!(!can_take_from_stock(Some(0), 1));
        let parts = [ItemId::OBSERVER_WARD, ItemId::OBSERVER_WARD, ItemId::BOOTS];
        assert!(!purchase_stock_available(&parts, |id| (id
            == ItemId::OBSERVER_WARD)
            .then_some(1)));
        assert!(purchase_stock_available(&parts, |id| (id
            == ItemId::OBSERVER_WARD)
            .then_some(2)));
    }
}
