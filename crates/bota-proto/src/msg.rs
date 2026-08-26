//! The messages themselves, and the lobby types they carry.

use crate::{
    EntityId, EventKind, HeroId, ItemId, MapId, Order, PlayerId, SlotId, Team, Vec2, WorldView,
};
use serde::{Deserialize, Serialize};

/// Why a participant connected.
///
/// Decides what the server sends back: a player and a bot get their own team's
/// fog of war, a spectator gets the whole map.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// A human at a client, taking a seat.
    Player,
    /// A program taking a seat. The simulation treats it exactly as a player.
    Bot,
    /// An observer with no seat.
    Spectator,
}

/// How the server decides when to advance a tick.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TickMode {
    /// Advance on a wall clock at the configured rate. An order that misses its
    /// tick applies on the next one.
    Realtime,
    /// Advance only once every participant has acknowledged the tick, with no
    /// bound on how long one may take.
    Lockstep,
}

/// One seat's hero choice.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pick {
    /// Which seat.
    pub slot: SlotId,
    /// Which side it plays for.
    pub team: Team,
    /// Which hero it picked.
    pub hero: HeroId,
}

/// One row of the lobby, before the match starts.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LobbySlot {
    /// Which seat this row is.
    pub slot: SlotId,
    /// Which side it plays for.
    pub team: Team,
    /// Display name of whoever holds it. Empty while the seat is open.
    pub name: String,
    /// What holds it. Absent while the seat is open.
    pub role: Option<Role>,
    /// Which hero has been picked. Absent until one is.
    pub hero: Option<HeroId>,
    /// Whether the participant has declared itself ready.
    pub ready: bool,
}

/// What the shop asks for one item.
///
/// The price of a thing nobody holds yet, so it belongs to no unit and is
/// stated once for the whole match. Everything an item is worth to whoever
/// carries it rides in [`ItemView`] instead, where it can differ between two
/// units holding the same item.
///
/// [`ItemView`]: crate::ItemView
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ShopEntry {
    /// Which item.
    pub id: ItemId,
    /// What it costs whole, in gold.
    pub cost: i32,
    /// What it is built from. Empty for one bought whole.
    pub components: Vec<ItemId>,
}

/// The public description of a match.
///
/// Sent when the match begins and to anyone joining later.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MatchInfo {
    /// Identifies this match in logs and replay files.
    pub match_id: u64,
    /// Which map is being played.
    pub map: MapId,
    /// Simulation ticks per second.
    pub tick_rate: u16,
    /// Ticks before the game clock reaches zero. The clock counts up from
    /// minus this; creep waves start at zero.
    pub pregame_ticks: u32,
    /// Every tree on the map. Static for the whole match; sent once here so
    /// the client can draw and never has to know the layout rules.
    pub trees: Vec<Vec2>,
    /// Cells per terrain axis.
    pub terrain_cells: u32,
    /// Run-length encoded terrain cells, row-major from the south-west
    /// corner, one byte each: bit 7 walkable ground, bit 6 river water, the
    /// low bits the elevation tier.
    pub terrain_rle: Vec<(u16, u8)>,
    /// Cells that block sight lines regardless of elevation: trees and the
    /// map's fog blocker walls. The client shades its own fog with these.
    pub opaque_cells: Vec<(u16, u16)>,
    /// How the server advances ticks.
    pub mode: TickMode,
    /// Every seat and its hero, sorted by [`SlotId`].
    pub picks: Vec<Pick>,
    /// Everything the shop sells, in item id order.
    pub shop: Vec<ShopEntry>,
}

/// Why the server refused an order.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RejectReason {
    /// The order arrived for a seat this connection does not hold.
    NotYourSlot,
    /// The hero is dead.
    HeroDead,
    /// The target does not exist, or is not currently visible to this team.
    /// One answer covers both cases.
    UnknownTarget,
    /// The ability or item does not accept this kind of target.
    WrongTargetKind,
    /// The ability or item is still on cooldown.
    OnCooldown,
    /// The target is beyond the ability's cast range.
    OutOfRange,
    /// Not enough mana.
    NotEnoughMana,
    /// Not enough gold.
    NotEnoughGold,
    /// The referenced ability or inventory slot is empty.
    EmptySlot,
    /// The ability works on its own and is never cast.
    NotCastable,
    /// The ability has no points in it yet.
    NotLearned,
    /// The item has no charges left to spend.
    NoCharges,
    /// The item has come out of the backpack and is not working yet.
    NotReady,
    /// The order named a unit this seat does not drive.
    NotYourUnit,
    /// No item with this id is sold.
    UnknownItem,
    /// No skill point is available, or the ability is already at its cap.
    CannotLevelUp,
    /// Buying and selling require standing in the fountain area.
    NotAtShop,
    /// The inventory is full.
    InventoryFull,
    /// A status such as silence or stun forbids this action right now.
    Disabled,
    /// The match has not started or has already finished.
    NotPlaying,
    /// Only the seat that bought an item may sell it or mark it for sale.
    NotYourItem,
    /// The spot aimed at is ground nothing may stand on.
    ClosedGround,
    /// The slot named is not one the unit carries on itself.
    NotInBag,
}

/// Final numbers for one seat.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotStats {
    /// Which seat.
    pub slot: SlotId,
    /// Kills scored.
    pub kills: u16,
    /// Times died.
    pub deaths: u16,
    /// Kills assisted.
    pub assists: u16,
    /// Enemy creeps last hit.
    pub last_hits: u16,
    /// Friendly creeps denied.
    pub denies: u16,
    /// Gold earned over the whole match, spent or not.
    pub net_worth: i32,
    /// Damage dealt to enemy heroes.
    pub hero_damage: i32,
    /// Damage dealt to enemy buildings.
    pub structure_damage: i32,
}

/// Everything a match produced, sent once when it ends.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MatchStats {
    /// Length of the match in ticks.
    pub duration: u32,
    /// One entry per seat, sorted by [`SlotId`].
    pub slots: Vec<SlotStats>,
}

/// One accepted order, as it is told to whoever may know of it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SlotOrder {
    /// Which seat gave it.
    pub slot: SlotId,
    /// Which unit it was for. Absent means the seat's own hero.
    pub unit: Option<EntityId>,
    /// The order itself.
    pub order: Order,
}

/// Anything a participant can say to the server.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ClientMsg {
    /// First message on a connection, before anything else is accepted.
    Hello {
        /// Why this connection exists.
        role: Role,
        /// Display name for the lobby and the scoreboard.
        name: String,
    },
    /// Choose a hero for the seat this connection holds.
    PickHero {
        /// Which hero.
        hero: HeroId,
    },
    /// Declare readiness, or withdraw it. The match starts when every seat is
    /// filled and ready.
    SetReady(bool),
    /// Tell one of the units this seat drives what to do.
    ///
    /// At most one order per seat survives per tick and the last one wins, so
    /// re-sending is harmless.
    Order {
        /// Sequence number, unique per connection and increasing. A
        /// [`ServerMsg::OrderRejected`] names the order by this.
        seq: u32,
        /// Which unit it is for. Absent means the seat's own hero, which is
        /// what most orders are for.
        unit: Option<EntityId>,
        /// What to do.
        order: Order,
    },
    /// Declare that this participant has finished thinking about a tick.
    ///
    /// Only meaningful in [`TickMode::Lockstep`].
    Ack {
        /// The tick being acknowledged.
        tick: u32,
    },
    /// Choose whose eyes a spectator watches through.
    ///
    /// A seat sets the fog of the snapshots to its side's and brings that
    /// seat's own orders; absent watches everything and is told none.
    /// Ignored for a seated participant, whose eyes are its own.
    ViewAs {
        /// Which seat to watch as. Absent for the whole map.
        seat: Option<SlotId>,
    },
}

/// Anything the server can say to a participant.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ServerMsg {
    /// Accepts a [`ClientMsg::Hello`] and states the terms of the match.
    Welcome {
        /// Handle for this connection.
        player_id: PlayerId,
        /// Which seat was assigned. Absent for a spectator.
        slot: Option<SlotId>,
        /// Simulation ticks per second.
        tick_rate: u16,
        /// How the server advances ticks.
        mode: TickMode,
    },
    /// The current state of the lobby, resent whenever it changes.
    LobbyState {
        /// Every seat, sorted by [`SlotId`].
        slots: Vec<LobbySlot>,
    },
    /// The match has begun.
    MatchStart {
        /// The public description of it.
        info: MatchInfo,
    },
    /// The state of the world on one tick, already filtered through this team's
    /// fog.
    ///
    /// Sent whole on every tick. A client can start rendering from any one of
    /// these without having received an earlier one.
    Snapshot {
        /// The state. Its own [`WorldView::tick`] says which tick it is.
        view: WorldView,
    },
    /// What happened during a tick, filtered to what this team may know.
    Events {
        /// Which tick these belong to.
        tick: u32,
        /// The events, in the order the simulation produced them.
        events: Vec<EventKind>,
    },
    /// An order was not accepted.
    OrderRejected {
        /// Which order, by the sequence number it was sent with.
        seq: u32,
        /// Why.
        reason: RejectReason,
    },
    /// The orders accepted on one tick, as far as the receiver may know
    /// them.
    ///
    /// Sent to a spectator watching through one seat's eyes, and carries
    /// that seat's orders alone. Watching everything brings none, and a
    /// seated participant knows its own.
    Orders {
        /// Which tick they were applied on.
        tick: u32,
        /// The orders, sorted by [`SlotId`].
        orders: Vec<SlotOrder>,
    },
    /// The match is over.
    MatchOver {
        /// Which side won.
        winner: Team,
        /// Final numbers.
        stats: MatchStats,
    },
    /// A participant's connection ended.
    ParticipantLeft {
        /// Which connection.
        player_id: PlayerId,
        /// Which seat it held. Absent for a spectator. The seat stays in the
        /// match and its hero keeps standing there.
        slot: Option<SlotId>,
    },
}

/// One frame of a replay file.
///
/// A replay file is a sequence of length-prefixed frames, framed exactly like
/// the socket, each carrying one record. Read by the client in replay mode.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ReplayRecord {
    /// A message of the fogless spectator stream.
    Msg(ServerMsg),
    /// The orders the server accepted on one tick.
    Orders {
        /// Which tick they were applied on.
        tick: u32,
        /// At most one order per seat, sorted by [`SlotId`].
        orders: Vec<SlotOrder>,
    },
}
