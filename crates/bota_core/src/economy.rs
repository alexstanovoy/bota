//! What a death pays and what may be put out by its own side.

use bota_proto::{Attribute, Fixed, ItemId, UnitKind};

use crate::{ItemDef, Recipe, item_def, kind_def, rules};

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

/// How long a seat waits before its hero comes back, by its level.
#[inline]
pub fn respawn_wait(level: u8) -> u32 {
    let at = usize::from(level.max(1) - 1).min(rules::RESPAWN_SECONDS.len() - 1);
    rules::RESPAWN_SECONDS[at] * rules::TICKS_PER_SECOND
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

/// What each of `earners` heroes is paid in assist gold for a hero worth
/// `fallen_worth`, by Dota 7.40's `15 + (50 + 0.037 * worth) / earners`,
/// rounded down, times the net worth factor of the earning side.
///
/// Liquipedia (*Gold*, "Assists") gives the factor as the fallen side's net
/// worth over the earning side's, which grows without bound for a
/// far-behind earner; the Dota 2 wiki gives it piecewise, `1 - gap /
/// leading worth` for the leading side and `1 + gap / leading worth` for
/// the trailing one, between zero and two. The two agree for a side that
/// leads; this takes the bounded form for one that trails.
pub fn assist_gold(fallen_worth: i32, earners: usize, earning_side: i64, fallen_side: i64) -> i32 {
    if earners == 0 {
        return 0;
    }
    let earners = i64::try_from(earners).expect("seats are few");
    let base = rules::ASSIST_GOLD_FLAT_MILLI
        + (rules::ASSIST_GOLD_SHARED_MILLI
            + rules::ASSIST_GOLD_NET_WORTH_MILLI * i64::from(fallen_worth.max(0)))
            / earners;
    let (earning_side, fallen_side) = (earning_side.max(0), fallen_side.max(0));
    let leading = earning_side.max(fallen_side);
    let milli = if leading == 0 {
        base
    } else if earning_side >= fallen_side {
        base * fallen_side / leading
    } else {
        base * (2 * leading - earning_side) / leading
    };
    i32::try_from(milli / 1000).expect("assist gold fits an i32")
}

/// The experience a lane creep denied by its own side pays the other side.
#[inline]
pub fn denied_xp(bounty_xp: i32) -> i32 {
    bounty_xp * rules::DENIED_XP_PCT / 100
}

/// What buying `item` still takes, as the items to buy, given what the buyer
/// holds that counts: which stacks count (the buyer's own, not for sale) is
/// for the caller to say by passing only those.
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
    missing_parts_with(
        item,
        held,
        primary,
        |id| item_def(id).into_iter().flat_map(ItemDef::recipes),
        |id| item_def(id).map_or(0, |def| def.cost),
    )
}

/// [`missing_parts`] over another statement of the shop, such as the one a
/// match describes on the wire: `recipes_of` lists an item's recipes, its
/// components first and none for one bought whole, and `cost_of` its price.
pub fn missing_parts_with<'a, R>(
    item: ItemId,
    held: &[ItemId],
    primary: Option<Attribute>,
    recipes_of: impl Fn(ItemId) -> R,
    cost_of: impl Fn(ItemId) -> i32,
) -> Vec<ItemId>
where
    R: Iterator<Item = Recipe<'a>>,
{
    let first_parts = |id: ItemId| recipes_of(id).next().map_or(&[][..], |recipe| recipe.parts);
    let mut best: Option<(i32, bool, Vec<ItemId>)> = None;
    for recipe in recipes_of(item) {
        let mut spare = held.to_vec();
        let mut wanted = Vec::new();
        for part in recipe.parts {
            parts_beyond(*part, &mut spare, &mut wanted, &first_parts);
        }
        // Saturating: the costs may come from a peer's statement of the shop.
        let cost = wanted
            .iter()
            .fold(0i32, |sum, part| sum.saturating_add(cost_of(*part)));
        let own = recipe.mode.is_some() && recipe.mode == primary;
        let better = best
            .as_ref()
            .is_none_or(|(least, was_own, _)| cost < *least || (cost == *least && own && !was_own));
        if better {
            best = Some((cost, own, wanted));
        }
    }
    best.map_or_else(|| vec![item], |(_, _, wanted)| wanted)
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
