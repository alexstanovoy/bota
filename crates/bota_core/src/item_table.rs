//! The catalog itself: the rows of `data/item_mechanics.rs` placed by id, and
//! what every row must hold. The numbers are Dota 7.41f's, generated from the
//! game's `scripts/npc/items.txt` into `data/items.rs`.

use bota_proto::ItemId;

use crate::data::ITEM_ROWS;
use crate::table::placed_by_id;
use crate::{ItemDef, MAX_COMPONENTS, UseSpec};

/// How many item ids the protocol has ever handed out, retired ones
/// included: every table indexed by [`ItemId`] is this long.
pub const ITEM_COUNT: usize = ITEM_ROWS.len();

/// The catalog, indexed by [`ItemId`]: every row of `ITEM_ROWS` at its id.
///
/// # Adding an item
///
/// Every item:
///
/// 1. Import it: `cargo run --manifest-path tools/import_items/Cargo.toml --
///    --add item_x` appends its line to `bota_proto/registry/ids.txt` and
///    writes its numbers into `data/items.rs` from the install's `items.txt`
///    (an `ItemId::X` constant in `bota_proto/src/ids.rs` only if code names
///    the item). A number where Dota 7.41f is not the install is an override
///    in the tool, with its evidence.
/// 2. Add its row to `ITEM_ROWS` in `data/item_mechanics.rs`:
///    `ItemDef { name, ..from(&ITEM_X) }` plus what it does, every number read
///    by its Dota key. A recipe is two rows, the recipe scroll and the build;
///    the build's parts come from the recipe's requirements.
/// 3. No version to bump: `data/` is the catalogue digest's, which moves the
///    wire fingerprint by itself. The determinism pins re-record (the drivers
///    buy by the length of this table), and for a stocked item so do the
///    dummy bench's digests.
/// 4. The shop lists it by itself: [`sold_items`](crate::sold_items) is every
///    entry that is neither retired nor `unbuyable`. A front end finds its art
///    and texts by its key.
///
/// A passive stat item that uses only the stats [`Carried`] has stops there.
/// Beyond it:
///
/// - A new stat: a [`Carried`] field with its `NOTHING` entry and its arm in
///   `carried_bonus` (`item.rs`), the server's `systems/stats.rs`, often a
///   `Stats` field.
/// - An active: a [`UseSpec`] variant holding its numbers, and its arm in
///   `use_effect` (`bota_server/src/game/config/item.rs`) with the effect as
///   an `impl World` function; a channel also needs its `use_goes_on`,
///   `use_breaks_off` and `use_completes` arms.
/// - A buff or debuff it applies: an `EffectId` and registry line, a
///   [`ModifierKind`](crate::ModifierKind) variant with its arms, an
///   [`EFFECTS`](crate::EFFECTS) row, and the server's `hash.rs`, `stats.rs`
///   and `systems/modifiers.rs` arms.
/// - A charge-spending consumable: the `charges`, `stack_limit`, `spends` and
///   `stock` fields; healing at once or reacting to damage needs a variant and
///   an arm.
///
/// Neutral items are not modelled yet. Every item gets its end-to-end test in
/// `bota_server/src/game/tests/`.
// A const, not a static: the client's limits and the catalog checks read it
// at compile time.
#[allow(clippy::large_const_arrays)]
pub const ITEMS: [ItemDef; ITEM_COUNT] =
    placed_by_id!(ITEM_ROWS, ITEM_COUNT, |row| row.id.0 as usize);

const _: () = {
    // A stocked item is bought whole, and is no part of a build.
    let mut at = 0;
    while at < ITEMS.len() {
        let def = &ITEMS[at];
        let instant = def.point == 0 && def.backswing == 0 && def.duration == 0;
        assert!(!def.ignores_channel || instant);
        assert!(!def.immediate || instant);
        if let Some(stock) = ITEMS[at].stock {
            assert!(ITEMS[at].components.is_empty());
            assert!(stock.initial <= stock.max);
            assert!(stock.max > 0);
            assert!(stock.restock_ticks > 0);
        }
        let mut part = 0;
        while part < ITEMS[at].components.len() {
            let held = &ITEMS[ITEMS[at].components[part].0 as usize];
            assert!(held.stock.is_none());
            // Nothing is built of a retired entry.
            assert!(!held.retired);
            part += 1;
        }
        // A built item costs its parts, the recipe scroll among them.
        assert!(def.components.is_empty() || def.cost == parts_cost(def.components));
        // Every other recipe costs what the first does, and only a built
        // item has one.
        let mut other = 0;
        while other < def.alternatives.len() {
            assert!(!def.components.is_empty());
            assert!(parts_cost(def.alternatives[other].parts) == parts_cost(def.components));
            other += 1;
        }
        // What the server relies on whatever a client orders: a recipe has
        // parts, within the slots an assembly takes, that a shop sells whole;
        // a stackable is bought in a lot of charges that fits its limit, with
        // no parts of its own; a cost is a price.
        assert!(def.components.len() <= MAX_COMPONENTS);
        assert!(def.cost >= 0);
        if def.stack_limit > 0 {
            assert!(def.charges >= 1 && def.charges <= def.stack_limit);
            assert!(def.components.is_empty() && def.alternatives.is_empty());
        }
        if let UseSpec::Replenish { mana } = def.effect {
            assert!(mana > 0);
        }
        let mut part = 0;
        while part < def.components.len() {
            assert!(sold_whole(def.components[part]));
            part += 1;
        }
        let mut other = 0;
        while other < def.alternatives.len() {
            let parts = def.alternatives[other].parts;
            assert!(!parts.is_empty() && parts.len() <= MAX_COMPONENTS);
            let mut part = 0;
            while part < parts.len() {
                assert!(sold_whole(parts[part]));
                part += 1;
            }
            other += 1;
        }
        // A retired entry costs, builds and carries nothing.
        if def.retired {
            assert!(def.cost == 0 && def.components.is_empty() && def.ranged.is_none());
        }
        at += 1;
    }
};

/// Whether a shop can sell an item as a part of a build: it is one of the
/// catalog's, a buyer takes it whole and no stock is kept of it.
const fn sold_whole(item: ItemId) -> bool {
    let def = &ITEMS[item.0 as usize];
    !def.unbuyable && def.stack_limit == 0
}

// The scroll every hero starts with is a one-charge purchase.
const _: () = {
    let scroll = &ITEMS[ItemId::TOWN_PORTAL_SCROLL.0 as usize];
    assert!(scroll.charges == 1 && !scroll.unbuyable);
};

/// What a set of parts costs together.
const fn parts_cost(parts: &[ItemId]) -> i32 {
    let mut sum = 0;
    let mut at = 0;
    while at < parts.len() {
        sum += ITEMS[parts[at].0 as usize].cost;
        at += 1;
    }
    sum
}
