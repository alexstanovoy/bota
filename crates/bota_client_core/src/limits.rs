//! Bounds on what a server may tell a client. Every sequence a client keeps
//! or reads is capped by one of these, so a corrupt or hostile stream cannot
//! make it grow without end.

use bota_core::rules;

/// A courier's bag: an inventory, no backpack.
pub const COURIER_ITEM_SLOTS: usize = rules::INVENTORY_SLOTS;
/// A hero's bag: the inventory and the backpack.
pub const HERO_ITEM_SLOTS: usize = rules::INVENTORY_SLOTS + rules::BACKPACK_SLOTS;
pub const STASH_ITEM_SLOTS: usize = rules::STASH_SLOTS;
/// Maximum number of seats in one match.
pub const MAX_SEATS: usize = 10;
/// Maximum visible units per snapshot and entity records retained.
pub const MAX_TRACKED_ENTITIES: usize = 4_096;
/// Maximum observed projectiles per snapshot.
pub const MAX_PROJECTILES: usize = 4_096;
/// Maximum ground items accepted in one snapshot.
pub const MAX_SNAPSHOT_LOOT: usize = 4_096;
/// World units a projectile may lie past each map edge. Bodies, loot and trees
/// stay on open ground, but straight-line effects fly to their full reach from
/// an in-map caster without clamping; the margin covers the longest of them
/// with headroom while still rejecting corrupt coordinates.
pub const PROJECTILE_MAP_MARGIN: i32 = 2_048;
/// Maximum number of ability slots accepted for any unit.
pub const MAX_ABILITY_SLOTS: usize = 8;

const _: () = {
    let mut at = 0;
    while at < bota_core::HEROES.len() {
        assert!(bota_core::HEROES[at].abilities.len() <= MAX_ABILITY_SLOTS);
        at += 1;
    }
};
/// Number of own hero, stash, and courier item slots.
pub const OWN_ITEM_SLOTS: usize = HERO_ITEM_SLOTS + STASH_ITEM_SLOTS + COURIER_ITEM_SLOTS;
/// Maximum number of shop entries: one for each item there is.
pub const MAX_SHOP_ITEMS: usize = bota_core::ITEMS.len();
/// Maximum number of components accepted for one shop entry.
pub const MAX_SHOP_COMPONENTS: usize = 16;
/// Most other recipes one shop entry may list.
pub const MAX_SHOP_RECIPES: usize = 8;
/// Most items one shop entry's first recipe lays out, itself and the parts of
/// its parts included.
pub const MAX_RECIPE_TREE: usize = 64;
/// Maximum number of recent events retained.
pub const MAX_RECENT_EVENTS: usize = 64;
/// Maximum events accepted per batch, equal to half the wire payload byte cap.
pub const MAX_EVENTS_PER_BATCH: usize = bota_proto::MAX_PAYLOAD_LEN / 2;
/// Most ticks one snapshot may tell were skipped: thirty seconds at 30 ticks
/// a second, the bound past which the server closes the connection.
pub const MAX_SKIPPED_TICKS: usize = 900;
/// Maximum number of static trees accepted from match metadata.
pub const MAX_STATIC_TREES: usize = 4_096;
/// Maximum number of planted trees in one snapshot.
pub const MAX_PLANTED_TREES: usize = 4_096;
/// Maximum number of effects accepted on one unit.
pub const MAX_EFFECTS_PER_UNIT: usize = 32;
/// Largest accepted manual health or mana restoration report, in whole pool units.
pub const MAX_RESTORATION_REPORT: i32 = 1_000_000;
/// Maximum terrain axis accepted from match metadata.
pub const MAX_TERRAIN_AXIS: u32 = 512;
/// Maximum number of terrain runs accepted from match metadata.
pub const MAX_TERRAIN_RUNS: usize = 262_144;
/// Maximum number of opaque terrain cells accepted from match metadata.
pub const MAX_OPAQUE_CELLS: usize = 262_144;
/// Messages a strict stream may deliver without a snapshot among them.
pub const MAX_MESSAGES_WITHOUT_SNAPSHOT: usize = 4_096;
