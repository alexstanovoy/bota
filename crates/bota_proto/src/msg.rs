//! The messages themselves, and the lobby types they carry.

use crate::{
    EntityId, EventKind, HeroId, ItemId, MapId, Order, PlayerId, SeatKey, SlotId, Team, Vec2,
    WorldView,
};
use serde::{Deserialize, Serialize};

/// Why a participant connected.
///
/// A player and a bot take a seat and see through its team's fog of war; a
/// spectator takes none.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// A human at a client, taking a seat.
    Player,
    /// A program taking a seat. The server treats it exactly as a player.
    Bot,
    /// An observer with no seat.
    Spectator,
    /// A connection that is told the lobby, the start, departures and the end,
    /// and never a tick.
    Referee,
}

/// How the server decides when to advance a tick.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TickMode {
    /// Advance on a wall clock at the configured rate. An order that misses
    /// its tick applies on the next one.
    Realtime,
    /// Advance once every connected seat has acknowledged the tick, or when
    /// the server's acknowledgement timeout runs out.
    Lockstep,
}

/// One seat's hero choice.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pick {
    pub slot: SlotId,
    pub team: Team,
    /// Which hero it picked.
    pub hero: HeroId,
}

/// One row of the lobby, before the match starts.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LobbySlot {
    /// Which seat this row is.
    pub slot: SlotId,
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
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ShopEntry {
    pub id: ItemId,
    /// What it costs whole, in gold.
    pub cost: i32,
    /// What it is built from. Empty for one bought whole.
    pub components: Vec<ItemId>,
    /// Other sets of parts it is built from as well. Empty for one with a
    /// single recipe.
    pub alternatives: Vec<Vec<ItemId>>,
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
    /// Every tree the map starts with.
    /// [`WorldView::felled_trees`](crate::WorldView::felled_trees) indexes
    /// into this.
    pub trees: Vec<Vec2>,
    /// Cells per terrain axis.
    pub terrain_cells: u32,
    /// Run-length encoded terrain cells, row-major from the south-west
    /// corner, one byte each: bit 7 walkable ground, bit 6 river water, the
    /// low bits the elevation tier.
    pub terrain_rle: Vec<(u16, u8)>,
    /// `(x, y)` cells of the terrain grid that block sight lines regardless of
    /// elevation: the map's own fog blocker walls. Trees block by elevation
    /// and come and go, so they are not here: a client lays them from
    /// [`MatchInfo::trees`] and the view's felled and planted trees.
    pub opaque_cells: Vec<(u16, u16)>,
    /// How the server advances ticks.
    pub mode: TickMode,
    /// Every seat and its hero, sorted by [`SlotId`].
    pub picks: Vec<Pick>,
    /// Everything the shop sells, in item id order.
    pub shop: Vec<ShopEntry>,
    /// Fountain of each team, Radiant then Dire.
    pub fountains: [Vec2; 2],
    /// How close to its own fountain a hero counts as standing in the home
    /// shop, in world units.
    pub shop_range: i32,
}

/// Why the server refused an order, or a [`ClientMsg::Hello`].
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RejectReason {
    HeroDead,
    /// The target does not exist, or is not currently visible to this team.
    UnknownTarget,
    /// The ability or item does not accept this kind of target.
    WrongTargetKind,
    /// The ability or item is still on cooldown.
    OnCooldown,
    NotEnoughMana,
    NotEnoughGold,
    /// The referenced ability or inventory slot is empty.
    EmptySlot,
    /// The ability works on its own and is never cast, or the item cannot be
    /// used.
    NotCastable,
    /// The ability has no points in it yet.
    NotLearned,
    /// The item has no charges left to spend.
    NoCharges,
    /// The item has come out of the backpack and is not working yet, or a
    /// mana item has nothing to fill.
    NotReady,
    /// The order named a unit this seat does not drive.
    NotYourUnit,
    /// No item with this id is sold.
    UnknownItem,
    /// Moving an item into or out of the stash requires standing at the
    /// home shop.
    NotAtShop,
    InventoryFull,
    /// The unit is stunned, feared or channelling.
    Disabled,
    /// Only the seat that bought an item may sell it or mark it for sale.
    NotYourItem,
    /// The spot aimed at is ground nothing may stand on.
    ClosedGround,
    /// The slot named is not one the unit carries on itself.
    NotInBag,
    /// The match was not started with cheats on.
    NoCheats,
    /// The cheat payload is the wrong shape or outside its bounds.
    BadCheat,
    /// The shop has none of the item left until it restocks.
    OutOfStock,
    /// No skill point is left, the hero's tree has no such talent, its tier
    /// has not opened at the hero's level, or the talent or its tier is
    /// already taken.
    CannotLearn,
    /// The stack holds a scroll that came for nothing: the one every hero
    /// starts with or one a death handed out. No shop buys it back.
    Unsellable,
    /// A `Hello` admitted to nothing: no key where one is needed, a key no
    /// seat or role is held for, no open seat, or an unreserved seat asked
    /// for mid-match without the key that claimed it.
    NotAdmitted,
    /// A `Hello` with the right key for a seat a live connection holds. It
    /// may come back once that connection has ended or been found dead.
    SeatHeld,
}

/// Final numbers for one seat.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotStats {
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

/// An order as a client sends it: the fields of [`ClientMsg::Order`], with the
/// tick it was decided on always told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutgoingOrder {
    /// Above every sequence number before it; a rejection carries it back.
    pub seq: u32,
    /// Which unit it is for. Absent means the seat's own hero.
    pub unit: Option<EntityId>,
    /// What to do.
    pub order: Order,
    /// The tick of the snapshot it was decided on, from which the server
    /// measures the order delay.
    pub seen: u32,
}

impl OutgoingOrder {
    /// The message that carries it.
    pub const fn message(self) -> ClientMsg {
        ClientMsg::Order {
            seq: self.seq,
            unit: self.unit,
            order: self.order,
            seen: Some(self.seen),
        }
    }
}

impl ClientMsg {
    /// The first message on a connection, stamped with this build's
    /// [`WIRE_FINGERPRINT`](crate::WIRE_FINGERPRINT).
    pub fn hello(role: Role, name: impl Into<String>, key: Option<SeatKey>) -> ClientMsg {
        ClientMsg::Hello {
            role,
            name: name.into(),
            key,
            wire_fingerprint: crate::WIRE_FINGERPRINT,
        }
    }
}

/// Anything a participant can say to the server.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ClientMsg {
    /// First message on a connection, before anything else is accepted.
    Hello {
        /// Why this connection exists.
        role: Role,
        /// Display name for the lobby and the scoreboard, cut to 32 bytes
        /// (see the [`contract`](crate::contract)).
        name: String,
        /// The key a server with reserved seats asks for.
        key: Option<SeatKey>,
        /// The [`WIRE_FINGERPRINT`](crate::WIRE_FINGERPRINT) the sender was
        /// built with. A server built with another refuses the connection with
        /// [`ServerMsg::WireMismatch`]. Last on the wire.
        wire_fingerprint: u64,
    },
    /// Choose a hero for the seat this connection holds.
    PickHero { hero: HeroId },
    /// Declare readiness, or withdraw it. The match starts when every seat is
    /// filled, picked and ready.
    SetReady(bool),
    /// Tell one of the units this seat drives what to do.
    ///
    /// At most one order per seat is applied per tick: the last one accepted.
    Order {
        /// Chosen by the sender. A [`ServerMsg::OrderRejected`] names the
        /// order by this.
        seq: u32,
        /// Which unit it is for. Absent means the seat's own hero.
        unit: Option<EntityId>,
        /// What to do.
        order: Order,
        /// The tick of the view the order was given on. Absent when the
        /// sender does not say; the server measures no delay from it then.
        /// Last on the wire.
        seen: Option<u32>,
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
    /// Ignored for a seated participant.
    ViewAs {
        /// Which seat to watch as. Absent, or a seat not in the match, for the
        /// whole map.
        seat: Option<SlotId>,
    },
    /// Answers a [`ServerMsg::Ping`]. Any message shows the connection is
    /// alive; this one is for a participant with nothing else to say.
    Pong,
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
    /// fog. Sent whole on every tick.
    Snapshot {
        /// The state. Its own [`WorldView::tick`] says which tick it is.
        view: WorldView,
    },
    /// What happened during a tick, filtered to what this team may know.
    /// Sent after every snapshot, including an empty batch, to complete that tick.
    Events {
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
    /// Sent only to a spectator watching through one seat's eyes, and
    /// carries that seat's orders alone.
    Orders {
        /// Which tick they were applied on.
        tick: u32,
        /// The orders, sorted by [`SlotId`].
        orders: Vec<SlotOrder>,
    },
    MatchOver {
        /// Which side won. [`Team::Neutral`] for a draw.
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
    /// Refuses a [`ClientMsg::Hello`] built with another wire: the
    /// connection closes after it. The registries only grow, so every build
    /// that has this message reads it, whatever else differs.
    WireMismatch {
        /// The [`WIRE_FINGERPRINT`](crate::WIRE_FINGERPRINT) the server was
        /// built with.
        server_fingerprint: u64,
    },
    /// Sent about once a second on every connection, whatever else is sent,
    /// so a client hears from a live server even in a quiet lobby. A
    /// participant answers with [`ClientMsg::Pong`] (or any other message);
    /// one that stays silent too long is disconnected.
    Ping,
    /// Refuses a [`ClientMsg::Hello`]: the connection closes after it.
    Refused {
        /// Why: [`RejectReason::NotAdmitted`] or [`RejectReason::SeatHeld`].
        reason: RejectReason,
    },
}

/// One frame of a replay file.
///
/// A replay file is a sequence of frames, framed exactly like the socket, each
/// carrying one record.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "a reader holds one record at a time, and every replay reader matches on the unboxed message"
)]
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
