//! Identifiers and small enumerations used across every message.

use serde::{Deserialize, Serialize};

/// A handle to an entity in the world: a unit, a projectile or an item on the
/// ground.
///
/// Generational: a handle to a dead entity never becomes valid again.
/// Meaningful only within the match that issued it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId {
    /// Slot index into the entity arena.
    pub idx: u32,
    /// Generation counter, bumped every time the slot is reused.
    pub generation: u32,
}

/// A seat in the match, `0..` the number of seats.
///
/// Fixed for the whole match, so it is usable as an array index.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotId(pub u8);

/// A connection, as seen by the network layer.
///
/// Every connection gets a fresh one. A spectator has a `PlayerId` and no
/// [`SlotId`].
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u32);

/// The secret that entitles its holder to one reserved seat, or to the
/// referee's connection.
///
/// Compared in constant time and never printed.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct SeatKey(pub [u8; 32]);

impl PartialEq for SeatKey {
    fn eq(&self, other: &Self) -> bool {
        let mut diff = 0u8;
        for (a, b) in self.0.iter().zip(&other.0) {
            diff |= a ^ b;
        }
        std::hint::black_box(diff) == 0
    }
}

impl Eq for SeatKey {}

impl core::fmt::Debug for SeatKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SeatKey(..)")
    }
}

/// A side of the map, or nobody's side.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Team {
    /// The side spawning in the lower left corner.
    Radiant,
    /// The side spawning in the upper right corner.
    Dire,
    /// The jungle's own: hostile to both sides. No seat plays for it.
    Neutral,
}

impl Team {
    /// The side across the map; the jungle's own is nobody's enemy side.
    pub const fn enemy(self) -> Team {
        match self {
            Team::Radiant => Team::Dire,
            Team::Dire => Team::Radiant,
            Team::Neutral => Team::Neutral,
        }
    }
}

/// Selects one of the playable heroes.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HeroId(pub u16);

impl HeroId {
    /// Pudge, the butcher.
    pub const PUDGE: HeroId = HeroId(0);
    /// Shadow Fiend, the nevermore.
    pub const SHADOW_FIEND: HeroId = HeroId(1);
}

/// Selects one specific ability, independent of which hero owns it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AbilityId(pub u16);

impl AbilityId {
    /// Pudge's hook.
    pub const MEAT_HOOK: AbilityId = AbilityId(0);
    /// Pudge's rot: a toggle that burns everything near, its owner included.
    pub const ROT: AbilityId = AbilityId(1);
    /// Pudge's flesh heap: what he keeps of every enemy hero that falls near
    /// him.
    pub const FLESH_HEAP: AbilityId = AbilityId(2);
    /// Pudge's dismember: holds one unit and eats it.
    pub const DISMEMBER: AbilityId = AbilityId(3);
    /// A courier's burst of speed.
    pub const BURST: AbilityId = AbilityId(4);
    /// A courier putting back what it carries.
    pub const RETURN_ITEMS: AbilityId = AbilityId(5);
    /// A courier taking what waits in the stash.
    pub const TAKE_STASH: AbilityId = AbilityId(6);
    /// A courier handing over what it carries.
    pub const DELIVER: AbilityId = AbilityId(7);
    /// A courier's shield, which nothing gets through.
    pub const SHIELD: AbilityId = AbilityId(8);
    /// Shadow Fiend's nearest raze.
    pub const RAZE_NEAR: AbilityId = AbilityId(9);
    /// Shadow Fiend's middle raze.
    pub const RAZE_MID: AbilityId = AbilityId(10);
    /// Shadow Fiend's farthest raze.
    pub const RAZE_FAR: AbilityId = AbilityId(11);
    /// Shadow Fiend's requiem: a line for each soul gathered, which it does not
    /// spend.
    pub const REQUIEM: AbilityId = AbilityId(12);
    /// Shadow Fiend's necromastery: a soul kept of everything he brings down.
    /// Innate: it sits in no slot and is never learned.
    pub const NECROMASTERY: AbilityId = AbilityId(13);
    /// Shadow Fiend's presence: enemies standing near him wear less armor.
    pub const PRESENCE: AbilityId = AbilityId(14);
    /// Shadow Fiend's feast of souls: attacks and steps come faster for a while,
    /// and souls are taken from the enemies near him.
    pub const FEAST_OF_SOULS: AbilityId = AbilityId(15);
    /// A Blood Grenade's throw: what names one in flight. Sits in no slot.
    pub const THROW_GRENADE: AbilityId = AbilityId(16);
}

/// Selects one purchasable item.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(pub u16);

impl ItemId {
    /// Boots of Speed.
    pub const BOOTS: ItemId = ItemId(0);
    /// Clarity.
    pub const CLARITY: ItemId = ItemId(1);
    /// Healing Salve.
    pub const HEALING_SALVE: ItemId = ItemId(2);
    /// Iron Branch.
    pub const IRON_BRANCH: ItemId = ItemId(3);
    /// Observer Ward.
    pub const OBSERVER_WARD: ItemId = ItemId(4);
    /// Quelling Blade.
    pub const QUELLING_BLADE: ItemId = ItemId(5);
    /// Sentry Ward.
    pub const SENTRY_WARD: ItemId = ItemId(6);
    /// Tango.
    pub const TANGO: ItemId = ItemId(7);
    /// Town Portal Scroll.
    pub const TOWN_PORTAL_SCROLL: ItemId = ItemId(8);
    /// Circlet.
    pub const CIRCLET: ItemId = ItemId(9);
    /// Gauntlets of Strength.
    pub const GAUNTLETS: ItemId = ItemId(10);
    /// Slippers of Agility.
    pub const SLIPPERS: ItemId = ItemId(11);
    /// Mantle of Intelligence.
    pub const MANTLE: ItemId = ItemId(12);
    /// Belt of Strength.
    pub const BELT: ItemId = ItemId(13);
    /// Band of Elvenskin.
    pub const BAND: ItemId = ItemId(14);
    /// Robe of the Magi.
    pub const ROBE: ItemId = ItemId(15);
    /// Ogre Axe.
    pub const OGRE_AXE: ItemId = ItemId(16);
    /// Blade of Alacrity.
    pub const BLADE_OF_ALACRITY: ItemId = ItemId(17);
    /// Staff of Wizardry.
    pub const STAFF_OF_WIZARDRY: ItemId = ItemId(18);
    /// Gloves of Haste.
    pub const GLOVES: ItemId = ItemId(19);
    /// Blades of Attack.
    pub const BLADES_OF_ATTACK: ItemId = ItemId(20);
    /// Broadsword.
    pub const BROADSWORD: ItemId = ItemId(21);
    /// Quarterstaff.
    pub const QUARTERSTAFF: ItemId = ItemId(22);
    /// Ring of Protection.
    pub const RING_OF_PROTECTION: ItemId = ItemId(23);
    /// Chainmail.
    pub const CHAINMAIL: ItemId = ItemId(24);
    /// Ring of Regen.
    pub const RING_OF_REGEN: ItemId = ItemId(25);
    /// Sage's Mask.
    pub const SAGES_MASK: ItemId = ItemId(26);
    /// Vitality Booster.
    pub const VITALITY_BOOSTER: ItemId = ItemId(27);
    /// Energy Booster.
    pub const ENERGY_BOOSTER: ItemId = ItemId(28);
    /// Power Treads.
    pub const POWER_TREADS: ItemId = ItemId(29);
    /// Phase Boots.
    pub const PHASE_BOOTS: ItemId = ItemId(30);
    /// Blink Dagger.
    pub const BLINK_DAGGER: ItemId = ItemId(31);
    /// Bracer.
    pub const BRACER: ItemId = ItemId(32);
    /// Wraith Band.
    pub const WRAITH_BAND: ItemId = ItemId(33);
    /// Null Talisman.
    pub const NULL_TALISMAN: ItemId = ItemId(34);
    /// Magic Stick.
    pub const MAGIC_STICK: ItemId = ItemId(35);
    /// Magic Wand.
    pub const MAGIC_WAND: ItemId = ItemId(36);
    /// The recipe Phase Boots are built with.
    pub const RECIPE_PHASE_BOOTS: ItemId = ItemId(37);
    /// The recipe a Bracer is built with.
    pub const RECIPE_BRACER: ItemId = ItemId(38);
    /// The recipe a Wraith Band is built with.
    pub const RECIPE_WRAITH_BAND: ItemId = ItemId(39);
    /// The recipe a Null Talisman is built with.
    pub const RECIPE_NULL_TALISMAN: ItemId = ItemId(40);
    /// The recipe a Magic Wand is built with.
    pub const RECIPE_MAGIC_WAND: ItemId = ItemId(41);
    /// Enchanted Mango.
    pub const MANGO: ItemId = ItemId(42);
    /// Eaglesong.
    pub const EAGLESONG: ItemId = ItemId(43);
    /// Claymore.
    pub const CLAYMORE: ItemId = ItemId(44);
    /// Talisman of Evasion.
    pub const TALISMAN_OF_EVASION: ItemId = ItemId(45);
    /// Butterfly.
    pub const BUTTERFLY: ItemId = ItemId(46);
    /// Javelin.
    pub const JAVELIN: ItemId = ItemId(47);
    /// Demon Edge.
    pub const DEMON_EDGE: ItemId = ItemId(48);
    /// Blitz Knuckles.
    pub const BLITZ_KNUCKLES: ItemId = ItemId(49);
    /// The recipe a Monkey King Bar is built with.
    pub const RECIPE_MONKEY_KING_BAR: ItemId = ItemId(50);
    /// Monkey King Bar.
    pub const MONKEY_KING_BAR: ItemId = ItemId(51);
    /// Wind Lace.
    pub const WIND_LACE: ItemId = ItemId(52);
    /// Blood Grenade.
    pub const BLOOD_GRENADE: ItemId = ItemId(53);
}

/// How something in a slot is aimed when it is used.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Aim {
    /// At nothing: it works on whoever used it.
    Own,
    /// At a spot on the ground.
    Point,
    /// At a unit.
    Unit,
    /// At the tree standing on the spot it is pointed at.
    Tree,
    /// At an allied building: a spot on the ground carries to the building
    /// nearest it, a building is aimed at itself, and nothing at all aims at
    /// the fountain (Dota's double tap).
    Building,
}

/// Names one kind of timed effect a unit can be under.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectId(pub u16);

impl EffectId {
    /// Health mended over time by a Healing Salve.
    pub const MENDING: EffectId = EffectId(1);
    /// Mana mended over time by a Clarity.
    pub const CLARITY: EffectId = EffectId(2);
    /// Standing in the fountain's light.
    pub const FOUNTAIN: EffectId = EffectId(3);
    /// Unable to act.
    pub const STUNNED: EffectId = EffectId(4);
    /// Slowed.
    pub const SLOWED: EffectId = EffectId(5);
    /// Burning, losing health.
    pub const BURNING: EffectId = EffectId(6);
    /// Hastened.
    pub const HASTENED: EffectId = EffectId(7);
    /// Nothing gets through.
    pub const SHIELDED: EffectId = EffectId(8);
    /// Phased: moves through units.
    pub const PHASED: EffectId = EffectId(9);
    /// Flesh Heap stacks.
    pub const FLESH_HEAP: EffectId = EffectId(10);
    /// Souls gathered.
    pub const SOULS: EffectId = EffectId(11);
    /// Armor torn.
    pub const ARMOR_BROKEN: EffectId = EffectId(12);
    /// Guarded.
    pub const GUARDED: EffectId = EffectId(13);
    /// Inspired.
    pub const INSPIRED: EffectId = EffectId(14);
    /// Shadowraze stacks.
    pub const SHADOWRAZE: EffectId = EffectId(15);
    /// Rot burning.
    pub const ROT: EffectId = EffectId(16);
    /// Afraid, running from its fear.
    pub const FEARED: EffectId = EffectId(17);
    /// Magic resistance torn by a Requiem of Souls line: `ticks_left` only.
    pub const MAGIC_BROKEN: EffectId = EffectId(18);
    /// Feast of Souls under way on its caster: `ticks_left` until it ends,
    /// `stacks` the souls it has collected so far.
    pub const FEASTING: EffectId = EffectId(19);
    /// What a Feast of Souls left behind once it ended: `ticks_left` until
    /// the souls held past the cap are lost, `stacks` the souls it took from
    /// enemies that have died since.
    pub const FEAST_LINGER: EffectId = EffectId(20);
    /// A Blood Grenade's debuff: `ticks_left` until it lifts. The unit is
    /// slowed and loses health on every whole second since the first one
    /// landed.
    pub const BLOOD_GRENADE: EffectId = EffectId(21);
}

/// An index into a unit's ability slots, as listed in
/// [`UnitView::abilities`](crate::UnitView::abilities).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AbilitySlot(pub u8);

/// One of a hero's eight talents, as Dota's talent tree draws them.
///
/// Tier `n / 2` opens at hero level 10, 15, 20 and 25; an even slot is the
/// left of its tier, an odd one the right. What each slot of a hero is comes
/// with the hero, not over the wire. A hero without talents has no slots.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TalentSlot(pub u8);

impl TalentSlot {
    /// Slots in a whole tree.
    pub const COUNT: u8 = 8;

    /// Which tier it belongs to, from zero for the level-10 tier.
    pub const fn tier(self) -> u8 {
        self.0 / 2
    }

    /// The other talent of its tier.
    pub const fn other(self) -> TalentSlot {
        TalentSlot(self.0 ^ 1)
    }
}

/// One of a hero's sixteen item slots.
///
/// Slots 0-5 are the inventory, where items work; 6-8 the backpack, where
/// they are carried inert; 9-14 the stash waiting at the home shop; 15 the
/// town portal slot, which holds Town Portal Scrolls and nothing else.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemSlot(pub u8);

impl ItemSlot {
    /// Dota's dedicated town portal slot beside the inventory: scrolls work
    /// from it and never leave it but to be read.
    pub const TOWN_PORTAL: ItemSlot = ItemSlot(15);
}

/// Selects the terrain and building layout a match is played on.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapId(pub u16);

/// What kind of thing a unit in a view is.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnitKind {
    /// A hero controlled by a player or a bot.
    Hero,
    /// A melee lane creep.
    CreepMelee,
    /// A melee lane creep carrying the flag: resistant to magic, and mends
    /// the health of its own side nearby.
    CreepFlagbearer,
    /// A ranged lane creep.
    CreepRanged,
    /// A siege creep.
    CreepSiege,
    /// A neutral camp creep.
    CreepNeutral,
    /// A lane tower.
    Tower,
    /// The structure that ends the match when destroyed.
    Ancient,
    /// A barracks; while it stands, the enemy creeps of its kind in its lane
    /// stay plain.
    Barracks,
    /// The fountain, which mends its own side and attacks enemies in reach.
    Fountain,
    /// An observer ward placed by a hero.
    Ward,
    /// A courier, one to a seat, that carries what its owner bought.
    Courier,
    /// A filler: a statue standing in a base that does nothing but pay gold
    /// when it falls.
    Filler,
}
