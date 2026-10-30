//! Taking a combined item apart and locking its parts against combining, as
//! Dota 7.41f does. The rules, with their sources:
//!
//! - Which items: only one built of parts. Its `ItemDisassembleRule` in the
//!   game's `scripts/npc/items.txt` (client 6957) decides when: `NEVER` never
//!   (Ward Dispenser, Hand of Midas, the Rapiers, Aghanim's Scepter, ...),
//!   `ALWAYS` at any time (Echo Sabre, Radiance, Vanguard, ...), and an item
//!   that names none within ten seconds of being combined (Liquipedia, *Item
//!   Disassembling*). Patch 7.41 took Ethereal Blade, Heaven's Halberd and
//!   Lotus Orb from the always list.
//! - That window ends at once when the item is used, when its carrier dies and
//!   when it enters the bag of a unit its owner does not control (Liquipedia).
//!   An item that came of a disassembly has none.
//! - Where: in the hero's inventory or backpack, on the courier and in the
//!   stash, which wants the unit in reach of it as swapping does (the game's
//!   `dota_hud_error_cant_disassemble_stash_out_of_range`). It works while the
//!   hero lies dead (Liquipedia), on the stash and on what the hero carried.
//!   It is an instant order: the body goes on with what it was doing.
//! - What comes back: the parts of the recipe the item was built by (the one
//!   its attribute names for an item with several), the recipe scroll
//!   included, each a fresh stack with no wait of its own; charges an item
//!   gathered (a Magic Wand's) go back to the parts that gather them, up to
//!   what each holds. They land where the item was, then in the unit's other
//!   free slots, and what does not fit on the ground at the unit (Liquipedia:
//!   "tries to fill your inventory or backpack first before dropping to the
//!   ground").
//! - Every part is locked against combining (the game's
//!   `DOTA_Glossary_Items_Assembling_Disassembly_Desc`: "all component Items
//!   are automatically Combine Locked").
//! - Locking: a locked item "will be ignored as components during Auto
//!   Combine" (`DOTA_Glossary_Items_Assembling_CombineLock_Desc`), in a bag,
//!   the stash and on the courier, and in what a purchase counts as held, so a
//!   purchase never promises a build the assembly would then refuse. Locked
//!   items sell, move, are carried by the courier and are used as ever. A lock
//!   is a choice of the owner's, set or cleared by an order, and a build
//!   comes unlocked.

use bota_proto::{Attribute, Disassembly, ItemId};

use crate::{DisassembleRule, ITEMS, item_def};

/// When an item with `window_left` ticks of its window left may be taken
/// apart, or nothing when it may not.
pub fn disassembly(id: ItemId, window_left: u32) -> Option<Disassembly> {
    let def = item_def(id)?;
    if def.components.is_empty() {
        return None;
    }
    match def.disassemble {
        DisassembleRule::Never => None,
        DisassembleRule::Always => Some(Disassembly::Always),
        DisassembleRule::Window => (window_left > 0).then_some(Disassembly::Window {
            ticks_left: window_left,
        }),
    }
}

/// The parts a build comes apart into: those of the recipe set to the
/// attribute it is set to, the first recipe when none is.
pub fn parts_of(id: ItemId, mode: Option<Attribute>) -> &'static [ItemId] {
    let Some(def) = item_def(id) else {
        return &[];
    };
    def.recipes()
        .find(|recipe| recipe.mode == mode)
        .or_else(|| def.recipes().next())
        .map_or(&[], |recipe| recipe.parts)
}

/// Whether an item is a part of some recipe, which is what a lock on it does
/// anything to.
pub fn is_part_of_a_recipe(id: ItemId) -> bool {
    ITEMS
        .iter()
        .filter(|def| !def.retired)
        .flat_map(|def| def.recipes())
        .any(|recipe| recipe.parts.contains(&id))
}
