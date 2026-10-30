//! Where a purchase goes: the one rule of stacking and placing, which the server buys by and a
//! client asks before it sends a buy order.
//!
//! A stack is read through [`HeldItem`], so the server's stacks and a view's items answer the
//! same questions. A bought item goes to the hero's bag when the hero stands in its shop, and to
//! the stash otherwise (or when the bag is full); mergeable charges fill a compatible stack first.

use bota_proto::{Attribute, ItemId, SlotId};

use crate::{item_def, rules};

/// What the rules of stacking and placing ask of a stack, wherever it is kept.
pub trait HeldItem {
    /// Which item it is. A Ward Dispenser is its own item.
    fn id(&self) -> ItemId;
    /// Its charges; of a Ward Dispenser, its Observer Wards.
    fn charges(&self) -> u8;
    /// The Sentry Wards a Ward Dispenser holds beside its charges. Zero for any other item.
    fn sentries(&self) -> u8;
    /// The seat that bought it.
    fn owner(&self) -> SlotId;
    /// The attribute it is set to.
    fn mode(&self) -> Option<Attribute>;
    /// Whether it is marked to be sold when it next reaches the shop.
    fn for_sale(&self) -> bool;
    /// Whether it is locked against combining.
    fn locked(&self) -> bool;
}

/// How many Observer Wards and Sentry Wards a stack of the ward family holds: the two wards and
/// the Ward Dispenser they make. Nothing for any other item.
pub fn ward_counts(id: ItemId, charges: u8, sentries: u8) -> Option<(u8, u8)> {
    match id {
        ItemId::OBSERVER_WARD => Some((charges, 0)),
        ItemId::SENTRY_WARD => Some((0, charges)),
        ItemId::WARD_DISPENSER => Some((charges, sentries)),
        _ => None,
    }
}

/// Charges `incoming` may add to `held`; ownership, mode, and sale marks must match.
///
/// Of one kind that stacks, up to its cap. With `combine`, the two wards and the Ward Dispenser
/// also merge into one dispenser, whole or not at all, as far as neither count passes a byte:
/// `u8::MAX` when the whole of the source fits. A stash builds nothing, so it passes false.
pub fn merge_room(held: &impl HeldItem, incoming: &impl HeldItem, combine: bool) -> u8 {
    if held.owner() != incoming.owner()
        || held.mode() != incoming.mode()
        || held.for_sale() != incoming.for_sale()
        || held.charges() == 0
        || incoming.charges() == 0
    {
        return 0;
    }
    let limit = item_def(held.id()).map_or(0, |def| def.stack_limit);
    if held.id() == incoming.id() && limit > 0 {
        debug_assert!(held.charges() <= limit && incoming.charges() <= limit);
        return limit.saturating_sub(held.charges());
    }
    let (Some((o1, s1)), Some((o2, s2))) = (
        ward_counts(held.id(), held.charges(), held.sentries()),
        ward_counts(incoming.id(), incoming.charges(), incoming.sentries()),
    ) else {
        return 0;
    };
    let fits = o1.checked_add(o2).is_some() && s1.checked_add(s2).is_some();
    if combine && fits { u8::MAX } else { 0 }
}

/// One container of stacks, as far as placing a stack in it goes.
pub struct Shelf<'a, P> {
    /// The slots, in slot order.
    pub slots: &'a [Option<P>],
    /// A hero's town portal slot, numbered [`rules::TOWN_PORTAL_SLOT`]: what it holds. Absent for
    /// a container that has none (a courier's, the stash).
    pub town_portal: Option<&'a Option<P>>,
    /// Whether the wards in it come together into a Ward Dispenser: false for the stash, which
    /// builds nothing.
    pub combines: bool,
}

impl<P: HeldItem> Shelf<'_, P> {
    /// Where a whole arriving stack would go: a scroll into the town portal slot when there is
    /// one, to merge or to fill it; anything else into the first compatible stack it fits whole,
    /// then the first empty slot. The slot's number, [`rules::TOWN_PORTAL_SLOT`] for the town
    /// portal slot.
    pub fn receiving_slot(&self, incoming: &impl HeldItem) -> Option<usize> {
        if incoming.id() == ItemId::TOWN_PORTAL_SCROLL
            && let Some(held) = self.town_portal
        {
            let fits = held
                .as_ref()
                .is_none_or(|held| merge_room(held, incoming, false) >= incoming.charges());
            return fits.then_some(rules::TOWN_PORTAL_SLOT);
        }
        let merge = self.slots.iter().position(|held| {
            held.as_ref().is_some_and(|held| {
                incoming.charges() > 0
                    && merge_room(held, incoming, self.combines) >= incoming.charges()
            })
        });
        merge.or_else(|| self.slots.iter().position(Option::is_none))
    }
}

/// Where one bought part goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Berth {
    /// The hero's bag, by slot number.
    Bag(usize),
    /// The stash, by slot number.
    Stash(usize),
}

impl HeldItem for bota_proto::ItemView {
    fn id(&self) -> ItemId {
        self.id
    }

    fn charges(&self) -> u8 {
        self.dispenser
            .map_or(self.charges.unwrap_or(0), |dispenser| dispenser.observers)
    }

    fn sentries(&self) -> u8 {
        self.dispenser.map_or(0, |dispenser| dispenser.sentries)
    }

    fn owner(&self) -> SlotId {
        self.owner
    }

    fn mode(&self) -> Option<Attribute> {
        self.mode
    }

    fn for_sale(&self) -> bool {
        self.selling == Some(true)
    }

    fn locked(&self) -> bool {
        self.locked == Some(true)
    }
}

/// A part as it is when bought: whole, for nobody to sell, set to the attribute its item is set
/// to.
struct Bought {
    id: ItemId,
    charges: u8,
    owner: SlotId,
    mode: Option<Attribute>,
}

impl HeldItem for Bought {
    fn id(&self) -> ItemId {
        self.id
    }
    fn charges(&self) -> u8 {
        self.charges
    }
    fn sentries(&self) -> u8 {
        0
    }
    fn owner(&self) -> SlotId {
        self.owner
    }
    fn mode(&self) -> Option<Attribute> {
        self.mode
    }
    fn for_sale(&self) -> bool {
        false
    }
    fn locked(&self) -> bool {
        false
    }
}

/// Where the `parts` a seat buys go, one berth to each, or nothing when they do not all fit.
/// `bag` is the hero's bag, absent while the hero is not in its shop or lies dead: what is bought
/// away from the fountain waits in the stash.
///
/// A part that merges (a consumable, a ward) is one purchase of its own, which joins a compatible
/// stack with room before it takes an empty slot; any other part takes the first empty slot, the
/// bag's before the stash's.
pub fn place_purchase<P: HeldItem>(
    buyer: SlotId,
    parts: &[ItemId],
    bag: Option<&Shelf<'_, P>>,
    stash: &Shelf<'_, P>,
) -> Option<Vec<Berth>> {
    let mut berths: Vec<Berth> = Vec::with_capacity(parts.len());
    for part in parts {
        let def = item_def(*part).filter(|def| !def.unbuyable)?;
        let berth = if def.stack_limit > 0 {
            debug_assert_eq!(parts.len(), 1, "a part that merges is bought alone");
            let bought = Bought {
                id: *part,
                charges: def.charges,
                owner: buyer,
                mode: def.mode,
            };
            bag.and_then(|bag| bag.receiving_slot(&bought))
                .map(Berth::Bag)
                .or_else(|| stash.receiving_slot(&bought).map(Berth::Stash))?
        } else {
            let empty = |shelf: &Shelf<'_, P>, berth: fn(usize) -> Berth| {
                shelf
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|(_, held)| held.is_none())
                    .map(|(at, _)| berth(at))
                    .find(|berth| !berths.contains(berth))
            };
            bag.and_then(|bag| empty(bag, Berth::Bag))
                .or_else(|| empty(stash, Berth::Stash))?
        };
        berths.push(berth);
    }
    Some(berths)
}

/// Whether the `parts` a seat buys fit where they go: [`place_purchase`] finds them berths.
pub fn purchase_fits<P: HeldItem>(
    buyer: SlotId,
    parts: &[ItemId],
    bag: Option<&Shelf<'_, P>>,
    stash: &Shelf<'_, P>,
) -> bool {
    place_purchase(buyer, parts, bag, stash).is_some()
}
