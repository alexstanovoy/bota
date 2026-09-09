//! What the bot buys, and the little it needs to know about items.
//!
//! Kept apart from the deeds because it is the one piece of taste in the list:
//! "buy" is a single deed, and what that buys is a shopping list rather than a
//! decision the model makes. When the model should be choosing the item too,
//! this becomes a block of deeds and nothing else moves.
//!
//! Prices and parts are written out here rather than read off the wire, because
//! neither crosses it: a snapshot carries which item sits in a slot and nothing
//! about what it cost. So this table mirrors the server's by hand, and nothing
//! checks that it still does. A price that drifts is not an error anywhere — it
//! is a net worth quietly counted wrong.

use bota_proto::ItemId;

use crate::Field;

/// Boots of Speed.
pub const BOOTS: u16 = 0;
/// Clarity.
pub const CLARITY: u16 = 1;
/// Healing Salve.
pub const SALVE: u16 = 2;
/// Iron Branch.
pub const BRANCH: u16 = 3;
/// Observer Ward.
pub const OBSERVER: u16 = 4;
/// Quelling Blade.
pub const QUELLING: u16 = 5;
/// Sentry Ward.
pub const SENTRY: u16 = 6;
/// Tango.
pub const TANGO: u16 = 7;
/// Town Portal Scroll.
pub const SCROLL: u16 = 8;
/// Circlet.
pub const CIRCLET: u16 = 9;
/// Gauntlets of Strength.
pub const GAUNTLETS: u16 = 10;
/// Slippers of Agility.
pub const SLIPPERS: u16 = 11;
/// Mantle of Intelligence.
pub const MANTLE: u16 = 12;
/// Belt of Strength.
pub const BELT: u16 = 13;
/// Band of Elvenskin.
pub const BAND: u16 = 14;
/// Robe of the Magi.
pub const ROBE: u16 = 15;
/// Ogre Axe.
pub const OGRE_AXE: u16 = 16;
/// Blade of Alacrity.
pub const ALACRITY: u16 = 17;
/// Staff of Wizardry.
pub const WIZARDRY: u16 = 18;
/// Gloves of Haste.
pub const GLOVES: u16 = 19;
/// Blades of Attack.
pub const BLADES: u16 = 20;
/// Broadsword.
pub const BROADSWORD: u16 = 21;
/// Quarterstaff.
pub const QUARTERSTAFF: u16 = 22;
/// Ring of Protection.
pub const RING_OF_PROTECTION: u16 = 23;
/// Chainmail.
pub const CHAINMAIL: u16 = 24;
/// Ring of Regeneration.
pub const RING_OF_REGEN: u16 = 25;
/// Sage's Mask.
pub const SAGES_MASK: u16 = 26;
/// Vitality Booster.
pub const VITALITY: u16 = 27;
/// Energy Booster.
pub const ENERGY: u16 = 28;
/// Power Treads.
pub const TREADS: u16 = 29;
/// Phase Boots.
pub const PHASE_BOOTS: u16 = 30;
/// Blink Dagger.
pub const BLINK: u16 = 31;
/// Bracer.
pub const BRACER: u16 = 32;
/// Wraith Band.
pub const WRAITH_BAND: u16 = 33;
/// Null Talisman.
pub const NULL_TALISMAN: u16 = 34;
/// Magic Stick.
pub const MAGIC_STICK: u16 = 35;
/// Magic Wand.
pub const MAGIC_WAND: u16 = 36;
/// The recipe Phase Boots are finished with.
pub const RECIPE_PHASE_BOOTS: u16 = 37;
/// The recipe a Bracer is finished with.
pub const RECIPE_BRACER: u16 = 38;
/// The recipe a Wraith Band is finished with.
pub const RECIPE_WRAITH_BAND: u16 = 39;
/// The recipe a Null Talisman is finished with.
pub const RECIPE_NULL_TALISMAN: u16 = 40;
/// The recipe a Magic Wand is finished with.
pub const RECIPE_MAGIC_WAND: u16 = 41;

/// How many items the shop sells.
pub const ITEMS_SOLD: usize = 42;

/// Slots a hero carries on itself, the backpack counted in.
pub const BAG_SLOTS: usize = 9;

/// Slots of the bag where an item works: the inventory proper, before the
/// backpack begins.
pub const WORN_SLOTS: usize = 6;

/// Slots the stash at the home shop holds.
pub const STASH_SLOTS: usize = 6;

/// What each item costs, by its number.
///
/// Every item the shop sells, so that what the seat owns can be added up
/// whatever it owns. Left short, an item missing from here is worth nothing at
/// all to the reckoning, and a dagger of two thousand two hundred and fifty
/// counts the same as an empty slot.
const PRICES: [i32; ITEMS_SOLD] = [
    500,  // boots of speed
    50,   // clarity
    110,  // healing salve
    50,   // iron branch
    100,  // observer ward
    225,  // quelling blade
    50,   // sentry ward
    90,   // tango
    100,  // town portal scroll
    155,  // circlet
    140,  // gauntlets of strength
    140,  // slippers of agility
    140,  // mantle of intelligence
    450,  // belt of strength
    450,  // band of elvenskin
    450,  // robe of the magi
    1000, // ogre axe
    1000, // blade of alacrity
    1000, // staff of wizardry
    450,  // gloves of haste
    450,  // blades of attack
    1000, // broadsword
    875,  // quarterstaff
    175,  // ring of protection
    550,  // chainmail
    175,  // ring of regeneration
    175,  // sage's mask
    1000, // vitality booster
    900,  // energy booster
    1400, // power treads
    1500, // phase boots
    2250, // blink dagger
    505,  // bracer
    505,  // wraith band
    505,  // null talisman
    200,  // magic stick
    450,  // magic wand
    100,  // recipe: phase boots
    210,  // recipe: bracer
    210,  // recipe: wraith band
    210,  // recipe: null talisman
    150,  // recipe: magic wand
];

/// What is built from what.
///
/// The server puts a build together by itself the moment its parts are all in
/// the bag, so the bot never asks for one — it buys the parts. Which means it
/// has to know what went into a build it is holding, or it would see the parts
/// gone and buy them again for ever.
const BUILDS: [(u16, &[u16]); 6] = [
    (TREADS, &[BOOTS, GLOVES, BELT]),
    (PHASE_BOOTS, &[BOOTS, BLADES, BLADES, RECIPE_PHASE_BOOTS]),
    (BRACER, &[CIRCLET, GAUNTLETS, RECIPE_BRACER]),
    (WRAITH_BAND, &[CIRCLET, SLIPPERS, RECIPE_WRAITH_BAND]),
    (NULL_TALISMAN, &[CIRCLET, MANTLE, RECIPE_NULL_TALISMAN]),
    (
        MAGIC_WAND,
        &[MAGIC_STICK, BRANCH, BRANCH, RECIPE_MAGIC_WAND],
    ),
];

/// What the bot wants to own, in the order it wants it.
///
/// Parts rather than builds, since a build is put together on its own once its
/// parts are held: boots, gloves and a belt become Power Treads without anybody
/// asking. The order is the build order, so what is wanted first is bought
/// first and nothing waits on gold for something further down.
pub const SHOPPING: [u16; 14] = [
    TANGO,
    BRANCH,
    BRANCH,
    SALVE,
    QUELLING,
    BOOTS,
    MAGIC_STICK,
    GLOVES,
    BELT,
    CIRCLET,
    SLIPPERS,
    RECIPE_WRAITH_BAND,
    RECIPE_MAGIC_WAND,
    CLARITY,
];

/// A courier's turn of speed.
pub const BURST: u16 = 8;
/// A courier's walk home.
pub const GO_HOME: u16 = 9;
/// A courier taking what waits in the stash.
pub const TAKE_STASH: u16 = 10;
/// A courier handing over what it carries.
pub const DELIVER: u16 = 11;

/// The items that can be used at all.
///
/// A snapshot says which item sits in a slot and how long it has left to wait,
/// but not whether it has any use to begin with. Offering one that has none is
/// a tick spent on an order the server will not take.
const USABLE: [u16; 13] = [
    CLARITY,
    SALVE,
    BRANCH,
    OBSERVER,
    QUELLING,
    SENTRY,
    TANGO,
    SCROLL,
    TREADS,
    PHASE_BOOTS,
    BLINK,
    MAGIC_STICK,
    MAGIC_WAND,
];

/// Whether an item is one that can be used.
pub fn can_be_used(item: u16) -> bool {
    USABLE.contains(&item)
}

/// The items that are spent by using them.
pub const CONSUMED: [u16; 6] = [CLARITY, SALVE, OBSERVER, SENTRY, TANGO, SCROLL];

/// Whether an item is spent by using it.
pub fn is_consumed(item: u16) -> bool {
    CONSUMED.contains(&item)
}

/// What one item costs. Nothing at all for a number the shop does not sell.
pub fn cost_of(item: u16) -> i32 {
    PRICES.get(usize::from(item)).copied().unwrap_or(0)
}

/// What everything the seat owns cost.
///
/// The bag, the stash and whatever the courier is carrying, because gold is
/// spent the moment an item is bought and where it sits afterwards is the
/// courier's business.
pub fn worth_of_goods(field: &Field) -> i32 {
    let (worn, riding) = worth_worn_and_riding(field);
    worn + riding + worth_of_consumables(field)
}

/// What the seat's durable goods cost, in two piles: what is worn — the
/// working slots of the bag — and what only rides along in the backpack, the
/// stash or the courier's load. Consumables sit in neither pile;
/// [`worth_of_consumables`] carries them.
pub fn worth_worn_and_riding(field: &Field) -> (i32, i32) {
    let worth = |slots: &[Option<bota_proto::ItemView>]| {
        slots
            .iter()
            .flatten()
            .filter(|had| !is_consumed(had.id.0))
            .map(|had| cost_of(had.id.0))
            .sum::<i32>()
    };
    let bag = field.me.map_or(&[][..], |me| &me.items[..]);
    let split = bag.len().min(WORN_SLOTS);
    let worn = worth(&bag[..split]);
    let riding = worth(&bag[split..])
        + field.seat.stash.as_ref().map_or(0, |slots| worth(slots))
        + field.courier.map_or(0, |bird| worth(&bird.items));
    (worn, riding)
}

/// What the seat's consumables cost, wherever they sit.
pub fn worth_of_consumables(field: &Field) -> i32 {
    let worth = |slots: &[Option<bota_proto::ItemView>]| {
        slots
            .iter()
            .flatten()
            .filter(|had| is_consumed(had.id.0))
            .map(|had| cost_of(had.id.0))
            .sum::<i32>()
    };
    field.me.map_or(0, |me| worth(&me.items))
        + field.seat.stash.as_ref().map_or(0, |slots| worth(slots))
        + field.courier.map_or(0, |bird| worth(&bird.items))
}

/// How many of an item the seat holds, counting the ones inside builds.
///
/// A part that has gone into a build is still a part the bot paid for. Counted
/// only where it can be seen, the boots that became Power Treads look unbought,
/// and the bot buys another pair every time it can afford one.
pub fn how_many_held(field: &Field, item: u16) -> usize {
    let counted = |slots: &[Option<bota_proto::ItemView>]| {
        slots
            .iter()
            .flatten()
            .map(|had| {
                usize::from(had.id.0 == item)
                    + BUILDS
                        .iter()
                        .filter(|(built, _)| *built == had.id.0)
                        .map(|(_, parts)| parts.iter().filter(|part| **part == item).count())
                        .sum::<usize>()
            })
            .sum::<usize>()
    };
    let bag = field.me.map_or(0, |me| counted(&me.items));
    let stash = field.seat.stash.as_ref().map_or(0, |slots| counted(slots));
    let carried = field.courier.map_or(0, |bird| counted(&bird.items));
    bag + stash + carried
}

/// The next thing on the list that is not owned and can be paid for.
///
/// What is on its way counts as owned: the stash and the courier are both
/// holding things the bot is about to have, and buying a second of each is how
/// gold gets wasted.
pub fn next_to_buy(field: &Field) -> Option<ItemId> {
    let gold = field.seat.gold.unwrap_or(0);
    let wanted = |item: u16| SHOPPING.iter().filter(|listed| **listed == item).count();
    SHOPPING
        .iter()
        .find(|item| how_many_held(field, **item) < wanted(**item) && gold >= cost_of(**item))
        .map(|item| ItemId(*item))
}
