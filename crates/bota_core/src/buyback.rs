//! Buyback: what it costs, and when a seat may buy its dead hero back.

use bota_proto::RejectReason;

use crate::rules;

/// What a buyback costs a seat whose gold and owned items are worth
/// `net_worth` together: Dota's `200 + net worth / 13`, the division whole.
/// No level and no clock enters it.
#[inline]
#[must_use]
pub const fn buyback_cost(net_worth: i32) -> i32 {
    rules::BUYBACK_BASE_COST.saturating_add(net_worth / rules::BUYBACK_NET_WORTH_SHARE)
}

/// What one owned stack adds to the net worth a buyback is priced by. An item that comes with
/// charges and is gone with its last one (a consumable: a Tango, a scroll, a ward, a Raindrop)
/// is worth the share of its cost its paid charges are of those it comes with, so nothing when
/// empty and more than its cost when stacked; any other item is worth its whole cost at any
/// charge: one that comes with none (a Magic Wand), and one that stays when empty (a Bottle).
/// Established from the installed game, Dota 7.41f build 6975.
#[inline]
#[must_use]
pub fn stack_worth(def: &crate::ItemDef, charges: u8, free_charges: u8) -> i32 {
    if def.charges == 0 || def.kept_empty {
        return def.cost;
    }
    let paid = i64::from(charges.saturating_sub(free_charges));
    let worth = i64::from(def.cost) * paid / i64::from(def.charges);
    i32::try_from(worth).unwrap_or(i32::MAX)
}

/// Whether a seat may buy its hero back on tick `now`, by Dota's checks in
/// Dota's order: the hero lies dead, the seat holds the cost (exactly the
/// cost is enough), the cooldown is over (`ready_at` itself is), and the hero
/// is not rising by a reincarnation.
pub fn buyback_refusal(
    hero_dead: bool,
    gold: i32,
    cost: i32,
    now: u32,
    ready_at: u32,
    reincarnating: bool,
) -> Result<(), RejectReason> {
    if !hero_dead {
        return Err(RejectReason::HeroAlive);
    }
    if gold < cost {
        return Err(RejectReason::NotEnoughGold);
    }
    if ready_at > now {
        return Err(RejectReason::OnCooldown);
    }
    if reincarnating {
        return Err(RejectReason::HeroAlive);
    }
    Ok(())
}
