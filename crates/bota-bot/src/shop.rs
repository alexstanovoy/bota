//! What the bot buys, and how what it buys reaches its hands.
//!
//! Prices and parts are read off [`MatchInfo::shop`] rather than written out
//! here: the wire carries both, so the one thing the bot has to know of
//! itself is which items it wants and in what order.
//!
//! The list is parts rather than builds. An order to buy a built item is
//! turned down unless the whole of its price is in hand, while the server
//! puts a build together on its own the moment its parts are in the bag —
//! so buying the parts spends gold as it arrives.

use bota_proto::{HeroId, ItemId, ItemView, MatchInfo, ShopEntry, Target};

use crate::{
    Ask, BELT, BOOTS, BRANCH, BROADSWORD, CIRCLET, CLARITY, COURIER_BATCH, COURIER_PATIENCE,
    DELIVER, ERRAND_TICKS, FIGHT_RANGE, Field, GAUNTLETS, GLOVES, MAGIC_STICK, MANTLE,
    QUARTERSTAFF, RECIPE_BRACER, RECIPE_MAGIC_WAND, RECIPE_NULL_TALISMAN, RECIPE_WRAITH_BAND,
    SAGES_MASK, SALVE, SCROLL, SHADOW_FIEND, SLIPPERS, SPARE_SLOTS, SYLLA, TAKE_STASH, TANGO,
};

/// The shop as the match described it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stall {
    /// Everything on sale, in item id order.
    pub entries: Vec<ShopEntry>,
}

impl Stall {
    /// The shop a match opened with.
    pub fn of(info: &MatchInfo) -> Stall {
        Stall {
            entries: info.shop.clone(),
        }
    }

    /// What one item costs whole. Nought for one the shop does not sell.
    pub fn cost_of(&self, item: ItemId) -> i32 {
        self.entry(item).map_or(0, |entry| entry.cost)
    }

    /// What one item is built from. Empty for one bought whole.
    pub fn parts_of(&self, item: ItemId) -> &[ItemId] {
        self.entry(item).map_or(&[][..], |entry| &entry.components)
    }

    /// How many of one item went into another, the item itself counted.
    ///
    /// A part that has gone into a build is still a part that was paid for.
    pub fn how_many_within(&self, held: ItemId, wanted: ItemId) -> usize {
        if held == wanted {
            return 1;
        }
        self.parts_of(held)
            .iter()
            .map(|part| self.how_many_within(*part, wanted))
            .sum()
    }

    /// How many of an item a row of slots holds, the builds counted into.
    pub fn how_many_in(&self, slots: &[Option<ItemView>], wanted: ItemId) -> usize {
        slots
            .iter()
            .flatten()
            .map(|held| self.how_many_within(held.id, wanted))
            .sum()
    }

    /// How many of an item the seat owns, wherever it sits: the bag, the
    /// stash, and whatever the courier is carrying.
    pub fn how_many_held(&self, field: &Field, wanted: ItemId) -> usize {
        let bag = field.me.map_or(0, |me| self.how_many_in(&me.items, wanted));
        let kit = field
            .seat
            .kit
            .as_ref()
            .map_or(0, |kit| self.how_many_in(&kit.items, wanted));
        let stash = field
            .seat
            .stash
            .as_ref()
            .map_or(0, |slots| self.how_many_in(slots, wanted));
        let riding = field
            .courier
            .map_or(0, |bird| self.how_many_in(&bird.items, wanted));
        // A standing hero carries its bag; a fallen one leaves it behind as a
        // kit. Only one of the two is ever there.
        bag + kit + stash + riding
    }

    /// The next thing on a hero's list, when there is gold for it.
    ///
    /// The list is walked in order and stops at the first thing not owned:
    /// skipping ahead to whatever is affordable spends on the tail of a list
    /// the gold its head was being saved for.
    ///
    /// A consumable with no working slot to go in is stepped over rather than
    /// stopped at. What holds it up is room, not gold, and gold that is not
    /// being saved for it should go on what comes next.
    pub fn next_buy(&self, field: &Field) -> Option<ItemId> {
        let room = field.free_slots() > SPARE_SLOTS;
        let mut wanted: Vec<(ItemId, usize)> = Vec::new();
        for item in shopping_list(field.hero) {
            let want = match wanted.iter_mut().find(|(had, _)| had == item) {
                Some((_, count)) => {
                    *count += 1;
                    *count
                }
                None => {
                    wanted.push((*item, 1));
                    1
                }
            };
            if self.how_many_held(field, *item) >= want {
                continue;
            }
            if is_consumable(*item) && !room {
                continue;
            }
            return (field.gold() >= self.cost_of(*item)).then_some(*item);
        }
        None
    }

    /// One row of the shop.
    fn entry(&self, item: ItemId) -> Option<&ShopEntry> {
        self.entries.iter().find(|entry| entry.id == item)
    }
}

/// The consumables the bot buys.
///
/// What an item **is** does not cross the wire, so the ones that are spent by
/// using them are named here. Their place in a list is where restocking
/// happens: drunk, a stack stops being held, and the next walk of the list
/// finds it wanting again.
pub const CONSUMABLES: [ItemId; 4] = [TANGO, CLARITY, SALVE, SCROLL];

/// Whether an item is one that is spent by using it.
pub fn is_consumable(item: ItemId) -> bool {
    CONSUMABLES.contains(&item)
}

/// Shadow Fiend's list: the drink first, then everything that answers the
/// one thing he runs out of — a talisman and a mask for the mana, a wand for
/// both pools — and plain damage last.
///
/// A raze costs the better part of a hundred mana against a pool that mends
/// some two a second, so what is bought is mana before damage. Nothing is
/// bought that no want on the ladder reaches for.
pub const FIEND_GOODS: [ItemId; 16] = [
    TANGO,
    CLARITY,
    SCROLL,
    BRANCH,
    BRANCH,
    CIRCLET,
    MANTLE,
    RECIPE_NULL_TALISMAN,
    SAGES_MASK,
    SALVE,
    BOOTS,
    MAGIC_STICK,
    RECIPE_MAGIC_WAND,
    GLOVES,
    BELT,
    BROADSWORD,
];

/// Sylla's list: a band for the agility, a wand, treads, then plain damage.
pub const SYLLA_GOODS: [ItemId; 16] = [
    TANGO,
    CLARITY,
    SCROLL,
    BRANCH,
    BRANCH,
    CIRCLET,
    SLIPPERS,
    RECIPE_WRAITH_BAND,
    SALVE,
    BOOTS,
    MAGIC_STICK,
    RECIPE_MAGIC_WAND,
    GLOVES,
    BELT,
    BROADSWORD,
    QUARTERSTAFF,
];

/// What a hero with no list of its own buys.
pub const PLAIN_GOODS: [ItemId; 14] = [
    TANGO,
    CLARITY,
    SCROLL,
    BRANCH,
    BRANCH,
    SALVE,
    BOOTS,
    MAGIC_STICK,
    RECIPE_MAGIC_WAND,
    GLOVES,
    BELT,
    CIRCLET,
    GAUNTLETS,
    RECIPE_BRACER,
];

/// What a hero wants to own, in the order it wants it.
pub fn shopping_list(hero: HeroId) -> &'static [ItemId] {
    match hero {
        SHADOW_FIEND => &FIEND_GOODS,
        SYLLA => &SYLLA_GOODS,
        _ => &PLAIN_GOODS,
    }
}

/// What the courier has been sent for, and when.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Errands {
    /// The tick an errand was last called for.
    sent_at: Option<u32>,
    /// The tick the stash stopped being empty.
    waiting_since: Option<u32>,
}

impl Errands {
    /// A courier that has been sent nowhere.
    pub fn new() -> Errands {
        Errands::default()
    }

    /// Takes one tick in, keeping how long the stash has been waiting.
    pub fn watch(&mut self, tick: u32, field: &Field) {
        let waiting = field
            .seat
            .stash
            .as_ref()
            .is_some_and(|slots| slots.iter().flatten().count() > 0);
        match (waiting, self.waiting_since) {
            (true, None) => self.waiting_since = Some(tick),
            (false, _) => self.waiting_since = None,
            (true, Some(_)) => {}
        }
    }

    /// The errand worth calling for this tick.
    ///
    /// A trip is not made for one item: it waits until [`COURIER_BATCH`] have
    /// piled up or the first has waited [`COURIER_PATIENCE`] ticks. It is
    /// held back entirely while an enemy hero stands near the seat's own
    /// hero, since a courier walks to where its owner stands.
    pub fn errand(&mut self, tick: u32, field: &Field) -> Option<Ask> {
        let bird = field.courier?;
        if self
            .sent_at
            .is_some_and(|at| tick.saturating_sub(at) < ERRAND_TICKS)
        {
            return None;
        }
        if field.foes_within(FIGHT_RANGE).next().is_some() {
            return None;
        }
        let carrying = bird.items.iter().flatten().count();
        let waiting = field
            .seat
            .stash
            .as_ref()
            .map_or(0, |slots| slots.iter().flatten().count());
        let patience_out = self
            .waiting_since
            .is_some_and(|at| tick.saturating_sub(at) >= COURIER_PATIENCE);
        let errand = if carrying > 0 {
            DELIVER
        } else if waiting >= COURIER_BATCH || (waiting > 0 && patience_out) {
            TAKE_STASH
        } else {
            return None;
        };
        let at = bird
            .abilities
            .iter()
            .position(|slot| slot.id == errand)
            .map(|at| bota_proto::AbilitySlot(at as u8))?;
        self.sent_at = Some(tick);
        Some(Ask::of(
            bird.id,
            bota_proto::Order::Cast {
                slot: at,
                target: Target::None,
            },
        ))
    }
}
