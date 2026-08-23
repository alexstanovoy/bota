//! What the client knows about abilities, items, effects and heroes.
//!
//! Names, blurbs and art only: every number an entry is worth rides the wire,
//! in the views for what a unit holds and in the shop table for what a thing
//! costs before anybody holds it.
//!
//! One entry to a thing, found by the id the wire carries. Anything the view
//! already brings -- level, mana cost, cooldown left, charges left -- is read
//! from the view; what stands here is what the wire does not send.

use bota_proto::{ItemId, ShopEntry};

/// One ability, as the client shows and aims it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbilityFace {
    /// The id it answers to on the wire.
    pub id: u16,
    /// Short name for the panel.
    pub name: &'static str,
    /// What it does, for the hover popup.
    pub blurb: &'static str,
    /// The drawing of it. Absent while there is none.
    pub icon: Option<&'static [u8]>,
}

/// One item, as the client shows and aims it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemFace {
    /// The id it answers to on the wire.
    pub id: u16,
    /// Short name for the panel and the shop.
    pub name: &'static str,
    /// Its one-line stats, for the shop row.
    pub stats: &'static str,
    /// What it does, for the hover popup.
    pub blurb: &'static str,
    /// The drawing of it. Absent while there is none.
    pub icon: Option<&'static [u8]>,
}

/// One timed effect, as the client shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectFace {
    /// The id it answers to on the wire.
    pub id: u16,
    /// Short name for the chip.
    pub name: &'static str,
    /// What it does, for the hover popup.
    pub blurb: &'static str,
    /// The drawing of it. Absent while there is none.
    pub icon: Option<&'static [u8]>,
}

/// One hero, as the client shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeroFace {
    /// The id it answers to on the wire.
    pub id: u16,
    /// Its name, for the roster and the panel.
    pub name: &'static str,
    /// The drawing of it. Absent while there is none.
    pub icon: Option<&'static [u8]>,
}

/// Every ability, in id order.
pub const ABILITIES: [AbilityFace; 17] = [
    AbilityFace {
        id: 0,
        name: "Crit",
        blurb: "Passive. 20/25/30/35% chance to strike for 175/200/225/250% damage.",
        icon: None,
    },
    AbilityFace {
        id: 1,
        name: "Frenzy",
        blurb: "No target. +20/28/36/44% attack speed for 6 s. 30/40/50/60 mana.",
        icon: None,
    },
    AbilityFace {
        id: 2,
        name: "Bounce",
        blurb: "Enemy target, range 550. 70/140/210/280 magic damage, then jumps to the 2/4/6/8 nearest new enemies.",
        icon: None,
    },
    AbilityFace {
        id: 3,
        name: "Volley",
        blurb: "Ultimate, no target. An attack at 80/100/120% damage flies at every enemy within 700.",
        icon: None,
    },
    AbilityFace {
        id: 4,
        name: "Hook",
        blurb: "Point target, range 1100. Catches the first unit in its way, drags it back and deals 90/180/270/360 pure damage.",
        icon: None,
    },
    AbilityFace {
        id: 5,
        name: "Rot",
        blurb: "Toggle. Burns everything within 250 for 30/60/90/120 a second and slows it, its owner included, but never kills its owner.",
        icon: None,
    },
    AbilityFace {
        id: 6,
        name: "Heap",
        blurb: "Passive. Magic resistance, and health for every death near you.",
        icon: None,
    },
    AbilityFace {
        id: 7,
        name: "Dismem",
        blurb: "Ultimate, enemy target, range 150. Holds it for 3 s, eating it and healing you.",
        icon: None,
    },
    AbilityFace {
        id: 8,
        name: "Burst",
        blurb: "No target. The courier flies 50% faster for 6 s. 120 s wait.",
        icon: None,
    },
    AbilityFace {
        id: 9,
        name: "Return",
        blurb: "No target. The courier puts what it holds back in the stash, then goes home.",
        icon: None,
    },
    AbilityFace {
        id: 10,
        name: "Stash",
        blurb: "No target. The courier takes what waits in your stash and carries it to you.",
        icon: None,
    },
    AbilityFace {
        id: 11,
        name: "Give",
        blurb: "No target. The courier carries what it holds to you, then goes home.",
        icon: None,
    },
    AbilityFace {
        id: 12,
        name: "Shield",
        blurb: "No target. Nothing gets through to the courier for 2 s. 200 s wait.",
        icon: None,
    },
    AbilityFace {
        id: 13,
        name: "Raze 1",
        blurb: "Point target. Burns everything within 250 of a spot 200 ahead for 90/160/230/300 magic damage. 10 s wait.",
        icon: None,
    },
    AbilityFace {
        id: 14,
        name: "Raze 2",
        blurb: "Point target. Burns everything within 250 of a spot 450 ahead for 90/160/230/300 magic damage. 10 s wait.",
        icon: None,
    },
    AbilityFace {
        id: 15,
        name: "Raze 3",
        blurb: "Point target. Burns everything within 250 of a spot 700 ahead for 90/160/230/300 magic damage. 10 s wait.",
        icon: None,
    },
    AbilityFace {
        id: 16,
        name: "Requiem",
        blurb: "Ultimate, no target. Every enemy within 900 takes 8/11/14 magic damage per soul held and is slowed for 2 s. The souls are kept.",
        icon: None,
    },
];

/// Every item the shop sells, in id order.
pub const ITEMS: [ItemFace; 42] = [
    ItemFace {
        id: 0,
        name: "Boots",
        stats: "+45 MS",
        blurb: "+45 movement speed.",
        icon: Some(include_bytes!("../assets/items/boots.svg")),
    },
    ItemFace {
        id: 1,
        name: "Clarity",
        stats: "150MP/25s",
        blurb: "Consumable. Restores 150 mana over 25 s. Any hero's hit breaks it.",
        icon: Some(include_bytes!("../assets/items/clarity.svg")),
    },
    ItemFace {
        id: 2,
        name: "Salve",
        stats: "400HP/10s",
        blurb: "Consumable. Restores 400 health over 10 s. Any hero's hit breaks it.",
        icon: Some(include_bytes!("../assets/items/healing_salve.svg")),
    },
    ItemFace {
        id: 3,
        name: "Branch",
        stats: "+30HP+15MP",
        blurb: "+30 maximum health, +15 maximum mana, +1 attack damage.",
        icon: Some(include_bytes!("../assets/items/iron_branch.svg")),
    },
    ItemFace {
        id: 4,
        name: "Obs",
        stats: "Vision",
        blurb: "Consumable. Stands a ward that sees 1600 and that the enemy cannot see.",
        icon: Some(include_bytes!("../assets/items/observer_ward.svg")),
    },
    ItemFace {
        id: 5,
        name: "Quell",
        stats: "+18 creep",
        blurb: "+18 attack damage against creeps. Fells the tree you point at.",
        icon: Some(include_bytes!("../assets/items/quelling_blade.svg")),
    },
    ItemFace {
        id: 6,
        name: "Sentry",
        stats: "True sight",
        blurb: "Consumable. Stands a ward that gives true sight, revealing enemy wards.",
        icon: Some(include_bytes!("../assets/items/sentry_ward.svg")),
    },
    ItemFace {
        id: 7,
        name: "Tango",
        stats: "115HP x3",
        blurb: "Three charges. Eats a tree to restore 115 health over 16 s.",
        icon: Some(include_bytes!("../assets/items/tango.svg")),
    },
    ItemFace {
        id: 8,
        name: "TP",
        stats: "Teleport",
        blurb: "Consumable. Channels, then carries you to an allied building.",
        icon: Some(include_bytes!("../assets/items/town_portal_scroll.svg")),
    },
    ItemFace {
        id: 9,
        name: "Circlet",
        stats: "+2 all",
        blurb: "+2 to every attribute.",
        icon: Some(include_bytes!("../assets/items/circlet.svg")),
    },
    ItemFace {
        id: 10,
        name: "Gauntlet",
        stats: "+3 STR",
        blurb: "+3 strength.",
        icon: Some(include_bytes!("../assets/items/gauntlets.svg")),
    },
    ItemFace {
        id: 11,
        name: "Slippers",
        stats: "+3 AGI",
        blurb: "+3 agility.",
        icon: Some(include_bytes!("../assets/items/slippers.svg")),
    },
    ItemFace {
        id: 12,
        name: "Mantle",
        stats: "+3 INT",
        blurb: "+3 intelligence.",
        icon: Some(include_bytes!("../assets/items/mantle.svg")),
    },
    ItemFace {
        id: 13,
        name: "Belt",
        stats: "+6 STR",
        blurb: "+6 strength.",
        icon: Some(include_bytes!("../assets/items/belt.svg")),
    },
    ItemFace {
        id: 14,
        name: "Band",
        stats: "+6 AGI",
        blurb: "+6 agility.",
        icon: Some(include_bytes!("../assets/items/band.svg")),
    },
    ItemFace {
        id: 15,
        name: "Robe",
        stats: "+6 INT",
        blurb: "+6 intelligence.",
        icon: Some(include_bytes!("../assets/items/robe.svg")),
    },
    ItemFace {
        id: 16,
        name: "Ogre Axe",
        stats: "+10 STR",
        blurb: "+10 strength.",
        icon: Some(include_bytes!("../assets/items/ogre_axe.svg")),
    },
    ItemFace {
        id: 17,
        name: "Alacrity",
        stats: "+10 AGI",
        blurb: "+10 agility.",
        icon: Some(include_bytes!("../assets/items/blade_of_alacrity.svg")),
    },
    ItemFace {
        id: 18,
        name: "Wizardry",
        stats: "+10 INT",
        blurb: "+10 intelligence.",
        icon: Some(include_bytes!("../assets/items/staff_of_wizardry.svg")),
    },
    ItemFace {
        id: 19,
        name: "Gloves",
        stats: "+20 AS",
        blurb: "+20 attack speed.",
        icon: Some(include_bytes!("../assets/items/gloves.svg")),
    },
    ItemFace {
        id: 20,
        name: "Blades",
        stats: "+9 DMG",
        blurb: "+9 attack damage.",
        icon: Some(include_bytes!("../assets/items/blades_of_attack.svg")),
    },
    ItemFace {
        id: 21,
        name: "Broad",
        stats: "+18 DMG",
        blurb: "+18 attack damage.",
        icon: Some(include_bytes!("../assets/items/broadsword.svg")),
    },
    ItemFace {
        id: 22,
        name: "Qstaff",
        stats: "+10DMG+10AS",
        blurb: "+10 attack damage, +10 attack speed.",
        icon: Some(include_bytes!("../assets/items/quarterstaff.svg")),
    },
    ItemFace {
        id: 23,
        name: "Ring Pro",
        stats: "+2 ARM",
        blurb: "+2 armor.",
        icon: Some(include_bytes!("../assets/items/ring_of_protection.svg")),
    },
    ItemFace {
        id: 24,
        name: "Chain",
        stats: "+5 ARM",
        blurb: "+5 armor.",
        icon: Some(include_bytes!("../assets/items/chainmail.svg")),
    },
    ItemFace {
        id: 25,
        name: "Regen",
        stats: "+2 HP/s",
        blurb: "+2 health a second.",
        icon: Some(include_bytes!("../assets/items/ring_of_regen.svg")),
    },
    ItemFace {
        id: 26,
        name: "Sage",
        stats: "+1 MP/s",
        blurb: "+1 mana a second.",
        icon: Some(include_bytes!("../assets/items/sages_mask.svg")),
    },
    ItemFace {
        id: 27,
        name: "Vitality",
        stats: "+250 HP",
        blurb: "+250 maximum health.",
        icon: Some(include_bytes!("../assets/items/vitality_booster.svg")),
    },
    ItemFace {
        id: 28,
        name: "Energy",
        stats: "+250 MP",
        blurb: "+250 maximum mana.",
        icon: Some(include_bytes!("../assets/items/energy_booster.svg")),
    },
    ItemFace {
        id: 29,
        name: "Treads",
        stats: "+45MS+25AS",
        blurb: "+45 movement speed, +25 attack speed, +10 to the attribute it is set to. Using them sets them to the next one.",
        icon: Some(include_bytes!("../assets/items/power_treads.svg")),
    },
    ItemFace {
        id: 30,
        name: "Phase",
        stats: "+45MS+18DMG",
        blurb: "+45 movement speed, +18 attack damage. Walks 20% faster and through bodies for 3 s. 8 s wait.",
        icon: Some(include_bytes!("../assets/items/phase_boots.svg")),
    },
    ItemFace {
        id: 31,
        name: "Blink",
        stats: "1200 jump",
        blurb: "Carries you to a point up to 1200 away. 15 s wait, and any hero's blow sets it back 3 s.",
        icon: Some(include_bytes!("../assets/items/blink_dagger.svg")),
    },
    ItemFace {
        id: 32,
        name: "Bracer",
        stats: "+6STR+3+3",
        blurb: "+6 strength, +3 agility, +3 intelligence.",
        icon: Some(include_bytes!("../assets/items/bracer.svg")),
    },
    ItemFace {
        id: 33,
        name: "Wraith",
        stats: "+6AGI+3+3",
        blurb: "+6 agility, +3 strength, +3 intelligence.",
        icon: Some(include_bytes!("../assets/items/wraith_band.svg")),
    },
    ItemFace {
        id: 34,
        name: "Null",
        stats: "+6INT+3+3",
        blurb: "+6 intelligence, +3 strength, +3 agility.",
        icon: Some(include_bytes!("../assets/items/null_talisman.svg")),
    },
    ItemFace {
        id: 35,
        name: "Stick",
        stats: "10 charges",
        blurb: "Gains a charge from every enemy cast within 1200. Spends them all to restore 15 health and mana each. 13 s wait.",
        icon: Some(include_bytes!("../assets/items/magic_stick.svg")),
    },
    ItemFace {
        id: 36,
        name: "Wand",
        stats: "+3 all",
        blurb: "+3 to every attribute. Gains a charge from every enemy cast within 1200, up to twenty. 13 s wait.",
        icon: Some(include_bytes!("../assets/items/magic_wand.svg")),
    },
    ItemFace {
        id: 37,
        name: "Rcp Phase",
        stats: "recipe",
        blurb: "Builds Phase Boots out of Boots and two Blades of Attack.",
        icon: Some(include_bytes!("../assets/items/recipe.svg")),
    },
    ItemFace {
        id: 38,
        name: "Rcp Bracer",
        stats: "recipe",
        blurb: "Builds a Bracer out of a Circlet and Gauntlets of Strength.",
        icon: Some(include_bytes!("../assets/items/recipe.svg")),
    },
    ItemFace {
        id: 39,
        name: "Rcp Wraith",
        stats: "recipe",
        blurb: "Builds a Wraith Band out of a Circlet and Slippers of Agility.",
        icon: Some(include_bytes!("../assets/items/recipe.svg")),
    },
    ItemFace {
        id: 40,
        name: "Rcp Null",
        stats: "recipe",
        blurb: "Builds a Null Talisman out of a Circlet and a Mantle of Intelligence.",
        icon: Some(include_bytes!("../assets/items/recipe.svg")),
    },
    ItemFace {
        id: 41,
        name: "Rcp Wand",
        stats: "recipe",
        blurb: "Builds a Magic Wand out of a Magic Stick and two Iron Branches.",
        icon: Some(include_bytes!("../assets/items/recipe.svg")),
    },
];

/// Every timed effect, in id order.
pub const EFFECTS: [EffectFace; 12] = [
    EffectFace {
        id: 0,
        name: "Frenzy",
        blurb: "Attack speed increased.",
        icon: None,
    },
    EffectFace {
        id: 1,
        name: "Mending",
        blurb: "Regenerating health.",
        icon: None,
    },
    EffectFace {
        id: 2,
        name: "Clarity",
        blurb: "Regenerating mana.",
        icon: None,
    },
    EffectFace {
        id: 3,
        name: "Fountain",
        blurb: "Regenerating health and mana for standing in the fountain.",
        icon: None,
    },
    EffectFace {
        id: 4,
        name: "Held",
        blurb: "Cannot move, attack or cast.",
        icon: None,
    },
    EffectFace {
        id: 5,
        name: "Slowed",
        blurb: "Movement speed reduced.",
        icon: None,
    },
    EffectFace {
        id: 6,
        name: "Burning",
        blurb: "Losing health over time.",
        icon: None,
    },
    EffectFace {
        id: 7,
        name: "Hastened",
        blurb: "Movement speed increased.",
        icon: None,
    },
    EffectFace {
        id: 8,
        name: "Shielded",
        blurb: "Nothing gets through.",
        icon: None,
    },
    EffectFace {
        id: 9,
        name: "Phased",
        blurb: "Walking through the bodies in the way.",
        icon: None,
    },
    EffectFace {
        id: 10,
        name: "Heap",
        blurb: "Health kept from every death nearby.",
        icon: None,
    },
    EffectFace {
        id: 11,
        name: "Souls",
        blurb: "Attack damage from every unit brought down.",
        icon: None,
    },
];

/// Every hero that can be picked, in id order.
pub const HEROES: [HeroFace; 3] = [
    HeroFace {
        id: 0,
        name: "Sylla",
        icon: None,
    },
    HeroFace {
        id: 1,
        name: "Pudge",
        icon: None,
    },
    HeroFace {
        id: 2,
        name: "Shadow Fiend",
        icon: None,
    },
];

/// What one item is built from, by id.
fn components_of(shop: &[ShopEntry], item: ItemId) -> &[ItemId] {
    shop.iter()
        .find(|entry| entry.id == item)
        .map_or(&[][..], |entry| &entry.components)
}

/// What the shop asks for one item whole, before anything already in hand.
pub fn whole_price(shop: &[ShopEntry], item: ItemId) -> i32 {
    shop.iter()
        .find(|entry| entry.id == item)
        .map_or(0, |entry| entry.cost)
}

/// What the shop asks a seat holding `held` for one item.
///
/// The rule the server charges by: what was asked for is bought however many
/// of it are already held, and only its parts are looked for in hand.
pub fn price_for(shop: &[ShopEntry], item: ItemId, held: &[ItemId]) -> i32 {
    let mut spare = held.to_vec();
    let mut wanted = Vec::new();
    match components_of(shop, item) {
        [] => wanted.push(item),
        parts => {
            for part in parts {
                parts_beyond(shop, *part, &mut spare, &mut wanted);
            }
        }
    }
    wanted.iter().map(|part| whole_price(shop, *part)).sum()
}

/// Lays out what one part still costs, spending `held` as it goes.
fn parts_beyond(
    shop: &[ShopEntry],
    item: ItemId,
    held: &mut Vec<ItemId>,
    wanted: &mut Vec<ItemId>,
) {
    if let Some(at) = held.iter().position(|id| *id == item) {
        held.remove(at);
        return;
    }
    match components_of(shop, item) {
        [] => wanted.push(item),
        parts => {
            for part in parts {
                parts_beyond(shop, *part, held, wanted);
            }
        }
    }
}

/// The ability of that id, or nothing for one the catalog does not hold.
pub fn ability(id: u16) -> Option<&'static AbilityFace> {
    ABILITIES.get(usize::from(id))
}

/// The item of that id, or nothing for one the catalog does not hold.
pub fn item(id: u16) -> Option<&'static ItemFace> {
    ITEMS.get(usize::from(id))
}

/// The effect of that id, or nothing for one the catalog does not hold.
pub fn effect(id: u16) -> Option<&'static EffectFace> {
    EFFECTS.get(usize::from(id))
}

/// The hero of that id, or nothing for one the catalog does not hold.
pub fn hero(id: u16) -> Option<&'static HeroFace> {
    HEROES.get(usize::from(id))
}
