//! Conservative resource reservations for orders already issued during a cast point.
//!
//! These scheduling guards protect pending orders; a false answer is not a server admission
//! refusal. Resources may change before a queued order executes, so even true is no guarantee.

use bota_core::Spends;
use bota_core::rules::MAX_QUEUED_ORDERS;
use bota_proto::{AbilityId, ItemSlot, ItemView, Order, UnitView};

use super::mana_affordability;
use super::pseudo_queue::OwedOrders;

/// A cast that goes off before what is given now, as it spends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spent {
    Ability(AbilityId),
    Item(ItemSlot),
}

/// What a body owes, as one snapshot of it and the seat's orders tell it.
pub struct Reservations {
    /// Whether the body stands in a cast point: an order to the body given now waits for it.
    casting: bool,
    /// The casts a cast given now waits behind: the one in hand, then those waiting, unless a
    /// move or the like waits, which the new cast replaces.
    ahead: [Option<Spent>; MAX_QUEUED_ORDERS + 1],
    /// The mana those spend.
    ahead_mana: i64,
    /// Whether the queue of casts is full: a cast given now is refused.
    queue_full: bool,
    /// The mana the cast in hand and the casts waiting will spend, which what goes off now
    /// must leave them.
    owed_mana: i64,
    /// The item slots the uses and puts waiting name. (The use in hand goes off with what its
    /// slot holds then, and no answer comes of it.)
    slots: u16,
    /// The slot of the item use in hand, when that use leaves it empty.
    emptied: Option<ItemSlot>,
}

impl Reservations {
    pub fn of(unit: &UnitView, owed: &OwedOrders) -> Self {
        let casting = unit.cast.is_some();
        let mut ahead = [None; MAX_QUEUED_ORDERS + 1];
        let mut ahead_mana = 0_i64;
        let mut emptied = None;
        if let Some(cast) = unit.cast {
            let in_hand = match cast.ability {
                Some(ability) => Some(Spent::Ability(ability)),
                None => owed.item_in_hand().map(Spent::Item),
            };
            ahead[0] = in_hand;
            ahead_mana = in_hand.map_or(0, |spent| mana_of(unit, spent));
            emptied = owed
                .item_in_hand()
                .filter(|&slot| item_in(unit, slot).is_some_and(use_empties));
        }
        let mut waiting_mana = 0_i64;
        let mut slots = 0_u16;
        for (index, order) in owed.waiting().enumerate().take(MAX_QUEUED_ORDERS) {
            if let Some(spent) = spent_by(unit, order.order) {
                waiting_mana += mana_of(unit, spent);
                if casting && owed.casts_waiting() {
                    ahead[index + 1] = Some(spent);
                }
            }
            if let Order::Use { slot, .. } | Order::Put { slot, .. } = order.order {
                slots |= 1_u16.checked_shl(u32::from(slot.0)).unwrap_or(0);
            }
        }
        let owed_mana = ahead_mana + waiting_mana;
        if casting && owed.casts_waiting() {
            ahead_mana += waiting_mana;
        }
        Self {
            casting,
            ahead,
            ahead_mana,
            queue_full: casting && owed.casts_waiting() && owed.len() >= MAX_QUEUED_ORDERS,
            owed_mana,
            slots,
            emptied,
        }
    }

    /// Whether `unit` may cast `ability` (ready by the view) for `cost` mana when its turn comes:
    /// an immediate one goes off now and must leave the mana of what the body owes; any other
    /// is neither ahead of itself nor short of the mana those ahead spend.
    pub fn allows_cast(&self, mana: i32, ability: AbilityId, cost: i32, immediate: bool) -> bool {
        if immediate {
            return affords_reserved(mana, self.owed_mana, cost);
        }
        !self.queue_full
            && !self.ahead.contains(&Some(Spent::Ability(ability)))
            && affords_reserved(mana, self.ahead_mana, cost)
    }

    /// Whether the item in `slot` (ready by the view) may be used when its turn comes: as
    /// [`Self::allows_cast`], and the uses of it ahead leave it, off cooldown, with a charge;
    /// one that wants mana missing waits only behind a cast that spends mana, which leaves
    /// more missing than the cast point regenerates.
    pub fn allows_use(&self, mana: i32, slot: ItemSlot, item: &ItemView) -> bool {
        let Some(def) = bota_core::item_def(item.id) else {
            return false;
        };
        if def.immediate {
            return affords_reserved(mana, self.owed_mana, item.mana_cost);
        }
        let uses_ahead = self
            .ahead
            .iter()
            .filter(|&&spent| spent == Some(Spent::Item(slot)))
            .count();
        let lasts = uses_ahead == 0
            || (def.cooldown == 0
                && !def.shared_wait
                && match def.spends {
                    Spends::Nothing => true,
                    Spends::One => item
                        .charges
                        .is_none_or(|charges| usize::from(charges) > uses_ahead),
                    Spends::All => false,
                });
        !self.queue_full
            && lasts
            && affords_reserved(mana, self.ahead_mana, item.mana_cost)
            && !(def.mana_deficit && self.casting && self.ahead_mana <= 0)
    }

    /// Whether the item in bag slot `slot` may be put down or handed over: an order to the
    /// body, it waits for the use in hand, which may use the last of it.
    pub fn allows_put(&self, slot: ItemSlot) -> bool {
        self.emptied != Some(slot)
    }

    /// Whether loot may be taken up: in a cast point the order would wait while a purchase,
    /// the stash or the courier may still fill the bag, so it is given once the body is free.
    pub const fn allows_take(&self) -> bool {
        !self.casting
    }

    /// Whether an order waiting names the item in `slot`, which a swap or a sale given now
    /// (they go through at once) would take from under it.
    pub fn names(&self, slot: usize) -> bool {
        u32::try_from(slot)
            .ok()
            .and_then(|shift| 1_u16.checked_shl(shift))
            .is_some_and(|mask| self.slots & mask != 0)
    }
}

fn spent_by(unit: &UnitView, order: Order) -> Option<Spent> {
    match order {
        Order::Cast { slot, .. } => unit
            .abilities
            .get(usize::from(slot.0))
            .map(|ability| Spent::Ability(ability.id)),
        Order::Use { slot, .. } => Some(Spent::Item(slot)),
        _ => None,
    }
}

fn mana_of(unit: &UnitView, spent: Spent) -> i64 {
    i64::from(match spent {
        Spent::Ability(id) => unit
            .abilities
            .iter()
            .take(bota_proto::limits::MAX_ABILITY_SLOTS)
            .find(|ability| ability.id == id)
            .map_or(0, |ability| ability.mana_cost),
        Spent::Item(slot) => item_in(unit, slot).map_or(0, |item| item.mana_cost),
    })
    .max(0)
}

pub fn item_in(unit: &UnitView, slot: ItemSlot) -> Option<&ItemView> {
    if slot == ItemSlot::TOWN_PORTAL {
        unit.town_portal.as_ref()
    } else {
        unit.items.get(usize::from(slot.0))?.as_ref()
    }
}

/// Whether one use of the item leaves nothing of it in its slot.
fn use_empties(item: &ItemView) -> bool {
    bota_core::item_def(item.id).is_some_and(|def| {
        def.cast_charges == 0
            && match def.spends {
                Spends::Nothing => false,
                Spends::One => item.charges.is_some_and(|charges| charges <= 1),
                Spends::All => true,
            }
    })
}

/// Whether the issue-time public mana budget covers this order after what is ahead of it.
pub fn order_mana_is_affordable(unit: &UnitView, owed: &OwedOrders, order: Order) -> bool {
    let owing = Reservations::of(unit, owed);
    match order {
        Order::Cast { slot, .. } => {
            unit.abilities
                .get(usize::from(slot.0))
                .is_some_and(|ability| {
                    let immediate =
                        bota_core::ability_def(ability.id).is_some_and(|def| def.immediate);
                    owing.allows_cast(unit.mana, ability.id, ability.mana_cost, immediate)
                })
        }
        Order::Use { slot, .. } => {
            item_in(unit, slot).is_some_and(|item| owing.allows_use(unit.mana, slot, item))
        }
        _ => true,
    }
}

fn affords_reserved(mana: i32, reserved: i64, cost: i32) -> bool {
    let remaining = i64::from(mana) - reserved;
    i32::try_from(remaining)
        .ok()
        .is_some_and(|mana| mana_affordability(mana, cost).is_legal())
}
