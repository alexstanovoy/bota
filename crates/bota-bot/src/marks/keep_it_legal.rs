//! Name nothing that cannot be done, and send nothing that will not be taken.
//!
//! Two things are counted, both of them a tick spent on nothing. A deed named
//! after being told it could not be done, which the seat loop drops. And an
//! order the server would not take, which is a place where what the bot
//! believes it may do and what the server enforces have come apart.
//!
//! One mark against it for each, so the number is how many ticks of the match
//! were thrown away. Nothing is ever paid: the mark is nought at best and
//! falls from there.
//!
//! A model cannot name a deed it was told it could not do — the mask is
//! applied before anything is compared — so the first count stands at nought
//! for anything playing by weights, and the second stands at nought while
//! [`allowed`](crate::allowed) and the server agree. That is what it is for.
//! A number that must stay at nought is read the moment it does not.

use crate::{Carried, Moment};

/// What a deed named against the flags costs.
const A_DEED_REFUSED: f32 = -1.0;
/// What an order the server would not take costs.
const AN_ORDER_REFUSED: f32 = -1.0;

/// What this tick was worth to this lesson.
pub fn score(now: &Moment, _carried: &mut Carried) -> f32 {
    A_DEED_REFUSED * f32::from(now.refused) + AN_ORDER_REFUSED * f32::from(now.rejected)
}
