//! Things that happened during one tick.
//!
//! A team is sent only the events it may know of, as the
//! [contract](crate::contract) says: those of the units it sees, and of who
//! dealt them only a dealer it sees.

use crate::{
    AbilityId, EntityId, Inflictor, ItemId, LotusTier, ObjectiveKind, RuneKind, SlotId, Team,
    UnitKind, Vec2,
};
use serde::{Deserialize, Serialize};

/// How a chunk of damage is reduced before it is applied.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DamageKind {
    /// Reduced by armor.
    Physical,
    /// Reduced by magic resistance.
    Magical,
    /// Not reduced by anything.
    Pure,
}

/// Why a death credited one component of gold. Variants are append-only.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GoldAwardKind {
    KillBounty,
    FlagbearerBonus,
    AssistGold,
}

/// One actual credit associated with a death, before presentation or aggregation.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GoldAward {
    pub recipient: SlotId,
    /// The recipient's side, for independent income privacy when a death rewards both sides.
    pub team: Team,
    pub kind: GoldAwardKind,
    pub amount: i32,
}

/// A single thing that happened on one tick.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// A unit took damage.
    Damaged {
        /// Who dealt it, if anyone; may name a unit that has since died.
        /// `None` to a side that does not see the dealer.
        source: Option<EntityId>,
        /// Who took it.
        target: EntityId,
        /// Health actually lost, after armor and resistance.
        amount: i32,
        /// Which reduction applied.
        kind: DamageKind,
        /// The ability or item that dealt it. Absent for an attack's own
        /// blow.
        inflictor: Option<Inflictor>,
        /// Whether it removed health rather than dealt damage (Dota's HP
        /// Removal): reduced as damage of its `kind` is, it set off nothing
        /// that answers to damage.
        removal: bool,
    },
    /// An attack did not land: the target evaded it, or it was thrown
    /// uphill and missed.
    Missed {
        /// Who swung; may name a unit that has since died. `None` to a side
        /// that does not see the swinger.
        source: Option<EntityId>,
        /// Who it was swung at.
        target: EntityId,
    },
    /// A unit was mended by somebody's hand: an item drunk or a charge
    /// spent. Passive regeneration and the fountain are not told of.
    Healed {
        /// Who mended it. `None` to a side that does not see the mender.
        source: Option<EntityId>,
        /// Who was mended.
        target: EntityId,
        /// Health the mending is good for: no more than it holds, no more
        /// than was missing when it began. A mend paid out over time may
        /// still be cut short by a blow.
        amount: i32,
        /// Mana it restores alongside, on the same counting.
        mana: i32,
        /// The item or ability that mends it.
        inflictor: Option<Inflictor>,
    },
    /// A unit died.
    Died {
        /// The unit that died.
        unit: EntityId,
        /// The unit that dealt the killing blow. Hidden from a side that is
        /// not told the physical attacker; it is not replaced by kill credit.
        killer: Option<EntityId>,
        /// Whether the killer was on the same team, making this a deny.
        denied: bool,
        /// Retired: always zero. `bounty` tells it.
        gold: i32,
        /// Gold paid to the credited hero's seat: its bounty and assist-gold
        /// share, a flagbearer's bonus and a tower's team bounty on top.
        /// On a deny only a tower's half team bounty. A shared nonhero finish
        /// reports the total paid to its recipients. Absent to the side that did not earn a
        /// non-hero kill: another's income is its own. A hero kill's gold
        /// is public, as Dota's kill feed tells it.
        bounty: Option<i32>,
        /// The seats of the enemy heroes besides the killer that dealt damage
        /// to, or put a debuff on, the fallen hero in the twenty seconds
        /// before it fell (Dota's assists), one bit for each: bit `n` is the
        /// seat numbered `n`. Zero for any other unit, and withheld with the
        /// killer from a side that is not told it.
        assists: u16,
        /// Disclosed credit components, when a breakdown is provided. Each side learns only
        /// its own recipients' awards; a full spectator learns all of them. An income recipient
        /// can receive this fact without seeing the fallen unit or its killer.
        #[serde(
            deserialize_with = "crate::bounded::option_boxed_seq::<_, _, { crate::limits::MAX_DEATH_GOLD_AWARDS }>"
        )]
        awards: Option<Box<Vec<GoldAward>>>,
        /// The publicly announced hero awarded credit for a real hero death.
        /// May differ from the physical killer. None for other unit deaths or
        /// when no individual hero receives the kill. Last on the wire.
        // Keep the optional handle out of the common event footprint.
        credited_hero: Option<Box<EntityId>>,
    },
    /// A unit's ability took effect.
    ///
    /// A cast is told at the moment of effect, not when the order was issued.
    AbilityCast {
        /// Who cast it.
        caster: EntityId,
        ability: AbilityId,
    },
    /// A hero gained a level. Not sent for a level gained while it is dead,
    /// and told to the other side only when it sees the hero.
    LevelUp {
        /// Which hero.
        unit: EntityId,
        /// The level just reached.
        level: u8,
    },
    /// A seat bought an item, or was handed one by a cheat. Told to its own
    /// team alone.
    ItemBought {
        /// Which seat bought it.
        slot: SlotId,
        /// What was bought.
        item: ItemId,
    },
    /// A building was destroyed.
    StructureDestroyed {
        /// Which building.
        unit: EntityId,
        /// Which team lost it.
        team: Team,
        /// What kind of building it was.
        kind: UnitKind,
        /// The lane it stands on, from zero, if it belongs to one.
        lane: Option<u8>,
        /// Its tier, from one, if it has one (a tower's).
        tier: Option<u8>,
        /// Whether a barracks feeds the ranged creeps; absent for any other building.
        ranged: Option<bool>,
    },
    /// A teleport channel ended, told on the tick it did and to both sides, as
    /// its [`TeleportView`](crate::TeleportView) is.
    Teleported {
        /// The seat whose hero channelled it.
        slot: SlotId,
        /// Whether the hero arrived. False when the channel was broken off or
        /// found no ground to land on.
        landed: bool,
    },
    /// Something thrown at a spot came down there and went off, as a Blood
    /// Grenade bursts. Told to the sides that see the spot, or to the
    /// thrower's when none does.
    Landed {
        /// What was thrown.
        inflictor: Inflictor,
        /// Who threw it, told as a blow's dealer is; may name a unit that has
        /// since died.
        caster: Option<EntityId>,
        /// Where it went off.
        at: Vec2,
        /// The missile that ended here; absent in records without flight identity.
        missile: Option<EntityId>,
    },
    /// A building changed hands: an Outpost captured. Told to both sides.
    Captured {
        /// Which building.
        unit: EntityId,
        /// The side that holds it now.
        team: Team,
        /// The hero whose channel took it; `None` to the other side.
        by: Option<EntityId>,
    },
    /// A Shrine of Wisdom was gathered. Told to both sides.
    ShrineActivated {
        /// Which shrine.
        unit: EntityId,
        /// The side that gathered it.
        team: Team,
        /// Experience each of the side's heroes it rewarded was given.
        xp: i32,
        /// The gatherer it rewarded; `None` to the other side.
        by: Option<EntityId>,
    },
    /// A tree came down: one of the map's own eaten by a Tango or cut down,
    /// or one put up felled or run out of its time. Told to the sides that
    /// see its spot, as the forest a side is shown changes only where it
    /// looks ([`WorldView::felled_trees`](crate::WorldView::felled_trees)).
    TreeFelled {
        /// Where it stood: a spot of
        /// [`MatchInfo::trees`](crate::MatchInfo::trees) or of
        /// [`WorldView::planted_trees`](crate::WorldView::planted_trees).
        at: Vec2,
        /// Who brought it down, told as a blow's dealer is; `None` for one
        /// that ran out of its time.
        by: Option<EntityId>,
    },
    /// One of the map's own trees grew back. Told to the sides that see its
    /// spot.
    TreeGrew {
        /// Where it stands: a spot of
        /// [`MatchInfo::trees`](crate::MatchInfo::trees).
        at: Vec2,
    },
    /// A unit used an item and it took effect, as an
    /// [`AbilityCast`](EventKind::AbilityCast) tells an ability's: told at the
    /// moment of effect, to the sides that see the user where it stood or where
    /// it stands after, and to the user's own. A Blink Dagger is told with
    /// the user's two places in the views before and after.
    ItemUsed {
        /// Who used it, told as a blow's dealer is.
        caster: Option<EntityId>,
        item: ItemId,
        /// The spot the use was aimed at: the point itself, or where the aimed
        /// unit stood. Absent for a use aimed at nothing or a point withheld from this side.
        at: Option<Vec2>,
        /// The aimed unit, when any. Another side that cannot see it receives neither its id nor
        /// its position; full streams retain it so replay POVs can apply unit sight.
        target: Option<EntityId>,
    },
    /// A rune appeared at a spot of the map. Told only to sides whose
    /// completed-tick snapshot shows it.
    RuneSpawned {
        /// Which rune.
        kind: RuneKind,
        /// Where it lies.
        at: Vec2,
    },
    /// A hero took a rune and it took effect, at once or out of a Bottle.
    /// Told to the sides that see the hero, and to its own.
    RuneActivated {
        /// The hero.
        unit: EntityId,
        /// Which rune.
        kind: RuneKind,
        /// Whether it came out of a Bottle.
        bottled: bool,
        /// The gold a Bounty Rune paid every hero of the hero's side. Absent
        /// for any other rune.
        gold: Option<i32>,
    },
    /// A hero took a rune into a Bottle. Its contents are told only to its own side.
    RuneBottled {
        /// The hero.
        unit: EntityId,
        /// Which rune.
        kind: RuneKind,
    },
    /// A hero destroyed a rune with an attack order: neither a last hit nor
    /// a deny. Told to the attacker's side, and to other sides only when
    /// they see both the spot and the attacker.
    RuneDenied {
        /// The hero.
        unit: EntityId,
        /// Which rune.
        kind: RuneKind,
        /// Where it lay.
        at: Vec2,
        /// The hero's side.
        team: Team,
    },
    /// A Lotus Pool grew a lotus. Told to both sides.
    LotusGrown {
        /// The pool.
        pool: EntityId,
        /// Which lotus.
        tier: LotusTier,
    },
    /// A hero plucked a lotus from a pool and has it in its bag. Told to
    /// both sides.
    LotusPlucked {
        /// The pool.
        pool: EntityId,
        /// The hero; `None` to a side that does not see it.
        unit: Option<EntityId>,
        /// Which lotus.
        tier: LotusTier,
    },
    /// A map boss fell; both sides know which side landed the last hit.
    BossFell {
        objective: ObjectiveKind,
        team: Option<Team>,
    },

    /// Public ground loot was destroyed by a seat, or by a unit of a side.
    ItemDestroyed {
        item: ItemId,
        by: Option<SlotId>,
        team: Team,
    },
    /// Public ground loot was picked up by a seat.
    ItemTaken {
        item: ItemId,
        by: SlotId,
        team: Team,
    },
    /// A placed item announced to both sides, including its map location.
    ItemPlaced { item: ItemId, team: Team, at: Vec2 },
    /// A boss announces its roar globally, without revealing coordinates or entity handles.
    BossRoared {
        objective: ObjectiveKind,
        team: Team,
    },
    /// A unit reached its attack point: a melee strike or a ranged launch.
    /// Told to sides that see the attacker at this Events envelope's tick.
    AttackLaunched {
        attacker: EntityId,
        /// Absent when that side does not see the target.
        target: Option<EntityId>,
    },
    /// A unit's warp through a Twin Gate ran its whole channel and carried
    /// it to the other gate. Told to a side that sees either gate or the
    /// unit where it arrives.
    GateWarped {
        /// The gate the channel was made at.
        from: EntityId,
        /// The gate it came out of.
        to: EntityId,
        /// The unit; `None` to a side that does not see it arrive.
        unit: Option<EntityId>,
    },
}

/// The seats a [`EventKind::Died`] names as assisting, in seat order.
pub fn assisting_seats(bits: u16) -> impl Iterator<Item = SlotId> {
    (0..u16::BITS as u8)
        .filter(move |seat| bits & (1 << seat) != 0)
        .map(SlotId)
}
