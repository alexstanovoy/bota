//! Bounds on what a server may tell a client. Every sequence a client keeps
//! or reads is capped by one of these, so a corrupt or hostile stream cannot
//! make it grow without end.
//!
//! The bounds a well-behaved server reaches are the protocol's
//! ([`bota_proto::limits`]), which the server keeps to and which are re-exported
//! here. What is defined here is the client's own: what it keeps of a stream,
//! or what the data fixes, each asserted over the data it bounds.

use bota_core::rules;
pub use bota_proto::limits::{
    COURIER_ITEM_SLOTS, HERO_ITEM_SLOTS, MAX_ABILITY_SLOTS, MAX_EFFECTS_PER_UNIT,
    MAX_EVENTS_PER_BATCH, MAX_OPAQUE_CELLS, MAX_PLANTED_TREES, MAX_PROJECTILES, MAX_SEATS,
    MAX_SHOP_COMPONENTS, MAX_SHOP_RECIPES, MAX_SKIPPED_TICKS, MAX_SNAPSHOT_LOOT,
    MAX_SNAPSHOT_RUNES, MAX_STATIC_TREES, MAX_TERRAIN_RUNS,
};

/// A stash's slots.
pub const STASH_ITEM_SLOTS: usize = rules::STASH_SLOTS;
/// Maximum visible units per snapshot and entity records retained: the most a
/// view lists.
pub const MAX_TRACKED_ENTITIES: usize = bota_proto::MAX_VIEW_UNITS;
/// World units a projectile may lie past each map edge. Bodies, loot and trees
/// stay on open ground, but straight-line effects fly to their full reach from
/// an in-map caster without clamping; the margin covers the longest of them
/// with headroom while still rejecting corrupt coordinates.
pub const PROJECTILE_MAP_MARGIN: i32 = 2_048;
/// Number of own hero, stash, and courier item slots.
pub const OWN_ITEM_SLOTS: usize = HERO_ITEM_SLOTS + STASH_ITEM_SLOTS + COURIER_ITEM_SLOTS;
/// Maximum number of shop entries: one for each item there is.
pub const MAX_SHOP_ITEMS: usize = bota_core::ITEMS.len();
/// Most items one shop entry's first recipe lays out, itself and the parts of
/// its parts included.
pub const MAX_RECIPE_TREE: usize = 64;
/// Maximum number of recent events retained.
pub const MAX_RECENT_EVENTS: usize = 64;
/// Largest accepted manual health or mana restoration report, in whole pool units.
pub const MAX_RESTORATION_REPORT: i32 = 1_000_000;
/// Maximum terrain axis accepted from match metadata.
pub const MAX_TERRAIN_AXIS: u32 = 512;
/// Messages a strict stream may deliver without a snapshot among them.
pub const MAX_MESSAGES_WITHOUT_SNAPSHOT: usize = 4_096;

// What the data fixes stays within what the protocol and the client allow, so
// that growing the data past a bound fails the build, not a match.
const _: () = {
    assert!(bota_core::ITEMS.len() <= bota_proto::MAX_SHOP_ENTRIES);
    let mut at = 0;
    while at < bota_core::HEROES.len() {
        assert!(bota_core::HEROES[at].abilities.len() <= MAX_ABILITY_SLOTS);
        at += 1;
    }
    let mut item = 0;
    while item < bota_core::ITEMS.len() {
        let def = &bota_core::ITEMS[item];
        assert!(def.components.len() <= MAX_SHOP_COMPONENTS);
        assert!(def.alternatives.len() <= MAX_SHOP_RECIPES);
        let mut recipe = 0;
        while recipe < def.alternatives.len() {
            assert!(def.alternatives[recipe].parts.len() <= MAX_SHOP_COMPONENTS);
            recipe += 1;
        }
        assert!(recipe_tree(item) <= MAX_RECIPE_TREE);
        item += 1;
    }
};

/// Items an item's first recipe lays out, itself and the parts of its parts
/// included: what the client checks a shop's entries against.
const fn recipe_tree(item: usize) -> usize {
    let parts = bota_core::ITEMS[item].components;
    let mut tree = 1;
    let mut at = 0;
    while at < parts.len() {
        tree += recipe_tree(parts[at].0 as usize);
        at += 1;
    }
    tree
}
