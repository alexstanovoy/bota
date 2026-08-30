//! Be worth as much as possible, and wear it rather than sit on it.
//!
//! Worth on a sliding count: gold at its face value, goods at half as much
//! again while they ride in the backpack, the stash or the courier's load,
//! and at twice what they cost once they sit in the working slots. Every step
//! a gold takes towards being worn pays: one mark for earning it, half the
//! price over for buying something with it, and the other half when the thing
//! is worn rather than carried.
//!
//! Paid a tick at a time as the difference since the tick before, downwards
//! as well as up. Gold lost on dying is worth lost at its face value; the
//! goods already worn are not lost at all.
//!
//! One mark a gold of face value, so the number this lesson reports is what
//! the seat ended up worth on this counting less what it started with.

use crate::{Carried, Field, Moment};

/// What one gold of worth is worth.
const A_GOLD: f32 = 1.0;
/// What a gold's cost of goods counts for, worn in the working slots.
const WORN: f32 = 2.0;
/// The same, riding in the backpack, the stash or the courier's load.
const RIDING: f32 = 1.5;

/// What this tick was worth to this lesson.
pub fn score(now: &Moment, carried: &mut Carried) -> f32 {
    let worth = counted(now.field);
    let Some(was) = carried.was_worth.replace(worth) else {
        return 0.0;
    };
    A_GOLD * (worth - was)
}

/// What the seat is worth on the sliding count.
///
/// Only the items the shop knows a price for. One it does not is left out
/// rather than guessed at, which undercounts by however much such an item cost.
fn counted(field: &Field) -> f32 {
    let (worn, riding) = crate::worth_worn_and_riding(field);
    let purse = field.seat.gold.unwrap_or(0) as f32;
    WORN * worn as f32 + RIDING * riding as f32 + purse
}
