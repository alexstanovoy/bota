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
    purchase_pool_iter(buyer, held).collect()
}

/// The allocation-free ownership filter used by [`purchase_pool`].
pub fn purchase_pool_iter<'a, P: HeldItem + 'a>(
    buyer: SlotId,
    held: impl IntoIterator<Item = &'a P>,
) -> impl Iterator<Item = ItemId> {
    held.into_iter()
        .filter(move |stack| stack.owner() == buyer && !stack.for_sale() && !stack.locked())
        .map(HeldItem::id)
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

/// What a buy order of `item` buys, given what the buyer holds that counts ([`purchase_pool`]).
///
/// [`missing_parts`] says what an item still lacks to be built. A buy order asks for one more copy,
/// so it differs where the buyer already holds every part of a copy that has not been built: the
/// parts wait in places that do not meet (a bag far from its shop, the stash, a courier), and
/// nothing builds until they do. Such a complete set is a copy already bought, and is set aside;
/// what is held beyond it counts toward the next copy, whose other parts are bought. A buyer
/// holding nothing beyond the sets (nothing toward one more copy) is told there is nothing to buy
/// rather than charged for a whole copy it did not ask a second time.
pub fn purchase_parts(item: ItemId, held: &[ItemId], primary: Option<Attribute>) -> Vec<ItemId> {
    evaluate_buy::<Vec<ItemId>, Vec<ItemId>>(item, held.to_vec(), primary)
        .unwrap_or_else(|never| match never {})
}

/// The rule of [`purchase_parts`] over any buffers, so that the allocating and the fixed-capacity
/// callers (the server, the client's order legality) are one implementation.
fn evaluate_buy<P: RecipeItems, W: RecipeItems>(
    item: ItemId,
    held: P,
    primary: Option<Attribute>,
) -> Result<W, W::Error> {
    let mut pool = held;
    let mut set_aside = false;
    // Each set aside leaves fewer items in the pool, so the run ends.
    for _ in 0..=pool.items().len() {
        let (wanted, spare) = evaluate_purchase::<P, W>(item, pool.clone(), primary)?;
        if wanted.items().is_empty() {
            pool = spare;
            set_aside = true;
            continue;
        }
        let toward_next_copy = spare.items().len() < pool.items().len();
        return Ok(if set_aside && !toward_next_copy {
            W::default()
        } else {
            wanted
        });
    }
    Ok(W::default())
}

/// The existing recipe evaluator's missing components and unconsumed ownership pool.
#[must_use]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurchaseRequirements {
    pub wanted: Vec<ItemId>,
    pub remaining_owned: Vec<ItemId>,
}

/// Maximum nodes of a catalogue recipe expansion, including intermediate builds.
pub const MAX_PURCHASE_TREE: usize = 64;

pub fn purchase_requirements(
    item: ItemId,
    held: &[ItemId],
    primary: Option<Attribute>,
) -> PurchaseRequirements {
    let (wanted, remaining_owned) =
        evaluate_purchase::<Vec<ItemId>, Vec<ItemId>>(item, held.to_vec(), primary)
            .unwrap_or_else(|never| match never {});
    PurchaseRequirements {
        wanted,
        remaining_owned,
    }
}

/// [`purchase_parts`] in fixed capacity, for clients that cannot allocate per order.
/// Nothing when the caller's ownership buffer or the catalogue expansion exceeds its bound.
pub fn bounded_purchase_parts<const HELD: usize>(
    item: ItemId,
    held: &[ItemId],
    primary: Option<Attribute>,
) -> Option<BoundedPurchaseParts> {
    let mut owned = RecipeBuffer::<HELD>::default();
    for &item in held {
        RecipeItems::push(&mut owned, item).ok()?;
    }
    let wanted = evaluate_buy::<_, RecipeBuffer<MAX_PURCHASE_TREE>>(item, owned, primary).ok()?;
    Some(BoundedPurchaseParts { wanted })
}

/// What a buy order buys, in the order of [`purchase_parts`].
pub struct BoundedPurchaseParts {
    wanted: RecipeBuffer<MAX_PURCHASE_TREE>,
}

impl BoundedPurchaseParts {
    pub fn wanted(&self) -> &[ItemId] {
        self.wanted.items()
    }
}

trait RecipeItems: Clone + Default {
    type Error;
    fn items(&self) -> &[ItemId];
    fn take(&mut self, item: ItemId) -> bool;
    fn push(&mut self, item: ItemId) -> Result<(), Self::Error>;
}

impl RecipeItems for Vec<ItemId> {
    type Error = std::convert::Infallible;
    fn items(&self) -> &[ItemId] {
        self
    }
    fn take(&mut self, item: ItemId) -> bool {
        if let Some(at) = self.iter().position(|id| *id == item) {
            self.remove(at);
            true
        } else {
            false
        }
    }
    fn push(&mut self, item: ItemId) -> Result<(), Self::Error> {
        Vec::push(self, item);
        Ok(())
    }
}

#[derive(Clone)]
struct RecipeBuffer<const N: usize> {
    items: [ItemId; N],
    length: usize,
}

impl<const N: usize> Default for RecipeBuffer<N> {
    fn default() -> Self {
        Self {
            items: [ItemId(0); N],
            length: 0,
        }
    }
}

impl<const N: usize> RecipeItems for RecipeBuffer<N> {
    type Error = ();
    fn items(&self) -> &[ItemId] {
        &self.items[..self.length]
    }
    fn take(&mut self, item: ItemId) -> bool {
        let Some(at) = self.items().iter().position(|id| *id == item) else {
            return false;
        };
        self.items.copy_within(at + 1..self.length, at);
        self.length -= 1;
        true
    }
    fn push(&mut self, item: ItemId) -> Result<(), Self::Error> {
        let place = self.items.get_mut(self.length).ok_or(())?;
        *place = item;
        self.length += 1;
        Ok(())
    }
}

fn evaluate_purchase<P: RecipeItems, W: RecipeItems>(
    item: ItemId,
    held: P,
    primary: Option<Attribute>,
) -> Result<(W, P), W::Error> {
    let mut best: Option<(i32, bool, W, P)> = None;
    for recipe in item_def(item).into_iter().flat_map(ItemDef::recipes) {
        let mut spare = held.clone();
        let mut wanted = W::default();
        for &part in recipe.parts {
            parts_beyond(part, &mut spare, &mut wanted)?;
        }
        let cost = wanted.items().iter().fold(0_i32, |sum, part| {
            sum.saturating_add(item_def(*part).map_or(0, |def| def.cost))
        });
        let own = recipe.mode.is_some() && recipe.mode == primary;
        if best.as_ref().is_none_or(|(least, was_own, _, _)| {
            cost < *least || (cost == *least && own && !was_own)
        }) {
            best = Some((cost, own, wanted, spare));
        }
    }
    match best {
        Some((_, _, wanted, spare)) => Ok((wanted, spare)),
        None => {
            let mut wanted = W::default();
            RecipeItems::push(&mut wanted, item)?;
            Ok((wanted, held))
        }
    }
}

/// Depth-first traversal of the compile-time checked catalogue. Invalid wire ids are leaves.
fn parts_beyond<P: RecipeItems, W: RecipeItems>(
    item: ItemId,
    held: &mut P,
    wanted: &mut W,
) -> Result<(), W::Error> {
    let mut pending = [ItemId(0); MAX_PURCHASE_TREE];
    pending[0] = item;
    let mut length = 1;
    for _ in 0..MAX_PURCHASE_TREE {
        if length == 0 {
            break;
        }
        length -= 1;
        let part = pending[length];
        if held.take(part) {
            continue;
        }
        let children = item_def(part).map_or(&[][..], first_recipe);
        if children.is_empty() {
            RecipeItems::push(wanted, part)?;
        } else {
            for &child in children.iter().rev() {
                pending[length] = child;
                length += 1;
            }
        }
    }
    Ok(())
}

const fn first_recipe(def: &ItemDef) -> &'static [ItemId] {
    if !def.components.is_empty() {
        def.components
    } else if !def.alternatives.is_empty() {
        def.alternatives[0].parts
    } else if !def.charge_recipes.is_empty() {
        def.charge_recipes[0]
    } else {
        &[]
    }
}

// Every alternative is bounded, including charge recipes and cycles. Descendants use
// their first recipe, as the planner does. Both pending nodes and output leaves fit.
const _: () = {
    let mut row = 0;
    while row < ITEMS.len() {
        let def = &ITEMS[row];
        assert!(recipe_fits(def.components));
        let mut at = 0;
        while at < def.alternatives.len() {
            assert!(recipe_fits(def.alternatives[at].parts));
            at += 1;
        }
        at = 0;
        while at < def.charge_recipes.len() {
            assert!(recipe_fits(def.charge_recipes[at]));
            at += 1;
        }
        row += 1;
    }
};

const fn recipe_fits(parts: &[ItemId]) -> bool {
    let mut pending = [ItemId(0); MAX_PURCHASE_TREE];
    if parts.len() > MAX_PURCHASE_TREE {
        return false;
    }
    let mut length = parts.len();
    let mut at = 0;
    while at < length {
        pending[at] = parts[at];
        at += 1;
    }
    let mut visited = 0;
    while length > 0 && visited < MAX_PURCHASE_TREE {
        length -= 1;
        let id = pending[length].0 as usize;
        if id >= ITEMS.len() {
            return false;
        }
        let children = first_recipe(&ITEMS[id]);
        at = 0;
        while at < children.len() {
            if length == MAX_PURCHASE_TREE {
                return false;
            }
            pending[length] = children[at];
            length += 1;
            at += 1;
        }
        visited += 1;
    }
    length == 0
}

#[cfg(test)]
mod tests {
    #[test]
    fn order_legality_recipe_buffer_covers_all_rows_and_consumes_each_held_part_once() {
        use bota_proto::ItemId;
        for def in &crate::ITEMS {
            if def.retired {
                continue;
            }
            assert!(
                super::bounded_purchase_parts::<0>(def.id, &[], None).is_some(),
                "{:?}",
                def.id
            );
        }
        let plan = super::bounded_purchase_parts::<1>(ItemId::BRACER, &[ItemId::GAUNTLETS], None)
            .expect("bounded recipe");
        assert_eq!(plan.wanted(), &[ItemId::CIRCLET, ItemId::RECIPE_BRACER]);
        assert!(
            super::bounded_purchase_parts::<0>(ItemId::BRACER, &[ItemId::GAUNTLETS], None)
                .is_none()
        );
    }

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
