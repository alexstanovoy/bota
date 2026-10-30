//! Every bound of the protocol: how large a frame may be, how much a seat may
//! ask of a tick, how long a name or a line may be, and how many entries each
//! collection of a message holds.
//!
//! A server keeps to these when it builds a message and a client refuses a
//! message past them, so a legal view is never refused and a hostile stream
//! never makes a client grow without end. They are part of the wire: the
//! values are folded into [`REGISTRY_DIGEST`](crate::REGISTRY_DIGEST), so two
//! builds that disagree on one meet at the hello and not in a refused view.
//!
//! A bound that the data fixes (a recipe's parts, the items of the shop) is
//! checked against the data where the data lives, with a `const` assertion.
//! A bound a client keeps for its own sake, as a guard against a hostile
//! server, lives in the client library and says what margin it keeps over
//! the ones here.

/// Declares the limits and lists their values, in declaration order, for the
/// wire digest.
macro_rules! limits {
    ($($(#[$doc:meta])* $name:ident: $ty:ty = $value:expr;)*) => {
        $($(#[$doc])* pub const $name: $ty = $value;)*

        /// Every limit's value in declaration order: what the wire digest
        /// folds.
        pub(crate) const VALUES: &[u64] = &[$($name as u64),*];
    };
}

limits! {
    /// Largest payload accepted in one frame, in bytes.
    MAX_PAYLOAD_LEN: usize = 4 * 1024 * 1024;
    /// Largest payload a client may send in one frame, in bytes: a `Hello`
    /// with the longest name a client announces is about a tenth of it. A
    /// server closes a connection whose frame announces more, before reading
    /// it.
    MAX_CLIENT_PAYLOAD_LEN: usize = 1024;

    /// Most ticks an order may wait past the earliest tick it could take
    /// effect on, base and jitter together.
    MAX_ORDER_DELAY_TICKS: u32 = 30;
    /// Most orders one unit holds queued (Dota's Shift-queue) behind the one
    /// it carries out.
    MAX_QUEUED_ORDERS: usize = 32;
    /// Most orders of one seat that may take effect on one tick, its units
    /// and queued orders together.
    MAX_ORDERS_PER_TICK: usize = 64;

    /// Most bytes of memory a reader may hold for one frame it decodes: twenty
    /// times the payload at most. A byte of the wire stands for at most about
    /// thirty bytes of a decoded element, so the collections are bounded by
    /// both their length and their weight, asserted below for the heaviest.
    MAX_DECODED_BYTES: usize = 80 * 1024 * 1024;

    /// Longest name a seat holder is shown under, in UTF-8 bytes; a longer
    /// one is cut at the last whole character within it.
    MAX_SHOWN_NAME_BYTES: usize = 32;
    /// Longest name, in bytes, the client library announces in a hello and
    /// the server takes a seat reservation under; a longer one is refused
    /// there. The server cuts what it shows to [`MAX_SHOWN_NAME_BYTES`].
    MAX_ANNOUNCED_NAME_BYTES: usize = 64;
    /// Longest chat line, in bytes of UTF-8.
    MAX_CHAT_BYTES: usize = 512;
    /// Longest chat line in characters as a reader counts them (extended
    /// grapheme clusters).
    MAX_CHAT_GRAPHEMES: usize = 127;

    /// Inventory slots, where items work.
    INVENTORY_SLOTS: usize = 6;
    /// Backpack slots, where items are carried inert.
    BACKPACK_SLOTS: usize = 3;
    /// Stash slots at the home shop.
    STASH_SLOTS: usize = 6;
    /// The number of a hero's town portal slot, after the inventory, the
    /// backpack and the stash: [`ItemSlot::TOWN_PORTAL`](crate::ItemSlot).
    TOWN_PORTAL_SLOT: usize = 15;

    /// Most seats in one match.
    MAX_SEATS: usize = 10;
    /// Most ability slots on any unit.
    MAX_ABILITY_SLOTS: usize = 8;
    /// Most modifiers one unit carries at once, hidden and shown. What a hero
    /// in a five-a-side fight bears (its own buffs, its allies' auras, its
    /// enemies' debuffs) is a few dozen. A unit that carries this many takes
    /// a fresh modifier by dropping the one with the least time left, or not
    /// at all when every one it carries is untimed. The effects a view lists
    /// on a unit therefore never exceed it, and a client may refuse a view
    /// that does.
    MAX_EFFECTS_PER_UNIT: usize = 128;
    /// Most units one view lists.
    MAX_VIEW_UNITS: usize = 4_096;
    /// Most projectiles one view lists.
    MAX_PROJECTILES: usize = 4_096;
    /// Most ground items one view lists.
    MAX_SNAPSHOT_LOOT: usize = 4_096;
    /// Most trees a match starts with.
    MAX_STATIC_TREES: usize = 4_096;
    /// Most trees planted in a match at once.
    MAX_PLANTED_TREES: usize = 4_096;
    /// Most ticks one snapshot may tell were skipped: thirty seconds at 30
    /// ticks a second, the bound past which the server closes the
    /// connection.
    MAX_SKIPPED_TICKS: usize = 900;
    /// Most events in one batch, half the bytes of a payload.
    MAX_EVENTS_PER_BATCH: usize = MAX_PAYLOAD_LEN / 2;
    /// Most entries in the shop of a match: room for every item there is.
    MAX_SHOP_ENTRIES: usize = 1_024;
    /// Most shelves of limited stock a view lists: both sides' for every entry
    /// of the shop.
    MAX_VIEW_STOCK: usize = 2 * MAX_SHOP_ENTRIES;
    /// Most orders told as accepted on one tick: every seat's.
    MAX_ORDERS_TOLD: usize = MAX_SEATS * MAX_ORDERS_PER_TICK;
    /// Most components one shop entry lists.
    MAX_SHOP_COMPONENTS: usize = 16;
    /// Most other recipes one shop entry may list.
    MAX_SHOP_RECIPES: usize = 8;
    /// Most runs of a map's terrain.
    MAX_TERRAIN_RUNS: usize = 262_144;
    /// Most opaque cells of a map.
    MAX_OPAQUE_CELLS: usize = 262_144;
}

/// A hero's bag: the inventory and the backpack.
pub const HERO_ITEM_SLOTS: usize = INVENTORY_SLOTS + BACKPACK_SLOTS;
/// A courier's bag: an inventory, no backpack.
pub const COURIER_ITEM_SLOTS: usize = INVENTORY_SLOTS;

// What the heaviest collections hold, element sizes included, fits what a frame may make a reader
// hold: a bound that grows (or an element that does) fails the build, not a reader.
const _: () = {
    use core::mem::size_of;
    let unit = size_of::<crate::UnitView>()
        + MAX_EFFECTS_PER_UNIT * size_of::<crate::EffectView>()
        + MAX_QUEUED_ORDERS * size_of::<crate::Order>()
        + HERO_ITEM_SLOTS * size_of::<Option<crate::ItemView>>()
        + MAX_ABILITY_SLOTS * size_of::<crate::AbilityView>();
    assert!(MAX_VIEW_UNITS * unit <= MAX_DECODED_BYTES);
    assert!(MAX_EVENTS_PER_BATCH * size_of::<crate::EventKind>() <= MAX_DECODED_BYTES);
    assert!(MAX_SNAPSHOT_LOOT * size_of::<crate::LootView>() <= MAX_DECODED_BYTES);
    assert!(MAX_PROJECTILES * size_of::<crate::ProjectileView>() <= MAX_DECODED_BYTES);
    assert!(MAX_SHOP_ENTRIES * size_of::<crate::ShopEntry>() <= MAX_DECODED_BYTES);
    assert!(MAX_ORDERS_TOLD * size_of::<crate::SlotOrder>() <= MAX_DECODED_BYTES);
};

const _: () = {
    assert!(MAX_CLIENT_PAYLOAD_LEN <= MAX_PAYLOAD_LEN);
    assert!(TOWN_PORTAL_SLOT == INVENTORY_SLOTS + BACKPACK_SLOTS + STASH_SLOTS);
    assert!(TOWN_PORTAL_SLOT == crate::ItemSlot::TOWN_PORTAL.0 as usize);
};
