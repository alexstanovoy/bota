//! Economic events trusted local match setup schedules: at a tick, a seat's
//! gold changes and items are handed to it, like a practice scenario's setup.
//!
//! Only a match its host builds in-process carries them (a
//! [`LocalMatch`](crate::local::LocalMatch) or a test): the served match's
//! setup has no way to name one, nothing on the wire tells of one, and the
//! items and gold reach the seat through the same machinery as the shop's,
//! so the match stays as deterministic as one without.

use bota_proto::{ItemId, SlotId};

use std::collections::VecDeque;

use crate::engine::{Fnv, StateHash, state_hash};

/// The most events one match setup may carry.
pub const MAX_ECONOMY_EVENTS: usize = 64;
/// The most items one event may hand out.
pub const MAX_ECONOMY_EVENT_ITEMS: usize = 6;
/// The largest change of gold one event may make, either way: Dota's
/// unreliable-gold cap.
pub const MAX_ECONOMY_EVENT_GOLD: i32 = 99_999;
/// The latest tick an event may name: two hours of play.
pub const MAX_ECONOMY_EVENT_TICK: u32 = 2 * 60 * 60 * bota_core::rules::TICKS_PER_SECOND;

/// One seat's change of fortune at one tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EconomyEvent {
    /// Whose.
    pub slot: SlotId,
    /// The tick it happens at, before that tick's orders; 0 is the first
    /// tick the match runs.
    pub tick: u32,
    /// Gold put into the seat's hand; a negative amount takes gold away,
    /// never below nothing.
    pub gold: i32,
    /// Items handed to the hero's bag, or the stash when the bag has no
    /// room; one the stash has no room for either is not handed out.
    pub items: Vec<ItemId>,
}

/// Why an event is not one a match setup may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EconomyEventError {
    /// It names a seat the match has no pick for.
    UnknownSeat,
    /// It happens past [`MAX_ECONOMY_EVENT_TICK`].
    TooLate,
    /// Its gold is beyond [`MAX_ECONOMY_EVENT_GOLD`] either way.
    TooMuchGold,
    /// It hands out more than [`MAX_ECONOMY_EVENT_ITEMS`] items.
    TooManyItems,
    /// It hands out an item the match's shop does not sell.
    NotSold { item: ItemId },
}

impl core::fmt::Display for EconomyEventError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EconomyEventError::UnknownSeat => {
                write!(f, "it names a seat the match has no pick for")
            }
            EconomyEventError::TooLate => {
                write!(f, "it happens past tick {MAX_ECONOMY_EVENT_TICK}")
            }
            EconomyEventError::TooMuchGold => {
                write!(f, "its gold is beyond {MAX_ECONOMY_EVENT_GOLD} either way")
            }
            EconomyEventError::TooManyItems => {
                write!(f, "it hands out more than {MAX_ECONOMY_EVENT_ITEMS} items")
            }
            EconomyEventError::NotSold { item } => {
                write!(f, "the match's shop does not sell item {}", item.0)
            }
        }
    }
}

impl std::error::Error for EconomyEventError {}

/// Why an event is not one a match on `catalogue` with seats `slots` may
/// carry, or nothing when it is.
pub fn check_economy_event(
    event: &EconomyEvent,
    slots: &[SlotId],
    catalogue: bota_core::Catalogue,
) -> Result<(), EconomyEventError> {
    if !slots.contains(&event.slot) {
        return Err(EconomyEventError::UnknownSeat);
    }
    if event.tick > MAX_ECONOMY_EVENT_TICK {
        return Err(EconomyEventError::TooLate);
    }
    if !(-MAX_ECONOMY_EVENT_GOLD..=MAX_ECONOMY_EVENT_GOLD).contains(&event.gold) {
        return Err(EconomyEventError::TooMuchGold);
    }
    if event.items.len() > MAX_ECONOMY_EVENT_ITEMS {
        return Err(EconomyEventError::TooManyItems);
    }
    if let Some(&item) = event
        .items
        .iter()
        .find(|item| !catalogue.sold_items().any(|sold| sold == **item))
    {
        return Err(EconomyEventError::NotSold { item });
    }
    Ok(())
}

/// The economic events of a match still to come, in tick order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EconomySchedule(pub VecDeque<EconomyEvent>);

/// An empty schedule adds nothing to the fingerprint, so a match without
/// economic events hashes as it did before they existed.
impl StateHash for EconomySchedule {
    fn state_hash(&self, fnv: &mut Fnv) {
        if !self.0.is_empty() {
            self.0.state_hash(fnv);
        }
    }
}

#[cfg(test)]
impl crate::engine::Specimen for EconomySchedule {
    fn low() -> Self {
        EconomySchedule::default()
    }

    fn high() -> Self {
        EconomySchedule(VecDeque::from([
            <EconomyEvent as crate::engine::Specimen>::high(),
        ]))
    }
}

state_hash!(EconomyEvent {
    slot,
    tick,
    gold,
    items
});
