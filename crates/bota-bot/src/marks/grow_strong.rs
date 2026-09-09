//! Be worth as much as possible, wear it rather than sit on it, and drink it
//! when it is needed.
//!
//! Worth on a sliding count: gold at its face value, goods at half as much
//! again while they ride in the backpack, the stash or the courier's load,
//! and at twice what they cost once they sit in the working slots. Every step
//! a gold takes towards being worn pays: one mark for earning it, half the
//! price over for buying something with it, and the other half when the thing
//! is worn rather than carried.
//!
//! A consumable is worth what it does, not where it sits: counted at its
//! face value in any slot, so buying one moves nothing and hoarding one
//! grows nothing. What pays is the drinking — health mended on this seat's
//! hero is a mark a point and mana half a mark, read off the [`Healed`]
//! events. A salve drunk while four hundred is missing pays four hundred
//! against the hundred and ten it wipes; drunk at full health it pays
//! nothing and the salve is gone.
//!
//! Paid a tick at a time as the difference since the tick before, downwards
//! as well as up. Gold lost on dying is worth lost at its face value; the
//! goods already worn are not lost at all.
//!
//! One mark a gold of face value, so the number this lesson reports is what
//! the seat ended up worth on this counting less what it started with.
//!
//! [`Healed`]: bota_proto::EventKind::Healed

use bota_proto::EventKind;

use crate::{Carried, Field, Moment};

/// What one gold of worth is worth.
const A_GOLD: f32 = 1.0;
/// What a gold's cost of goods counts for, worn in the working slots.
const WORN: f32 = 2.0;
/// The same, riding in the backpack, the stash or the courier's load.
const RIDING: f32 = 1.5;
/// What one hit point mended on the seat's own hero is worth.
const A_MENDED_HP: f32 = 1.0;
/// What one point of mana mended on it is worth.
const A_MENDED_MANA: f32 = 0.5;

/// What this tick was worth to this lesson.
pub fn score(now: &Moment, carried: &mut Carried) -> f32 {
    let worth = counted(now.field);
    let mended = mended_on_me(now);
    let Some(was) = carried.was_worth.replace(worth) else {
        return mended;
    };
    A_GOLD * (worth - was) + mended
}

/// What the seat is worth on the sliding count.
///
/// Only the items the shop knows a price for. One it does not is left out
/// rather than guessed at, which undercounts by however much such an item cost.
fn counted(field: &Field) -> f32 {
    let (worn, riding) = crate::worth_worn_and_riding(field);
    let spent_ahead = crate::worth_of_consumables(field) as f32;
    let purse = field.seat.gold.unwrap_or(0) as f32;
    WORN * worn as f32 + RIDING * riding as f32 + spent_ahead + purse
}

/// What the health and mana mended on this seat's hero during the tick pay.
fn mended_on_me(now: &Moment) -> f32 {
    let Some(me) = now.field.me else {
        return 0.0;
    };
    now.events
        .iter()
        .map(|event| match event {
            EventKind::Healed {
                target,
                amount,
                mana,
                ..
            } if *target == me.id => A_MENDED_HP * *amount as f32 + A_MENDED_MANA * *mana as f32,
            _ => 0.0,
        })
        .sum()
}
