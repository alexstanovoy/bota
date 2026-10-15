//! What the client knows about abilities, items, effects and heroes.
//!
//! Names, blurbs and art only, one entry to a thing, found by the id the wire
//! carries. The numbers are the rules': `bota_core` holds what a thing costs
//! and does, and the views tell what a unit holds.

use bota_core::{attributes_at_level, base_damage_least, hero_def};
use bota_proto::{AbilityId, EffectId, Fixed, HeroId, ItemId};

/// The shape a projectile of an ability is drawn as. Radii are in world units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Art {
    /// A missile: a dot.
    Missile,
    /// A burst on the ground, `radius` wide.
    Burst { radius: u16 },
    /// A soul in flight: a streak along its facing.
    Soul,
    /// One link of a chain.
    Link,
    /// A hold on the unit it stands on.
    Hold,
    /// A cloud on the ground, `radius` wide.
    Cloud { radius: u16 },
}

/// One ability, as the client shows and aims it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbilityFace {
    /// The id it answers to on the wire.
    pub id: u16,
    /// Short name for the panel.
    pub name: &'static str,
    /// What it does, for the hover popup.
    pub blurb: &'static str,
    /// How what it shows on the ground is drawn.
    pub art: Art,
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
    /// Whether it hurts whoever wears it: a debuff, drawn apart from the
    /// buffs.
    pub harmful: bool,
}

/// One hero, as the client shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeroFace {
    /// The id it answers to on the wire.
    pub id: u16,
    /// Its name, for the roster and the panel.
    pub name: &'static str,
}

/// Every ability, in id order.
pub const ABILITIES: [AbilityFace; 17] = [
    AbilityFace {
        id: AbilityId::MEAT_HOOK.0,
        name: "Hook",
        blurb: "Point target, range 1300. Catches the first unit in its way, drags it back and deals 90/180/270/360 pure damage.",
        art: Art::Link,
    },
    AbilityFace {
        id: AbilityId::ROT.0,
        name: "Rot",
        blurb: "Toggle. Burns every enemy within 250 for 30/60/90/120 a second and slows it 10/15/20/25%. It burns its owner for the same, but never kills its owner.",
        art: Art::Cloud { radius: 250 },
    },
    AbilityFace {
        id: AbilityId::FLESH_HEAP.0,
        name: "Heap",
        blurb: "Passive. 12/14/16/18% magic resistance on top of your own, and +1.5/2/2.5/3 strength for every enemy hero that dies within 450 of you, whoever brought it down. Stacks are kept through your own death.",
        art: Art::Missile,
    },
    AbilityFace {
        id: AbilityId::DISMEMBER.0,
        name: "Dismem",
        blurb: "Ultimate, enemy target, range 200. Holds it for 3 s, eating it and healing you.",
        art: Art::Hold,
    },
    AbilityFace {
        id: AbilityId::BURST.0,
        name: "Burst",
        blurb: "No target. The courier flies 50% faster for 6 s. 120 s wait.",
        art: Art::Missile,
    },
    AbilityFace {
        id: AbilityId::RETURN_ITEMS.0,
        name: "Return",
        blurb: "No target. The courier puts what it holds back in the stash, then goes home.",
        art: Art::Missile,
    },
    AbilityFace {
        id: AbilityId::TAKE_STASH.0,
        name: "Stash",
        blurb: "No target. The courier takes what waits in your stash and carries it to you.",
        art: Art::Missile,
    },
    AbilityFace {
        id: AbilityId::DELIVER.0,
        name: "Give",
        blurb: "No target. The courier carries what it holds to you, then goes home.",
        art: Art::Missile,
    },
    AbilityFace {
        id: AbilityId::SHIELD.0,
        name: "Shield",
        blurb: "No target. Nothing gets through to the courier for 2 s. 200 s wait.",
        art: Art::Missile,
    },
    AbilityFace {
        id: AbilityId::RAZE_NEAR.0,
        name: "Raze 1",
        blurb: "No target, 0.55 s cast point. Burns every enemy but a building within 250 of a spot 200 ahead of where you face for 85/150/215/280 magic damage, plus 2 per soul held and 35/50/65/80 per prior same-caster stack. Each damaging hit refreshes all stacks for 6 s. One point levels all three razes. 75 mana, 9 s wait.",
        art: Art::Burst { radius: 250 },
    },
    AbilityFace {
        id: AbilityId::RAZE_MID.0,
        name: "Raze 2",
        blurb: "No target, 0.55 s cast point. Burns every enemy but a building within 250 of a spot 450 ahead of where you face for 85/150/215/280 magic damage, plus 2 per soul held and 35/50/65/80 per prior same-caster stack. Each damaging hit refreshes all stacks for 6 s. One point levels all three razes. 75 mana, 9 s wait.",
        art: Art::Burst { radius: 250 },
    },
    AbilityFace {
        id: AbilityId::RAZE_FAR.0,
        name: "Raze 3",
        blurb: "No target, 0.55 s cast point. Burns every enemy but a building within 250 of a spot 700 ahead of where you face for 85/150/215/280 magic damage, plus 2 per soul held and 35/50/65/80 per prior same-caster stack. Each damaging hit refreshes all stacks for 6 s. One point levels all three razes. 75 mana, 9 s wait.",
        art: Art::Burst { radius: 250 },
    },
    AbilityFace {
        id: AbilityId::REQUIEM.0,
        name: "Requiem",
        blurb: "Ultimate, no target, 1.67 s cast point. Lets a line fly out for every soul held, up to 20, out to 1000. Each line burns every enemy but a building it crosses for 80/120/160 magic damage, and every hit adds 0.6 s of fear, a 20/25/30% slow and 5/10/15% less magic resistance, up to 2.15 s. What stands close is crossed by many lines. The souls are kept. On your death, half the souls you keep fly out as lines that only burn.",
        art: Art::Soul,
    },
    AbilityFace {
        id: AbilityId::NECROMASTERY.0,
        name: "Necro",
        blurb: "Innate. A soul is kept of everything you bring down, four for a hero, each worth 1 attack damage and 0.8 more every 6 levels. Holds up to 20; a death lets 30% of them go.",
        art: Art::Missile,
    },
    AbilityFace {
        id: AbilityId::PRESENCE.0,
        name: "Presence",
        blurb: "Passive. Enemies within 1200 that see you wear 2.5/4/5.5/7 less armor.",
        art: Art::Missile,
    },
    AbilityFace {
        id: AbilityId::FEAST_OF_SOULS.0,
        name: "Feast",
        blurb: "No target, goes off beside whatever you are doing. For 8 s: +35/50/65/80 attack speed and +4/6/8/10% movement speed, and every 0.5 s souls from two enemies within 600 you have not fed on yet, heroes first: 3 a hero, 1 a creep, from up to 4/6/8/10 enemies. Then the souls of those still standing go; the rest stay 8 s past the cap. 60/65/70/75 mana, 21 s wait.",
        art: Art::Missile,
    },
    AbilityFace {
        id: AbilityId::THROW_GRENADE.0,
        name: "Grenade",
        blurb: "A Blood Grenade in flight, at 1100 a second, bursting 300 wide where it lands.",
        art: Art::Missile,
    },
];

/// Every item the shop sells, in id order.
pub const ITEMS: [ItemFace; 54] = [
    ItemFace {
        id: ItemId::BOOTS.0,
        name: "Boots",
        stats: "+45 MS",
        blurb: "+45 movement speed.",
    },
    ItemFace {
        id: ItemId::CLARITY.0,
        name: "Clarity",
        stats: "150MP/25s",
        blurb: "Consumable. Restores 150 mana over 25 s. A blow from a hero or a tower breaks it.",
    },
    ItemFace {
        id: ItemId::HEALING_SALVE.0,
        name: "Salve",
        stats: "400HP/10s",
        blurb: "Consumable. Restores 400 health over 10 s. A blow from a hero or a tower breaks it.",
    },
    ItemFace {
        id: ItemId::IRON_BRANCH.0,
        name: "Branch",
        stats: "+30HP+15MP",
        blurb: "+30 maximum health, +15 maximum mana, +1 attack damage.",
    },
    ItemFace {
        id: ItemId::OBSERVER_WARD.0,
        name: "Obs",
        stats: "Vision",
        blurb: "Consumable. Stands a ward that sees 1600 and that the enemy cannot see.",
    },
    ItemFace {
        id: ItemId::QUELLING_BLADE.0,
        name: "Quell",
        stats: "+18 creep",
        blurb: "+18 attack damage against creeps. Fells the tree you point at.",
    },
    ItemFace {
        id: ItemId::SENTRY_WARD.0,
        name: "Sentry",
        stats: "True sight",
        blurb: "Consumable. Stands a ward that gives true sight, revealing enemy wards.",
    },
    ItemFace {
        id: ItemId::TANGO.0,
        name: "Tango",
        stats: "115HP x3",
        blurb: "Three charges. Eats a tree to restore 115 health over 16 s.",
    },
    ItemFace {
        id: ItemId::TOWN_PORTAL_SCROLL.0,
        name: "TP",
        stats: "Teleport",
        blurb: "Consumable. Channels, then carries you to an allied building.",
    },
    ItemFace {
        id: ItemId::CIRCLET.0,
        name: "Circlet",
        stats: "+2 all",
        blurb: "+2 to every attribute.",
    },
    ItemFace {
        id: ItemId::GAUNTLETS.0,
        name: "Gauntlet",
        stats: "+3 STR",
        blurb: "+3 strength.",
    },
    ItemFace {
        id: ItemId::SLIPPERS.0,
        name: "Slippers",
        stats: "+3 AGI",
        blurb: "+3 agility.",
    },
    ItemFace {
        id: ItemId::MANTLE.0,
        name: "Mantle",
        stats: "+3 INT",
        blurb: "+3 intelligence.",
    },
    ItemFace {
        id: ItemId::BELT.0,
        name: "Belt",
        stats: "+6 STR",
        blurb: "+6 strength.",
    },
    ItemFace {
        id: ItemId::BAND.0,
        name: "Band",
        stats: "+6 AGI",
        blurb: "+6 agility.",
    },
    ItemFace {
        id: ItemId::ROBE.0,
        name: "Robe",
        stats: "+6 INT",
        blurb: "+6 intelligence.",
    },
    ItemFace {
        id: ItemId::OGRE_AXE.0,
        name: "Ogre Axe",
        stats: "+10 STR",
        blurb: "+10 strength.",
    },
    ItemFace {
        id: ItemId::BLADE_OF_ALACRITY.0,
        name: "Alacrity",
        stats: "+10 AGI",
        blurb: "+10 agility.",
    },
    ItemFace {
        id: ItemId::STAFF_OF_WIZARDRY.0,
        name: "Wizardry",
        stats: "+10 INT",
        blurb: "+10 intelligence.",
    },
    ItemFace {
        id: ItemId::GLOVES.0,
        name: "Gloves",
        stats: "+20 AS",
        blurb: "+20 attack speed.",
    },
    ItemFace {
        id: ItemId::BLADES_OF_ATTACK.0,
        name: "Blades",
        stats: "+9 DMG",
        blurb: "+9 attack damage.",
    },
    ItemFace {
        id: ItemId::BROADSWORD.0,
        name: "Broad",
        stats: "+18 DMG",
        blurb: "+18 attack damage.",
    },
    ItemFace {
        id: ItemId::QUARTERSTAFF.0,
        name: "Qstaff",
        stats: "+10DMG+10AS",
        blurb: "+10 attack damage, +10 attack speed.",
    },
    ItemFace {
        id: ItemId::RING_OF_PROTECTION.0,
        name: "Ring Pro",
        stats: "+2 ARM",
        blurb: "+2 armor.",
    },
    ItemFace {
        id: ItemId::CHAINMAIL.0,
        name: "Chain",
        stats: "+5 ARM",
        blurb: "+5 armor.",
    },
    ItemFace {
        id: ItemId::RING_OF_REGEN.0,
        name: "Regen",
        stats: "+2 HP/s",
        blurb: "+2 health a second.",
    },
    ItemFace {
        id: ItemId::SAGES_MASK.0,
        name: "Sage",
        stats: "+1 MP/s",
        blurb: "+1 mana a second.",
    },
    ItemFace {
        id: ItemId::VITALITY_BOOSTER.0,
        name: "Vitality",
        stats: "+250 HP",
        blurb: "+250 maximum health.",
    },
    ItemFace {
        id: ItemId::ENERGY_BOOSTER.0,
        name: "Energy",
        stats: "+250 MP",
        blurb: "+250 maximum mana.",
    },
    ItemFace {
        id: ItemId::POWER_TREADS.0,
        name: "Treads",
        stats: "+45MS+25AS",
        blurb: "+45 movement speed, +25 attack speed, +10 to the attribute it is set to. Using them sets them to the next one.",
    },
    ItemFace {
        id: ItemId::PHASE_BOOTS.0,
        name: "Phase",
        stats: "+45MS+18DMG",
        blurb: "+45 movement speed, +18 attack damage. Walks 20% faster and through bodies for 3.1 s. 8 s wait.",
    },
    ItemFace {
        id: ItemId::BLINK_DAGGER.0,
        name: "Blink",
        stats: "1200 jump",
        blurb: "Carries you to a point up to 1200 away. 15 s wait, and a blow from a hero or a tower leaves it at least 3 s from ready.",
    },
    ItemFace {
        id: ItemId::BRACER.0,
        name: "Bracer",
        stats: "+6STR+3+3",
        blurb: "+6 strength, +3 agility, +3 intelligence.",
    },
    ItemFace {
        id: ItemId::WRAITH_BAND.0,
        name: "Wraith",
        stats: "+6AGI+3+3",
        blurb: "+6 agility, +3 strength, +3 intelligence.",
    },
    ItemFace {
        id: ItemId::NULL_TALISMAN.0,
        name: "Null",
        stats: "+6INT+3+3",
        blurb: "+6 intelligence, +3 strength, +3 agility.",
    },
    ItemFace {
        id: ItemId::MAGIC_STICK.0,
        name: "Stick",
        stats: "10 charges",
        blurb: "Gains a charge from every enemy cast within 1200. Spends them all to restore 15 health and mana each. 13 s wait.",
    },
    ItemFace {
        id: ItemId::MAGIC_WAND.0,
        name: "Wand",
        stats: "+3 all",
        blurb: "+3 to every attribute. Gains a charge from every enemy cast within 1200, up to twenty. 13 s wait.",
    },
    ItemFace {
        id: ItemId::RECIPE_PHASE_BOOTS.0,
        name: "Rcp Phase",
        stats: "recipe",
        blurb: "Builds Phase Boots out of Boots and two Blades of Attack.",
    },
    ItemFace {
        id: ItemId::RECIPE_BRACER.0,
        name: "Rcp Bracer",
        stats: "recipe",
        blurb: "Builds a Bracer out of a Circlet and Gauntlets of Strength.",
    },
    ItemFace {
        id: ItemId::RECIPE_WRAITH_BAND.0,
        name: "Rcp Wraith",
        stats: "recipe",
        blurb: "Builds a Wraith Band out of a Circlet and Slippers of Agility.",
    },
    ItemFace {
        id: ItemId::RECIPE_NULL_TALISMAN.0,
        name: "Rcp Null",
        stats: "recipe",
        blurb: "Builds a Null Talisman out of a Circlet and a Mantle of Intelligence.",
    },
    ItemFace {
        id: ItemId::RECIPE_MAGIC_WAND.0,
        name: "Rcp Wand",
        stats: "recipe",
        blurb: "Builds a Magic Wand out of a Magic Stick and two Iron Branches.",
    },
    ItemFace {
        id: ItemId::MANGO.0,
        name: "Mango",
        stats: "+0.4 HP/s each",
        blurb: "One charge per purchase, stacks up to three. Each charge adds 0.4 health regeneration per second. Self-use consumes one charge to restore up to 100 mana instantly. Requires missing mana.",
    },
    ItemFace {
        id: ItemId::EAGLESONG.0,
        name: "Eaglesong",
        stats: "+25 AGI",
        blurb: "+25 agility.",
    },
    ItemFace {
        id: ItemId::CLAYMORE.0,
        name: "Claymore",
        stats: "+20 DMG",
        blurb: "+20 attack damage.",
    },
    ItemFace {
        id: ItemId::TALISMAN_OF_EVASION.0,
        name: "Talisman",
        stats: "15% evasion",
        blurb: "15% of attacks at the carrier miss. Of several evasions carried, only the best counts.",
    },
    ItemFace {
        id: ItemId::BUTTERFLY.0,
        name: "Butterfly",
        stats: "+30 AGI, 35% eva",
        blurb: "+30 agility, +30 attack damage, +20% base attack speed. 35% of attacks at the carrier miss. Of several evasions carried, only the best counts.",
    },
    ItemFace {
        id: ItemId::JAVELIN.0,
        name: "Javelin",
        stats: "25% pierce",
        blurb: "25% of attacks pierce: they cannot miss and deal 60 bonus magical damage. Not against buildings.",
    },
    ItemFace {
        id: ItemId::DEMON_EDGE.0,
        name: "Demon Edge",
        stats: "+40 DMG",
        blurb: "+40 attack damage.",
    },
    ItemFace {
        id: ItemId::BLITZ_KNUCKLES.0,
        name: "Blitz",
        stats: "+35 AS",
        blurb: "+35 attack speed.",
    },
    ItemFace {
        id: ItemId::RECIPE_MONKEY_KING_BAR.0,
        name: "Rcp MKB",
        stats: "recipe",
        blurb: "Builds a Monkey King Bar out of a Demon Edge, Blitz Knuckles and a Javelin.",
    },
    ItemFace {
        id: ItemId::MONKEY_KING_BAR.0,
        name: "MKB",
        stats: "+50 DMG, 80% pierce",
        blurb: "+50 attack damage, +50 attack speed, +50 attack range in melee hands. 80% of attacks pierce: they cannot miss and deal 70 bonus magical damage. Not against buildings. Of several pierces carried, only the best counts.",
    },
    ItemFace {
        id: ItemId::WIND_LACE.0,
        name: "Lace",
        stats: "+15 MS",
        blurb: "+15 movement speed, on top of boots. Of several laces carried, only one counts.",
    },
    ItemFace {
        id: ItemId::BLOOD_GRENADE.0,
        name: "Grenade",
        stats: "+50 HP",
        blurb: "+50 health. Thrown at a spot within 900, 0.2 s cast point, costs 75 health but never the last point: enemy heroes and creeps within 300 of where it lands take 50 magic damage, then 15 every second for 5 s, slowed 15%. Stacks two to a slot. The shop holds three at most, two at the start, one more every 3 min. 10 s wait.",
    },
];

/// Every timed effect, in id order. The ids are the wire's, so there is a
/// gap where a retired effect stood.
pub const EFFECTS: [EffectFace; 21] = [
    EffectFace {
        id: EffectId::MENDING.0,
        name: "Mending",
        blurb: "Regenerating health.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::CLARITY.0,
        name: "Clarity",
        blurb: "Regenerating mana.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::FOUNTAIN.0,
        name: "Fountain",
        blurb: "Regenerating health and mana for standing in the fountain.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::STUNNED.0,
        name: "Held",
        blurb: "Cannot move, attack or cast.",
        harmful: true,
    },
    EffectFace {
        id: EffectId::SLOWED.0,
        name: "Slowed",
        blurb: "Movement speed reduced.",
        harmful: true,
    },
    EffectFace {
        id: EffectId::BURNING.0,
        name: "Burning",
        blurb: "Losing health over time.",
        harmful: true,
    },
    EffectFace {
        id: EffectId::HASTENED.0,
        name: "Hastened",
        blurb: "Movement speed increased.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::SHIELDED.0,
        name: "Shielded",
        blurb: "Nothing gets through.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::PHASED.0,
        name: "Phased",
        blurb: "Walking through the bodies in the way.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::FLESH_HEAP.0,
        name: "Heap",
        blurb: "Strength kept from every enemy hero that died nearby.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::SOULS.0,
        name: "Souls",
        blurb: "Attack damage from every unit brought down.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::ARMOR_BROKEN.0,
        name: "Presence",
        blurb: "Armor worn down by a dark presence nearby.",
        harmful: true,
    },
    EffectFace {
        id: EffectId::GUARDED.0,
        name: "Guarded",
        blurb: "Armor and health mended, for standing by a tower of your own.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::INSPIRED.0,
        name: "Inspired",
        blurb: "Health mended and magic shrugged off, for marching beside the one carrying the flag.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::SHADOWRAZE.0,
        name: "Razed",
        blurb: "Shadowrazes from the same caster deal 35/50/65/80 more magic damage per stack. Each damaging hit refreshes all stacks for 6 s; up to 255 stacks.",
        harmful: true,
    },
    EffectFace {
        id: EffectId::ROT.0,
        name: "Rot",
        blurb: "The rot is on. It is never listed on a unit: the toggle shows on the ability.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::FEARED.0,
        name: "Feared",
        blurb: "Running from whoever put the fear on. Cannot attack or cast.",
        harmful: true,
    },
    EffectFace {
        id: EffectId::MAGIC_BROKEN.0,
        name: "Torn",
        blurb: "Magic resistance torn by a requiem.",
        harmful: true,
    },
    EffectFace {
        id: EffectId::FEASTING.0,
        name: "Feast",
        blurb: "Attacking and moving faster while taking souls from enemies nearby; counts the souls taken.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::FEAST_LINGER.0,
        name: "Fed",
        blurb: "Souls a feast took from enemies since fallen, kept past the cap until it runs out.",
        harmful: false,
    },
    EffectFace {
        id: EffectId::BLOOD_GRENADE.0,
        name: "Grenade",
        blurb: "Slowed 15% and losing 15 health to magic every second, from a Blood Grenade.",
        harmful: true,
    },
];

/// Every hero that can be picked, in id order.
pub const HEROES: [HeroFace; 2] = [
    HeroFace {
        id: HeroId::PUDGE.0,
        name: "Pudge",
    },
    HeroFace {
        id: HeroId::SHADOW_FIEND.0,
        name: "Shadow Fiend",
    },
];

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
    EFFECTS.iter().find(|face| face.id == id)
}

/// The hero of that id, or nothing for one the catalog does not hold.
pub fn hero(id: u16) -> Option<&'static HeroFace> {
    HEROES.get(usize::from(id))
}

/// One talent of a hero's tree, as the talent panel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TalentFace {
    /// What it gives, short, as Dota's tree writes it.
    pub name: &'static str,
    /// What it does, for the hover popup.
    pub blurb: &'static str,
}

/// Hero level each tier of talents opens at, one talent of the two; the
/// other of a tier opens at [`TALENT_SECOND_LEVELS`].
pub const TALENT_LEVELS: [u8; 4] = [10, 15, 20, 25];
/// Hero level the other talent of each tier opens at.
pub const TALENT_SECOND_LEVELS: [u8; 4] = [27, 28, 29, 30];

/// Shadow Fiend's tree at Dota 7.41f, by talent slot: the left of each tier
/// first, the level-10 tier first.
const SHADOW_FIEND_TALENTS: [TalentFace; 8] = [
    TalentFace {
        name: "+30 Feast of Souls Attack Speed",
        blurb: "A feast gives 30 more attack speed.",
    },
    TalentFace {
        name: "+30 Shadowraze Stack Damage",
        blurb: "Each stack of a raze's mark adds 30 more magic damage.",
    },
    TalentFace {
        name: "+2 Feast of Souls Souls Per Hero",
        blurb: "A feast takes 2 more souls from every enemy hero.",
    },
    TalentFace {
        name: "+1.5 Presence Armor Reduction",
        blurb: "The presence takes 1.5 more armor from enemies near you.",
    },
    TalentFace {
        name: "+0.2s Requiem Fear per Line (max +0.45s)",
        blurb: "Every requiem line fears, slows and tears 0.2 s longer, up to 0.45 s more in all.",
    },
    TalentFace {
        name: "+5 Necromastery Max Souls",
        blurb: "Hold 5 more souls from kills.",
    },
    TalentFace {
        name: "Shadowraze Applies Attack Damage",
        blurb: "A raze also deals your attack damage, as physical damage.",
    },
    TalentFace {
        name: "Feast of Souls Grants +30% Cast Speed",
        blurb: "While a feast runs, cast points last 70% of their length.",
    },
];

/// A hero's talent tree by talent slot, if it has one.
pub fn talents(hero: u16) -> Option<&'static [TalentFace; 8]> {
    (hero == HeroId::SHADOW_FIEND.0).then_some(&SHADOW_FIEND_TALENTS)
}

/// A unit's attack damage the way Dota's panel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRange {
    /// The least and the most a swing deals without bonuses: the base
    /// range plus the primary attribute.
    pub min: i32,
    pub max: i32,
    /// What items and effects add on top of that, shown apart.
    pub bonus: i32,
}

/// Splits the range a swing draws from, as the view tells it, into the base
/// range and the bonus on top.
pub fn damage_range(unit: &bota_proto::UnitView) -> DamageRange {
    // White: the hero's base range and its primary attribute's base (as it
    // starts and grows by level); green: everything else.
    let base_least = unit
        .hero
        .and_then(hero_def)
        .map(|hero| base_damage_least(hero.unit, unit.level));
    let bonus = base_least.map_or(0, |base| (unit.attack_damage_min - base).max(0));
    DamageRange {
        min: unit.attack_damage_min - bonus,
        max: unit.attack_damage_max - bonus,
        bonus,
    }
}

/// A hero's strength, agility and intelligence of its level alone, before
/// items, souls, buffs and talents: whole points, as Dota counts them.
pub fn base_attributes(unit: &bota_proto::UnitView) -> Option<[i32; 3]> {
    let has = attributes_at_level(hero_def(unit.hero?)?.unit, unit.level);
    Some([has.strength, has.agility, has.intelligence].map(Fixed::to_int))
}
