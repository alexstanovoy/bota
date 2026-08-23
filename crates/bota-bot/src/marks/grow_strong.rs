//! Be worth as much as possible, and wear it rather than sit on it.
//!
//! Net worth with the purse counted at half its face value: everything the seat
//! owns at what it cost — the bag, the stash and whatever the courier is
//! carrying — plus half of the gold it has not spent.
//!
//! So a gold is paid for twice over, half each time. Half when it is earned,
//! and the other half when it is turned into something: buying moves gold from
//! the side counted at a half to the side counted whole, which pays half of
//! what the item cost on the tick it is bought. Gold that is never spent is
//! never paid its second half.
//!
//! Paid a tick at a time as the difference since the tick before, downwards as
//! well as up. Gold lost on dying is worth lost, at half a mark a gold; the
//! goods it already bought are not lost at all.
//!
//! One mark a gold, so the number this lesson reports is what the seat ended up
//! worth on this counting less what it started with.

use crate::{Carried, Field, Moment};

/// What one gold of worth is worth.
const A_GOLD: f32 = 1.0;
/// What a gold still in the purse counts for, against one already spent.
const IN_THE_PURSE: f32 = 0.5;

/// What this tick was worth to this lesson.
pub fn score(now: &Moment, carried: &mut Carried) -> f32 {
    let worth = worn(now.field);
    let Some(was) = carried.was_worth.replace(worth) else {
        return 0.0;
    };
    A_GOLD * (worth - was)
}

/// What the seat is worth, with the purse counted at less than its face value.
///
/// Only the items the shop knows a price for. One it does not is left out
/// rather than guessed at, which undercounts by however much such an item cost.
fn worn(field: &Field) -> f32 {
    let goods = crate::worth_of_goods(field) as f32;
    let purse = field.seat.gold.unwrap_or(0) as f32;
    goods + IN_THE_PURSE * purse
}
