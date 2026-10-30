//! The messages themselves, and the lobby types they carry.

use crate::{
    EntityId, EventKind, Heard, HeroId, ItemId, Line, MapId, ObjectiveSet, Order, PlayerId,
    RuneSet, SayRefusal, SeatKey, SlotId, Team, Vec2, WorldView,
};
use serde::{Deserialize, Serialize};

/// Why a participant connected.
///
/// A player and a bot take a seat and see through its team's fog of war; a
/// spectator takes none.
///
/// Frozen, as [`ClientMsg::Hello`] that carries it: a role no older server
/// knows would end the connection before the server reads the fingerprint.
/// Finer kinds of watching come from the key a spectator presents.
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

/// Where the hero selection stands (see the [`contract`](crate::contract)).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickPhase {
    /// The clock has not started: not every seat has been held yet.
    #[default]
    Waiting,
    /// The selection time is running.
    Selecting {
        /// Milliseconds until it runs out, as the server saw it when it sent
        /// this.
        left_ms: u32,
    },
    /// The selection time ran out: a seat still without a hero loses starting
    /// gold for every second until it picks.
    Late {
        /// Milliseconds until a seat still without a hero is given a random
        /// one, as the server saw it when it sent this.
        left_ms: u32,
    },
    /// Every hero is picked and final: the tactical phase, in which the
    /// seats plan their start (the shop is theirs to browse) before the match
    /// begins.
    Tactical {
        /// Milliseconds until the match begins, as the server saw it when it
        /// sent this.
        left_ms: u32,
    },
}

/// What the shop asks for one item.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ShopEntry {
    pub id: ItemId,
    /// What it costs whole, in gold.
    pub cost: i32,
    /// What it is built from. Empty for one bought whole.
    #[serde(
        deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_SHOP_COMPONENTS }>"
    )]
    pub components: Vec<ItemId>,
    /// Other sets of parts it is built from as well. Empty for one with a
    /// single recipe.
    #[serde(
        deserialize_with = "crate::bounded::seq_of_seq::<_, _, { crate::limits::MAX_SHOP_RECIPES }, { crate::limits::MAX_SHOP_COMPONENTS }>"
    )]
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
    #[serde(deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_STATIC_TREES }>")]
    pub trees: Vec<Vec2>,
    /// Cells per terrain axis.
    pub terrain_cells: u32,
    /// Run-length encoded terrain cells, row-major from the south-west
    /// corner, one byte each: bit 7 walkable ground, bit 6 river water, the
    /// low bits the elevation tier.
    #[serde(deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_TERRAIN_RUNS }>")]
    pub terrain_rle: Vec<(u16, u8)>,
    /// `(x, y)` cells of the terrain grid that block sight lines regardless of
    /// elevation: the map's own fog blocker walls. Trees block by elevation
    /// and come and go, so they are not here: a client lays them from
    /// [`MatchInfo::trees`] and the view's felled and planted trees.
    #[serde(deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_OPAQUE_CELLS }>")]
    pub opaque_cells: Vec<(u16, u16)>,
    /// How the server advances ticks.
    pub mode: TickMode,
    /// Every seat and its hero, sorted by [`SlotId`].
    #[serde(deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_SEATS }>")]
    pub picks: Vec<Pick>,
    /// Everything the shop sells, in item id order. Derived: the build's
    /// rules state the same shop (`bota_core::sold_items`, and the wire
    /// fingerprint pins the two equal), which a client that has them reads
    /// instead; this is the statement for a client that has not.
    #[serde(deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_SHOP_ENTRIES }>")]
    pub shop: Vec<ShopEntry>,
    /// Fountain of each team, Radiant then Dire.
    pub fountains: [Vec2; 2],
    /// How close to its own fountain a hero counts as standing in the home
    /// shop, in world units.
    pub shop_range: i32,
    /// Whether the match honours cheat orders, as its host set it: a client
    /// and a replay tell a cheat match by it, as Dota marks one.
    pub cheats: bool,
    /// Which of the map's objectives stand in the match.
    pub objectives: ObjectiveSet,
    /// Which kinds of rune the match spawns.
    pub runes: RuneSet,
    /// The items the match plays with: the catalogue every seat holds. No
    /// id past it is sold, held or told in the match. Last on the wire.
    pub catalogue: CataloguePrefix,
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
    /// The unit's queue holds [`MAX_QUEUED_ORDERS`](crate::limits::MAX_QUEUED_ORDERS) orders
    /// already, or the seat gave more than
    /// [`MAX_ORDERS_PER_TICK`](crate::limits::MAX_ORDERS_PER_TICK) orders that would take effect
    /// on one tick.
    QueueFull,
    /// A [`ClientMsg::Catalogue`] whose items differ from the server's, or
    /// one shorter than a match under way plays with.
    CatalogueMismatch,
    /// A [`Disassemble`](crate::Order::Disassemble) of an item that cannot be
    /// taken apart now: it is built of nothing, its rule is never, or its
    /// rule allows it only within a while of its being combined and that has
    /// run out.
    NotDisassemblable,
    /// A [`SetCombineLock`](crate::Order::SetCombineLock) on an item that is
    /// part of no recipe.
    CannotLockCombining,
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
    #[serde(deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_SEATS }>")]
    pub slots: Vec<SlotStats>,
}

/// What a server that runs one match for a launcher reports when the match
/// is over: postcard bytes, hex-encoded on its `RESULT` line.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MatchReport {
    /// Every seat and the hero it played, sorted by [`SlotId`].
    #[serde(deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_SEATS }>")]
    pub picks: Vec<Pick>,
    /// The final numbers, as [`ServerMsg::MatchOver`] told them.
    pub stats: MatchStats,
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
    /// Whether it was given queued, to wait its turn behind what the unit
    /// does and has queued. Last on the wire.
    pub queue: bool,
}

impl SlotOrder {
    /// Calls `visit` on every entity handle the order carries: the unit it
    /// is for and the one it names.
    pub fn visit_ids(&mut self, visit: &mut dyn FnMut(&mut EntityId)) {
        self.unit.iter_mut().for_each(&mut *visit);
        self.order.visit_ids(visit);
    }
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
    /// Whether it waits its turn in the unit's queue (Dota's Shift).
    pub queue: bool,
}

impl OutgoingOrder {
    /// The message that carries it.
    pub const fn message(self) -> ClientMsg {
        ClientMsg::Order {
            seq: self.seq,
            unit: self.unit,
            order: self.order,
            seen: Some(self.seen),
            queue: self.queue,
        }
    }
}

impl ClientMsg {
    /// The first message on a connection, stamped with the sender's wire
    /// fingerprint: `bota_core::WIRE_FINGERPRINT`, which folds the rules a
    /// client checks its orders against into the
    /// [`REGISTRY_DIGEST`](crate::REGISTRY_DIGEST).
    pub fn hello(
        role: Role,
        name: impl Into<String>,
        key: Option<SeatKey>,
        wire_fingerprint: u64,
    ) -> ClientMsg {
        ClientMsg::Hello {
            role,
            name: name.into(),
            key,
            wire_fingerprint,
        }
    }
}

/// Anything a participant can say to the server.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ClientMsg {
    /// First message on a connection, before anything else is accepted.
    ///
    /// Frozen: postcard reads fields by position, so a field added here
    /// would leave a server without it unable to read the hello of a client
    /// with it, and the two builds would part without
    /// [`ServerMsg::WireMismatch`] naming them. What a later build has to
    /// say first goes into a message of its own after the
    /// [`ServerMsg::Welcome`].
    Hello {
        /// Why this connection exists.
        role: Role,
        /// Display name for the lobby and the scoreboard, cut to 32 bytes
        /// (see the [`contract`](crate::contract)).
        name: String,
        /// The key a server with reserved seats asks for.
        key: Option<SeatKey>,
        /// The wire fingerprint the sender was built with
        /// (`bota_core::WIRE_FINGERPRINT`: the registries and the rules). A
        /// server built with another refuses the connection with
        /// [`ServerMsg::WireMismatch`]. Last on the wire.
        wire_fingerprint: u64,
    },
    /// Choose a hero for the seat this connection holds. A hero the server
    /// does not allow, or any pick once the seat was ready with one, changes
    /// nothing (see the [`contract`](crate::contract)).
    PickHero { hero: HeroId },
    /// Declare readiness, or withdraw it. The match starts when every seat is
    /// filled, picked and ready.
    SetReady(bool),
    /// Tell one of the units this seat drives what to do.
    ///
    /// Each unit takes at most one order given unqueued per tick, the last
    /// one accepted, and every queued order given after it (see the
    /// [`contract`](crate::contract)).
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
        seen: Option<u32>,
        /// Whether the order waits its turn behind what the unit does and has
        /// queued, as Dota's Shift-queue; unqueued, an order that
        /// [`queues`](Order::queues) replaces the unit's queue. Last on the
        /// wire.
        queue: bool,
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
    /// A seat sets the fog of the snapshots to its side's; absent watches
    /// everything. Borrowed eyes are never told orders. Ignored for a seated
    /// participant and for a spectator whose key fixed its eyes.
    ViewAs {
        /// Which seat to watch as. Absent, or a seat not in the match, for the
        /// whole map.
        seat: Option<SlotId>,
    },
    /// Answers a [`ServerMsg::Ping`]. Any message shows the connection is
    /// alive; this one is for a participant with nothing else to say.
    Pong,
    /// Says a line to an audience. The server passes it on as
    /// [`ServerMsg::Heard`], to the speaker too, or refuses it with
    /// [`ServerMsg::SayRefused`] (see the [`contract`](crate::contract)).
    Say(Line),
    /// Asks the server to compress what it sends on this connection from now
    /// on, chat aside, as [`ServerMsg::Compressed`] parts of one stream.
    /// Allowed once the `Hello` is sent; a second one changes nothing.
    Compress(Codec),
    /// Leaves the match for good, as Dota's LEAVE GAME: a seat that sends it while the match is
    /// being played leaves it, and its team loses at once (the game is then reported as
    /// won by the others, naming the leaver). Once the match is decided it changes nothing.
    /// Ignored in the lobby, where closing the connection frees the seat.
    LeaveGame,
    /// Asks the server to answer at once with [`ServerMsg::Echo`] naming the same `id`, so the
    /// sender can time the round trip of the connection. The server's network side answers it
    /// the moment it reads it, on the connection's own threads: never through the simulation, so
    /// the answer waits for no tick and nothing of it reaches the match. Allowed on any
    /// connection, `Hello` or not.
    Probe { id: u32 },
    /// The items the sender was built with, said right after the `Hello`:
    /// the server plays a match on the catalogue every seat holds, so a
    /// client built before an item was added plays on, and refuses one whose
    /// items differ from its own with [`RejectReason::CatalogueMismatch`]. A
    /// client that says nothing is taken to hold the server's own.
    Catalogue(CataloguePrefix),
}

/// The first `items` item ids of a build's catalogue and a digest of what
/// they are: every number and rule a match plays them by. Ids only grow, so
/// two builds agree on the items of the shorter one's prefix exactly when
/// the longer one's digest of that prefix equals the shorter one's.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CataloguePrefix {
    /// How many item ids, from the first.
    pub items: u16,
    /// The digest of those items (`bota_core`'s).
    pub digest: u64,
}

/// How a connection's stream is compressed.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Codec {
    /// Raw DEFLATE (RFC 1951) with its 32 KiB window, flushed with an empty
    /// stored block after every message or tick the server sends.
    Deflate,
}

/// Anything the server can say to a participant.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ServerMsg {
    /// Accepts a [`ClientMsg::Hello`] and states the terms of the match.
    Welcome {
        /// Handle for this connection: seats are numbered from one in the
        /// order they were taken, watchers apart from them, so the number
        /// says nothing of how many came to watch.
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
        #[serde(deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_SEATS }>")]
        slots: Vec<LobbySlot>,
        /// Whether two seats may pick the same hero. When not, a hero some
        /// seat holds is refused to the others.
        same_heroes: bool,
        /// The state of the selection clock at the moment of sending.
        pick: PickPhase,
        /// The heroes a seat may pick: the server's pool (every hero when it names none). A pool
        /// of one is a forced pick: the server gives it to each seat as it is taken, ready.
        /// Last on the wire.
        heroes: Vec<HeroId>,
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
        #[serde(
            deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_EVENTS_PER_BATCH }>"
        )]
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
    /// Sent live only to a spectator whose seat key fixed its eyes, carrying
    /// that seat's orders alone; and late, with every seat's orders, to a
    /// late spectator watching the whole map (see the
    /// [`contract`](crate::contract)).
    Orders {
        /// Which tick they were applied on.
        tick: u32,
        /// The orders, in the order they were applied: by [`SlotId`], then
        /// in the order each seat gave them.
        #[serde(
            deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_ORDERS_TOLD }>"
        )]
        orders: Vec<SlotOrder>,
    },
    MatchOver {
        /// Which side won. [`Team::Neutral`] for a draw.
        winner: Team,
        /// Final numbers.
        stats: MatchStats,
    },
    /// A participant's connection ended. Every connection is told of a
    /// seat's; of a spectator's or a referee's only the spectators are, as
    /// players in Dota are not told about spectators.
    ParticipantLeft {
        /// Which connection.
        player_id: PlayerId,
        /// Which seat it held. Absent for a spectator. The seat stays in the
        /// match and its hero keeps standing there.
        slot: Option<SlotId>,
    },
    /// Refuses a [`ClientMsg::Hello`] built with another wire: the
    /// connection closes after it. The registries only grow, so every build
    /// that has this message reads it, whatever else differs. Frozen, as the
    /// hello is: a client with a field added here could not read the refusal
    /// of a server without it.
    WireMismatch {
        /// The wire fingerprint the server was built with.
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
    /// A line said to this connection's audience. Never compressed.
    Heard(Heard),
    /// The line this connection said last was not passed on.
    SayRefused { reason: SayRefusal },
    /// The next bytes of this connection's compressed stream, once it asked
    /// for one with [`ClientMsg::Compress`]: they unpack into whole frames of
    /// server messages, the way the connection itself carries them, and the
    /// stream is flushed at the end of each part, so a part never ends inside
    /// a frame of the next (see the [`contract`](crate::contract)).
    Compressed(Vec<u8>),
    /// The answer to a [`ClientMsg::Probe`], carrying its `id`. Sent ahead of anything still
    /// queued for the connection, but behind a frame already going out.
    Echo { id: u32 },
    /// Tells that an order was received, the moment the connection's network side read it: before
    /// the simulation has seen it, so it says nothing of whether the order is valid (a refusal
    /// follows as [`ServerMsg::OrderRejected`]) and answers a round trip, not a tick. Sent ahead
    /// of anything still queued, once for every [`ClientMsg::Order`] that passed the
    /// connection's allowance.
    OrderReceived {
        /// The order, by the sequence number it was sent with.
        seq: u32,
        /// The earliest tick the order can take effect on: the one after the tick the world stood
        /// on when the order was read. A server that delays orders on purpose
        /// (`--order-delay-ticks`) carries it out later still; the first snapshot after it tells
        /// the measured delay ([`WorldView::order_delays`](crate::WorldView::order_delays)).
        tick: u32,
        /// When it was read, in microseconds since the connection opened, on the server's
        /// monotonic clock.
        at_us: u64,
    },
}

/// What a replay file is, and which wire recorded it: its first record.
///
/// Frozen: no field is ever added to it, so every build reads the header of
/// a replay any build wrote, and a player refuses a replay of another wire by
/// naming both fingerprints rather than failing on some record in its
/// middle. Whatever a later build has to say about a replay goes into a
/// record of its own after the header.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReplayHeader {
    /// [`ReplayHeader::MAGIC`]: marks the file as a bota replay.
    pub magic: [u8; 8],
    /// The wire fingerprint of the build that recorded it
    /// (`bota_core::WIRE_FINGERPRINT`).
    pub wire_fingerprint: u64,
}

impl ReplayHeader {
    /// The bytes every replay's header starts with.
    pub const MAGIC: [u8; 8] = *b"bota.brp";

    /// The header a build of `wire_fingerprint` writes, and the only one it
    /// plays.
    pub const fn new(wire_fingerprint: u64) -> ReplayHeader {
        ReplayHeader {
            magic: ReplayHeader::MAGIC,
            wire_fingerprint,
        }
    }
}

/// One frame of a replay file.
///
/// A replay file is a sequence of frames, framed exactly like the socket, each
/// carrying one record: a [`ReplayRecord::Header`] first, then the match. A
/// server writes the frames as one Zstandard stream (RFC 8878); a player
/// takes them so or as they are, telling the two apart by their first bytes
/// (see the [`contract`](crate::contract)).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    target_pointer_width = "64",
    expect(
        clippy::large_enum_variant,
        reason = "a reader holds one record at a time, and every replay reader matches on the unboxed message"
    )
)]
pub enum ReplayRecord {
    /// A message of the fogless spectator stream.
    Msg(ServerMsg),
    /// The orders the server accepted on one tick.
    Orders {
        /// Which tick they were applied on.
        tick: u32,
        /// The orders, in the order they were applied: by [`SlotId`], then
        /// in the order each seat gave them.
        #[serde(
            deserialize_with = "crate::bounded::seq::<_, _, { crate::limits::MAX_ORDERS_TOLD }>"
        )]
        orders: Vec<SlotOrder>,
    },
    /// The first record of every replay, and of no other place in it.
    Header(ReplayHeader),
    /// A line said in the lobby or the match, recorded as the server took it
    /// in, among the records of the tick it stood on.
    Heard(Heard),
}
