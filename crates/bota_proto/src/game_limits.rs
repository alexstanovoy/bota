//! Bounds used by simulation order processing, state and per-side visibility.
//! Wire-only framing, decoding and chat limits remain in `limits`.

pub const MAX_ORDER_DELAY_TICKS: u32 = 30;
pub const MAX_QUEUED_ORDERS: usize = 32;
pub const MAX_ORDERS_PER_TICK: usize = 64;
pub const INVENTORY_SLOTS: usize = 6;
pub const BACKPACK_SLOTS: usize = 3;
pub const STASH_SLOTS: usize = 6;
pub const TOWN_PORTAL_SLOT: usize = 15;
pub const MAX_SEATS: usize = 10;
/// At most one bounty, assist share and flagbearer bonus per seat.
pub const MAX_DEATH_GOLD_AWARDS: usize = 3 * MAX_SEATS;
pub const MAX_ABILITY_SLOTS: usize = 8;
/// Physical modifier storage is independent of the projected movement-controller rows.
pub const MAX_MODIFIERS_PER_UNIT: usize = 128;
pub const MAX_EFFECTS_PER_UNIT: usize = MAX_MODIFIERS_PER_UNIT + 2;
pub const MAX_VIEW_UNITS: usize = 4_096;
pub const MAX_PROJECTILES: usize = 4_096;
pub const MAX_SNAPSHOT_LOOT: usize = 4_096;
pub const MAX_STATIC_TREES: usize = 4_096;
pub const MAX_PLANTED_TREES: usize = 4_096;
pub const MAX_SKIPPED_TICKS: usize = 900;
pub const MAX_SHOP_ENTRIES: usize = 1_024;
pub const MAX_VIEW_STOCK: usize = 2 * MAX_SHOP_ENTRIES;
pub const MAX_ORDERS_TOLD: usize = MAX_SEATS * MAX_ORDERS_PER_TICK;
pub const MAX_SHOP_COMPONENTS: usize = 16;
pub const MAX_SHOP_RECIPES: usize = 8;
pub const MAX_TERRAIN_RUNS: usize = 262_144;
pub const MAX_OPAQUE_CELLS: usize = 262_144;
pub const MAX_SNAPSHOT_RUNES: usize = 1_024;
pub const MAX_RUNES_AT_SPOT: usize = 16;
pub const MAX_LOTUS_POOLS: usize = 16;
pub const MAX_SECRET_SHOPS: usize = 8;
pub const MAX_SHOP_ZONE_CORNERS: usize = 32;

/// A hero's bag: the inventory and the backpack.
pub const HERO_ITEM_SLOTS: usize = INVENTORY_SLOTS + BACKPACK_SLOTS;
/// A courier's bag: an inventory, no backpack.
pub const COURIER_ITEM_SLOTS: usize = INVENTORY_SLOTS;
