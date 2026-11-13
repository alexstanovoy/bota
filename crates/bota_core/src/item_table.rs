//! The catalog itself: the rows of `data/item_mechanics.rs` placed by id, and
//! what every row must hold. The numbers are Dota 7.41f's, generated from the
//! game's `scripts/npc/items.txt` into `data/items.rs`.

use bota_proto::ItemId;

use crate::data::ITEM_ROWS;
use crate::table::placed_by_id;
use crate::{AllyUse, ItemDef, MAX_COMPONENTS, MAX_ITEM_STRIKES, UseSpec};

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
///    `ItemDef { ..from(&ITEM_X) }` plus what it does, every number read
///    by its Dota key. A recipe is two rows, the recipe scroll and the build;
///    the build's parts come from the recipe's requirements.
/// 3. Nothing else moves: the wire fingerprint holds no items, so older
///    clients and replays keep playing, on the [`Catalogue`](crate::Catalogue)
///    they hold. The determinism pins of a full catalogue re-record (the
///    drivers buy by the length of this table), and for a stocked item so do
///    the dummy bench's digests. A changed number of an item already there
///    changes that item's digest: builds that differ on it no longer meet.
/// 4. The shop lists it by itself: [`sold_items`](crate::sold_items) is every
///    entry that is neither retired nor `unbuyable`. A front end finds its art
///    and texts by its key.
///
/// A passive stat item that uses only the stats [`Carried`](crate::Carried) has stops there.
/// Beyond it:
///
/// - A new stat: a [`Carried`](crate::Carried) field with its `NOTHING` entry and its arm in
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
    keyed_by_wait_group(placed_by_id!(ITEM_ROWS, ITEM_COUNT, |row| row.id.0 as usize));

/// The catalogue with each item that shares a named cooldown ([`ItemDef::wait_group`]) owed
/// under the first of the catalogue naming the same one ([`ItemDef::wait_key`]): the
/// lotuses' "famango" under the Healing Lotus, the Magic Wand's "magicwand" under the Magic
/// Stick.
const fn keyed_by_wait_group(mut items: [ItemDef; ITEM_COUNT]) -> [ItemDef; ITEM_COUNT] {
    let mut at = 0;
    while at < ITEM_COUNT {
        if let Some(group) = items[at].wait_group {
            let mut first = 0;
            while first < at {
                if let Some(theirs) = items[first].wait_group
                    && crate::item_data::str_eq(group, theirs)
                {
                    items[at].wait_key = Some(items[first].id);
                    break;
                }
                first += 1;
            }
        }
        at += 1;
    }
    items
}

// The groups the install names, as the catalogue keys them.
const _: () = {
    use bota_proto::ItemId;
    assert!(ITEMS[ItemId::HEALING_LOTUS.0 as usize].wait_key.is_none());
    assert!(matches!(
        ITEMS[ItemId::GREAT_HEALING_LOTUS.0 as usize].wait_key,
        Some(id) if id.0 == ItemId::HEALING_LOTUS.0
    ));
    assert!(matches!(
        ITEMS[ItemId::MAGIC_WAND.0 as usize].wait_key,
        Some(id) if id.0 == ItemId::MAGIC_STICK.0
    ));
};

const _: () = {
    // A stocked item is bought whole, and is no part of a build.
    let mut at = 0;
    while at < ITEMS.len() {
        let def = &ITEMS[at];
        assert!(
            ward_sources_precede_item(def),
            "ward values must be inside the negotiated prefix"
        );
        let instant = def.point == 0 && def.backswing == 0 && def.duration == 0;
        assert!(!def.ignores_channel || instant);
        assert!(!def.immediate || instant);
        if let Some(stock) = ITEMS[at].stock {
            assert!(ITEMS[at].components.is_empty());
            assert!(stock.initial <= stock.max);
            assert!(stock.max > 0);
            assert!(stock.restock_ticks > 0);
            // A shelf that opens later gains its first one then.
            assert!(stock.opens_ticks == 0 || stock.initial < stock.max);
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
        // An item's strikes keep their rolls in places of their own, so many to an item.
        assert!(def.carried.strikes.len() <= MAX_ITEM_STRIKES);
        if let Some(ranged) = def.ranged {
            assert!(ranged.strikes.len() <= MAX_ITEM_STRIKES);
        }
        if def.stack_limit > 0 {
            assert!(def.charges >= 1 && def.charges <= def.stack_limit);
            assert!(def.components.is_empty() && def.alternatives.is_empty());
        }
        if let UseSpec::Replenish { mana } = def.effect {
            assert!(mana > 0);
        }
        // What an ally is given exists and is given out whole; what lands on an ally is an
        // effect that can land on one.
        if let Some(ally) = def.ally {
            assert!(def.cast_range > 0 && def.aim.is_some());
            match ally {
                AllyUse::Give { item } => {
                    assert!(def.charges > 0 || matches!(def.effect, UseSpec::Dispense));
                    if let Some(item) = item {
                        assert!(
                            ITEMS[item.0 as usize].lifetime > 0 && ITEMS[item.0 as usize].unbuyable
                        );
                    }
                }
                AllyUse::Effect { percent } => {
                    assert!(percent > 0 && percent <= 100);
                    assert!(matches!(
                        def.effect,
                        UseSpec::Mend(_) | UseSpec::Replenish { .. } | UseSpec::Eat { .. }
                    ));
                }
            }
        }
        assert!(def.lifetime == 0 || def.unbuyable);
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

/// Script values cross the private data boundary through catalogue rows. A const
/// constructor can read these without depending on the size of the placed table.
pub(crate) const fn script_values(id: ItemId) -> &'static [(&'static str, &'static [crate::Dec])] {
    let mut row = 0;
    while row < ITEM_ROWS.len() {
        if ITEM_ROWS[row].id.0 == id.0 {
            return ITEM_ROWS[row].values;
        }
        row += 1;
    }
    panic!("unknown item script")
}

pub(crate) const fn ward_sources_precede_item(item: &ItemDef) -> bool {
    if let crate::UseSpec::Ward { units, .. } = item.effect {
        let mut row = 0;
        while row < units.len() {
            if units[row].source_item().0 > item.id.0 {
                return false;
            }
            row += 1;
        }
    }
    true
}
