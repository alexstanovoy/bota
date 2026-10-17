//! Everything the client knows, and how server messages change it.

use std::sync::Arc;

use bota_proto::{
    AbilityId, ClientMsg, EffectId, EntityId, EventKind, MatchStats, Order, PlayerView, ServerMsg,
    SlotId, StockView, TalentSlot, TalentView, Team, UnitKind, UnitView, Vec2, WorldView,
};

use crate::handshake::Lobby;
use crate::match_static::MatchStatic;
use crate::orders::{IssuedOrder, refusal};
use crate::seat::Seat;
use crate::session::{
    AckPolicy, ClientSession, DEFAULT_FORGET_AFTER_TICKS, Ingested, SessionOptions,
};
use crate::ui::derive::Derived;
use crate::ui::replay::ReplayPlayer;
use crate::ui::smooth::Smooth;

/// A live match as the app reads it once a frame: the socket client on a
/// desktop ([`Net`](crate::Net)), a transport of the embedder's own where
/// there are no sockets (a browser).
pub trait LiveLink {
    /// Everything that arrived since the last call, without waiting.
    fn poll(&mut self) -> Vec<ServerMsg>;
    /// Sends one message upstream; a failure marks the link closed.
    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()>;
    fn is_closed(&self) -> bool;
}

#[cfg(not(target_arch = "wasm32"))]
impl LiveLink for crate::socket::Net {
    fn poll(&mut self) -> Vec<ServerMsg> {
        crate::socket::Net::poll(self)
    }

    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        crate::transport::Transport::send(self, msg)
    }

    fn is_closed(&self) -> bool {
        crate::socket::Net::is_closed(self)
    }
}

/// Where messages come from: a live link or a file.
pub enum Source {
    Live(Box<dyn LiveLink>),
    Replay(Box<ReplayPlayer>),
}

impl Source {
    /// Everything due since the last frame.
    pub fn poll(&mut self, dt: f32) -> Vec<ServerMsg> {
        match self {
            Source::Live(net) => net.poll(),
            Source::Replay(player) => player.poll(dt),
        }
    }

    /// Sends upstream. A replay swallows everything.
    pub fn send(&mut self, msg: &ClientMsg) {
        if let Source::Live(net) = self {
            // A failed send marks the connection closed, which
            // `check_connection` reports.
            let _ = net.send(msg);
        }
    }
}

/// One seat's latest order, kept to be drawn over the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShownOrder {
    pub slot: SlotId,
    /// Which unit it was for. Absent means the seat's own hero.
    pub unit: Option<EntityId>,
    /// The tick it was given on, which decides how faded it is drawn.
    pub tick: u32,
    pub order: Order,
}

/// Which screen the client is on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for seats, picks and readiness.
    Lobby,
    /// The match itself, including its end banner.
    Playing,
}

/// What the camera is carried by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pin {
    /// A seat's hero, whichever body it is standing in: a respawned hero is
    /// a new entity.
    Hero(SlotId),
    /// One unit, for as long as it stands.
    Unit(EntityId),
}

/// What was last reached for, so a second reach for the same thing counts as
/// a pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tap {
    /// The key for one's own hero.
    Hero,
    /// The key for one's own courier.
    Courier,
    /// A unit, clicked in the world.
    Unit(EntityId),
    /// A seat, clicked in the top bar.
    Seat(SlotId),
}

/// Seconds after one press within which a second makes a pair.
pub const DOUBLE_TAP: f32 = 0.35;

/// Whether reaching for something makes a pair with what was reached for
/// last, and what is worth remembering afterwards.
///
/// A pair is spent when it is made: three presses are one pair and one
/// single, not two pairs.
pub fn tap_again(last: Option<(Tap, f32)>, what: Tap) -> (bool, Option<(Tap, f32)>) {
    if last.map(|(seen, _)| seen) == Some(what) {
        (true, None)
    } else {
        (false, Some((what, DOUBLE_TAP)))
    }
}

/// What a floater tells of, which decides how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloaterKind {
    /// Health taken off by a plain blow.
    Damage,
    /// Health taken off by magic.
    Magic,
    /// An attack that missed.
    Miss,
    /// A level gained.
    Level,
    /// Gold won off a kill.
    Gold,
    /// A creep denied.
    Deny,
    /// Health mended.
    Heal,
}

/// A number or word floating off a unit.
pub struct Floater {
    pub text: String,
    pub kind: FloaterKind,
    /// World position it rises from: where its unit stood when it was set
    /// off.
    pub world: (f32, f32),
    /// The unit it tells of, for a front end to keep it over that unit.
    pub unit: EntityId,
    /// Seconds since it appeared.
    pub age: f32,
}

/// Seconds a floater rises before it is gone.
pub const FLOATER_LIFE: f32 = 1.3;
/// An interactive client acknowledges a snapshot as soon as it is folded.
const SESSION_OPTIONS: SessionOptions = SessionOptions {
    ack: AckPolicy::OnSnapshot,
    forget_after_ticks: DEFAULT_FORGET_AFTER_TICKS,
};
/// Ticks per second assumed before a match tells its own.
const DEFAULT_TICK_RATE: u16 = bota_core::rules::TICKS_PER_SECOND as u16;
/// Floaters in flight at most; the oldest give way.
const MAX_FLOATERS: usize = 96;
/// Kill feed lines kept at most.
const MAX_FEED: usize = 8;
/// Cues waiting for the front end at most; more are dropped.
const MAX_CUES: usize = 64;

/// Something worth a sound or a flourish, told by the events of a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CueKind {
    /// A hero lets an attack go.
    Attack,
    /// The watched hero's attack lands.
    Hit,
    /// The watched hero lands the killing blow on a creep.
    LastHit,
    /// The watched hero denies an allied creep.
    Deny,
    /// A raze goes off.
    Raze,
    /// A requiem goes off.
    Requiem,
    /// The watched hero is low on health.
    LowHp,
    /// One's own seat buys an item.
    Buy,
    /// The watched hero gains a level.
    LevelUp,
    /// A tower strikes the watched hero.
    TowerShot,
    /// A hero falls.
    HeroDeath,
    /// The watched hero brings an enemy hero down.
    HeroKill,
    /// An order of this seat was turned down.
    Refused,
}

/// A cue: what, how loud from 0 to 1, and where in the world it happens;
/// absent for one that belongs to the watched hero itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cue {
    pub kind: CueKind,
    /// Loudness before distance, 0 to 1.
    pub gain: f32,
    /// World position it comes from.
    pub at: Option<(f32, f32)>,
}

/// One line of the kill feed.
pub struct FeedLine {
    pub text: String,
    /// Seconds since it appeared.
    pub age: f32,
}

/// The whole client.
pub struct App {
    pub source: Source,
    /// Whether the server hung up on us.
    pub lost: bool,
    pub phase: Phase,
    /// The seat this client acts as: the one it was welcomed to, or the one a
    /// replay is watched as. Absent for a spectator.
    pub my_slot: Option<SlotId>,
    /// The slot taken up and waiting for a click to aim it, if one is.
    pub aiming: Option<crate::ui::slots::Slot>,
    /// An item slot picked up and waiting for the destination click.
    pub held_item: Option<u8>,
    /// The lobby as last told: our wire identity once welcomed, and the
    /// seats as last broadcast.
    pub lobby: Lobby,
    /// Seat names from the lobby.
    pub names: Vec<(SlotId, String)>,
    /// The stream from the lobby on, and what this client knows of the
    /// match from its start.
    seat: Seat,
    /// Whether the next left click is an attack-move.
    pub attack_move_armed: bool,
    /// The unit picked, if one is. Nothing picked means one's own hero.
    pub selected: Option<EntityId>,
    /// What the camera is pinned to. Nothing means a free camera.
    pub pinned: Option<Pin>,
    /// What was last reached for, and how long is left to make it a pair.
    pub tapped: Option<(Tap, f32)>,
    /// Whether the camera has been put on one's own hero once, at the start.
    pub found_my_hero: bool,
    /// Whether readiness has been declared.
    pub ready: bool,
    /// The latest order of every seat it is known for: this seat's own in a
    /// live match, whatever the server tells a spectator, everybody's in a
    /// replay.
    pub shown_orders: Vec<ShownOrder>,
    /// The seat whose eyes a spectator has asked to watch through.
    pub eyes: Option<SlotId>,
    pub floaters: Vec<Floater>,
    pub feed: Vec<FeedLine>,
    /// Which slot the order of a sequence number came from, so a refusal
    /// can be shown on the slot that earned it.
    pub aimed_from: Option<(u32, crate::ui::slots::Slot)>,
    /// The slot whose order was last refused, and the seconds left on it.
    pub refused: Option<(crate::ui::slots::Slot, f32)>,
    /// The last rejected order, shown briefly: text and seconds left.
    pub reject: Option<(String, f32)>,
    /// The console line being typed. Absent while it is closed.
    pub console: Option<String>,
    /// The end of the match, once it came.
    pub over: Option<(Team, MatchStats)>,
    /// The clock between snapshots.
    pub smooth: Smooth,
    /// What successive snapshots tell that one alone does not.
    pub derived: Derived,
    /// Cues waiting for the front end, drained by it every frame.
    pub cues: Vec<Cue>,
    /// Seconds before the next low-health cue may come.
    pub heartbeat: f32,
    /// The seat a replay is watched as, as if it were playing.
    pub pov: Option<SlotId>,
    /// The item the quick-buy intent works towards.
    pub quick_buy: Option<u16>,
}

impl App {
    /// A client over a source, before anything has been heard.
    pub fn new(source: Source) -> App {
        App {
            source,
            lost: false,
            phase: Phase::Lobby,
            my_slot: None,
            aiming: None,
            held_item: None,
            lobby: Lobby::default(),
            names: Vec::new(),
            seat: Seat::watching(SESSION_OPTIONS),
            attack_move_armed: false,
            selected: None,
            pinned: None,
            tapped: None,
            found_my_hero: false,
            ready: false,
            shown_orders: Vec::new(),
            eyes: None,
            floaters: Vec::new(),
            feed: Vec::new(),
            aimed_from: None,
            refused: None,
            reject: None,
            console: None,
            over: None,
            smooth: Smooth::default(),
            derived: Derived::default(),
            cues: Vec::new(),
            heartbeat: 0.0,
            pov: None,
            quick_buy: None,
        }
    }

    /// What the client knows of the match, once it has started.
    pub fn session(&self) -> Option<&ClientSession> {
        self.seat.session()
    }

    /// What the match fixes at its start, once it has started.
    pub fn statics(&self) -> Option<&Arc<MatchStatic>> {
        self.seat.session().map(ClientSession::statics)
    }

    /// The freshest state of the world.
    pub fn view(&self) -> Option<&WorldView> {
        self.seat.session()?.current()
    }

    /// The state one snapshot ago, for placing and naming what has just
    /// left the view.
    pub fn prev_view(&self) -> Option<&WorldView> {
        self.seat.session()?.previous()
    }

    /// Ticks per second; a default until the match starts.
    pub fn tick_rate(&self) -> u16 {
        self.statics()
            .map_or(DEFAULT_TICK_RATE, |statics| statics.metadata().tick_rate)
    }

    /// Ticks before the game clock reaches zero.
    pub fn pregame_ticks(&self) -> u32 {
        self.statics()
            .map_or(0, |statics| statics.metadata().pregame_ticks)
    }

    /// Cells per terrain axis; zero until the match starts.
    pub fn terrain_cells(&self) -> usize {
        self.statics()
            .map_or(0, |statics| statics.metadata().terrain_cells as usize)
    }

    /// The decoded terrain cells, row-major from the south-west corner: bit 7
    /// walkable, bit 6 water, low bits the elevation tier.
    pub fn terrain(&self) -> &[u8] {
        self.seat.session().map_or(&[], |s| s.statics().terrain())
    }

    /// Every tree the map starts with.
    pub fn trees(&self) -> &[Vec2] {
        self.seat.session().map_or(&[], |s| s.statics().trees())
    }

    /// Sequence number of the latest order given.
    pub fn seq(&self) -> u32 {
        self.seat
            .session()
            .and_then(|session| session.ledger().last_seq())
            .unwrap_or(0)
    }

    /// Seconds the cast point of an ability lasts for a unit: the rules'
    /// own, shortened while a feast runs on a unit that learned the talent
    /// for it.
    pub fn cast_point_secs(&self, unit: &UnitView, ability: AbilityId) -> Option<f32> {
        let point = bota_core::ability_def(ability)?.point;
        let tree = unit.hero.and_then(bota_core::hero_def)?.talents;
        let learned = self.seat.session()?.talents_of(unit.id);
        let talent = tree.is_some_and(|tree| {
            bota_core::has_talent(tree, learned, bota_core::Talent::FeastCastSpeed)
        });
        let feasting = unit.effects.iter().any(|e| e.id == EffectId::FEASTING);
        Some(bota_core::cast_point_ms(point, feasting && talent) as f32 / 1000.0)
    }

    /// The tick a unit began the cast point it is in, if it is in one.
    pub fn cast_began(&self, unit: EntityId) -> Option<u32> {
        self.seat.session()?.entity(unit)?.casting_since
    }

    /// What one's side's shop has left of an item sold from a limited
    /// stock; absent for an item without one.
    pub fn stock_of(&self, item: u16) -> Option<StockView> {
        let team = self.my_team().or_else(|| self.fog_team())?;
        self.view()?
            .stock
            .iter()
            .find(|s| s.team == team && s.item.0 == item)
            .copied()
    }

    /// The talents of a seat's hero as the view tells them: learned, and
    /// for one's own side what could be taken; the Attribute Bonus with them.
    pub fn talents_of(&self, slot: SlotId) -> Option<TalentView> {
        self.view()?
            .talents
            .iter()
            .find(|t| t.slot == slot)
            .copied()
    }

    /// Takes a talent of one's hero's tree; talents spend no skill point.
    pub fn learn_talent(&mut self, talent: u8) {
        self.send_order(Order::LearnTalent {
            talent: TalentSlot(talent),
        });
    }

    /// Spends a skill point of one's hero on its Attribute Bonus.
    pub fn learn_attribute_bonus(&mut self) {
        self.send_order(Order::LearnAttributeBonus);
    }

    /// The name the player of a seat gave in the lobby, if any.
    pub fn player_name(&self, slot: SlotId) -> Option<&str> {
        self.names
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, name)| name.as_str())
    }

    /// The display name of a seat.
    ///
    /// A seat that sent no name, as in a replay, is called by its hero and
    /// its side, and by its number before the match tells either.
    pub fn seat_name(&self, slot: SlotId) -> String {
        if let Some((_, name)) = self.names.iter().find(|(s, _)| *s == slot) {
            return name.clone();
        }
        match self
            .player(slot)
            .and_then(|p| Some((bota_core::hero_def(p.hero)?, p.team)))
        {
            Some((hero, team)) => format!("{} ({team:?})", hero.name),
            None => format!("seat {}", slot.0),
        }
    }

    /// What is known of a unit: its live state, or the last one seen of it
    /// and how many ticks ago that was.
    pub fn known(&self, id: EntityId) -> Option<(&UnitView, u32)> {
        self.seat.session()?.known(id)
    }

    /// The body a seat last stood in, whether or not it still stands.
    pub fn body_of(&self, slot: SlotId) -> Option<EntityId> {
        self.seat.session()?.body_of(slot)
    }

    /// Our hero's unit id, while it is alive and known.
    pub fn my_hero(&self) -> Option<EntityId> {
        self.player(self.my_slot?)?.unit
    }

    /// A seat's scoreboard row in the current view.
    pub(crate) fn player(&self, slot: SlotId) -> Option<&PlayerView> {
        self.view()?.players.iter().find(|p| p.slot == slot)
    }

    /// Which unit the slot panel and the keys are about.
    ///
    /// Whatever is picked, if anything is; one's own hero otherwise.
    pub fn commanded(&self) -> Option<EntityId> {
        self.selected.or_else(|| self.my_hero())
    }

    /// Picks a unit, and pins the camera to it when `again` says the same
    /// thing was reached for twice. Picking alone never moves the camera.
    pub fn choose(&mut self, unit: Option<EntityId>, again: bool) {
        self.selected = unit;
        self.aiming = None;
        if again {
            self.pinned = unit.map(|id| match self.hero_seat(id) {
                Some(slot) => Pin::Hero(slot),
                None => Pin::Unit(id),
            });
        }
    }

    /// Which seat a unit is the standing hero of, if it is one.
    fn hero_seat(&self, id: EntityId) -> Option<SlotId> {
        let (unit, _) = self.known(id)?;
        (unit.kind == UnitKind::Hero)
            .then_some(unit.owner)
            .flatten()
    }

    /// Which unit the camera is carried by right now, if any still stands.
    pub fn pinned_unit(&self) -> Option<EntityId> {
        match self.pinned? {
            Pin::Hero(slot) => self.player(slot)?.unit,
            Pin::Unit(id) => self.view()?.units.iter().find(|u| u.id == id).map(|u| u.id),
        }
    }

    /// Whether this reach for something is the second of a pair.
    pub fn tapped_twice(&mut self, what: Tap) -> bool {
        let (again, kept) = tap_again(self.tapped, what);
        self.tapped = kept;
        again
    }

    /// The seat whose gold, stash and score the panel is about.
    ///
    /// Whoever owns what is picked, and one's own seat when what is picked
    /// answers to nobody.
    pub fn panel_slot(&self) -> Option<SlotId> {
        let owner = self
            .selected
            .and_then(|id| self.known(id))
            .and_then(|(unit, _)| unit.owner);
        owner.or_else(|| self.default_slot())
    }

    /// Our own courier, while one stands.
    pub fn my_courier(&self) -> Option<EntityId> {
        let view = self.view()?;
        let mine = self.my_slot?;
        view.units
            .iter()
            .find(|unit| unit.kind == UnitKind::Courier && unit.owner == Some(mine))
            .map(|unit| unit.id)
    }

    /// Whether a unit is one this seat drives.
    pub fn drives(&self, id: EntityId) -> bool {
        let Some(view) = self.view() else {
            return false;
        };
        view.units
            .iter()
            .find(|unit| unit.id == id)
            .is_some_and(|unit| unit.owner.is_some() && unit.owner == self.my_slot)
    }

    /// Which of our units an order goes to.
    ///
    /// Whatever of ours is selected takes it; with our own hero selected, or
    /// nothing of ours at all, the order names nobody and the server sends it
    /// to the hero.
    pub fn ordering_unit(&self) -> Option<EntityId> {
        let id = self.selected?;
        let view = self.view()?;
        let mine = self.my_slot?;
        let unit = view.units.iter().find(|unit| unit.id == id)?;
        (unit.owner == Some(mine) && Some(id) != self.my_hero()).then_some(id)
    }

    /// Which side this player is on, once the match has told us.
    pub fn my_team(&self) -> Option<Team> {
        Some(self.player(self.my_slot?)?.team)
    }

    /// Whether the current selection is a unit this seat drives, or nothing
    /// is selected.
    pub fn controls_selection(&self) -> bool {
        self.selected.is_none_or(|id| self.drives(id))
    }

    /// The team whose fog this client lives under. Absent for spectators,
    /// who see everything.
    pub fn fog_team(&self) -> Option<Team> {
        if self.my_slot.is_some() {
            return self.my_team();
        }
        // A spectator's fog is whatever eyes the snapshot came through.
        self.view()?.viewer
    }

    /// Whether our hero stands in its home shop.
    pub fn at_home_shop(&self) -> bool {
        let at = || {
            let p = self.player(self.my_slot?)?;
            let unit = self.view()?.units.iter().find(|u| Some(u.id) == p.unit)?;
            Some(bota_core::at_home_shop(
                self.session()?.map(),
                p.team,
                unit.pos,
            ))
        };
        at().unwrap_or(false)
    }

    /// The seat the bottom panel falls back to: our own, else the first one.
    pub fn default_slot(&self) -> Option<SlotId> {
        self.my_slot
            .or_else(|| Some(self.view()?.players.first()?.slot))
    }

    /// Sends an order with the next sequence number.
    pub fn send_order(&mut self, order: Order) {
        let unit = self.ordering_unit();
        self.send_order_to(unit, order);
    }

    /// Sends one order to a named unit, whatever is selected. Absent names
    /// the seat's hero. An order given before the first snapshot goes
    /// nowhere.
    pub fn send_order_to(&mut self, unit: Option<EntityId>, order: Order) {
        let Some(Ok(sent)) = self
            .seat
            .session_mut()
            .map(|session| session.issue(IssuedOrder { unit, order }))
        else {
            return;
        };
        if let Some(slot) = self.my_slot {
            self.note_order(slot, unit, sent.seen, order);
        }
        self.source.send(&sent.message());
    }

    /// Remembers one seat's order for the overlay, replacing its last one.
    pub fn note_order(&mut self, slot: SlotId, unit: Option<EntityId>, tick: u32, order: Order) {
        self.shown_orders.retain(|shown| shown.slot != slot);
        self.shown_orders.push(ShownOrder {
            slot,
            unit,
            tick,
            order,
        });
    }

    /// Which slot of our courier holds one ability.
    pub fn courier_slot(&self, ability: u16) -> Option<u8> {
        let view = self.view()?;
        let courier = self.my_courier()?;
        let unit = view.units.iter().find(|unit| unit.id == courier)?;
        unit.abilities
            .iter()
            .position(|held| held.id.0 == ability)
            .map(|slot| slot as u8)
    }

    /// Applies one server message: the lobby's to the lobby screen, and every
    /// one to the seat's stream, which builds the session at the start.
    pub fn handle(&mut self, msg: ServerMsg) {
        self.note_lobby(&msg);
        let events = match &msg {
            ServerMsg::Events { events, .. } => events.clone(),
            _ => Vec::new(),
        };
        match self.seat.ingest(msg, &mut ()) {
            Ok(Ingested::Started { .. }) => {
                self.shown_orders.clear();
                self.phase = Phase::Playing;
            }
            Ok(Ingested::Snapshot { ack, .. }) => self.on_snapshot(ack),
            Ok(Ingested::TickComplete { .. }) => {
                for event in events {
                    self.absorb_event(event);
                }
            }
            Ok(Ingested::Rejected { seq, reason }) => self.on_rejected(seq, reason),
            Ok(Ingested::Over { winner, stats }) => self.over = Some((winner, stats)),
            Ok(Ingested::Passed(_)) => {}
            Err(error) => self.fail(&error.to_string()),
        }
    }

    /// What the lobby screen and the overlay take from a message, before the
    /// match and during it.
    fn note_lobby(&mut self, msg: &ServerMsg) {
        self.lobby.observe(msg);
        match msg {
            ServerMsg::Welcome { slot, .. } => {
                self.my_slot = slot.or(self.pov);
                let live = matches!(self.source, Source::Live(_));
                if let (true, Some(seated), None) = (live, self.lobby.seated(), self.seat.session())
                {
                    self.seat = Seat::new(seated, SESSION_OPTIONS);
                }
            }
            ServerMsg::LobbyState { slots } => {
                self.names = slots
                    .iter()
                    .filter(|s| !s.name.is_empty())
                    .map(|s| (s.slot, s.name.clone()))
                    .collect();
            }
            ServerMsg::Orders { tick, orders } => {
                for given in orders {
                    self.note_order(given.slot, given.unit, *tick, given.order);
                }
            }
            ServerMsg::ParticipantLeft {
                slot: Some(slot), ..
            } => {
                let text = format!("{} disconnected", self.seat_name(*slot));
                self.feed.push(FeedLine { text, age: 0.0 });
            }
            _ => {}
        }
    }

    /// Answers the acknowledgement the server waits for, and turns what the
    /// folded snapshot brought into effects.
    fn on_snapshot(&mut self, ack: Option<u32>) {
        if let Some(tick) = ack {
            self.source.send(&ClientMsg::Ack { tick });
        }
        // The events of snapshots the server skipped for a slow client, so
        // no popup, sound or feed line is lost.
        let skipped: Vec<EventKind> = session_skipped(self.seat.session());
        for event in skipped {
            self.absorb_event(event);
        }
        self.smooth.snapshot();
        if let Some(view) = self.seat.session().and_then(ClientSession::current) {
            self.derived.snapshot(view);
        }
        self.launch_cues();
    }

    /// Shows a refused order, on the slot that earned it.
    fn on_rejected(&mut self, seq: u32, reason: bota_proto::RejectReason) {
        self.reject = Some((refusal(reason).to_string(), 2.5));
        self.cue(CueKind::Refused, 0.8, None);
        self.refused = self
            .aimed_from
            .filter(|(sent, _)| *sent == seq)
            .map(|(_, slot)| (slot, 1.2));
    }

    /// The server's stream broke the rules of a match: stop listening.
    fn fail(&mut self, why: &str) {
        self.lost = true;
        self.feed.push(FeedLine {
            text: format!("the server broke the protocol: {why}"),
            age: 0.0,
        });
    }

    /// Notices a dead socket once and says so.
    pub fn check_connection(&mut self) {
        if self.lost || self.over.is_some() {
            return;
        }
        if let Source::Live(net) = &self.source
            && net.is_closed()
        {
            self.lost = true;
            self.feed.push(FeedLine {
                text: "connection to the server lost".to_string(),
                age: 0.0,
            });
        }
    }

    /// One frame of the client: takes in whatever arrived, moves the
    /// clock between snapshots on, and ages the transient effects.
    pub fn pump(&mut self, dt: f32) {
        self.smooth.advance(dt);
        let msgs = self.source.poll(dt);
        for msg in msgs {
            self.handle(msg);
        }
        self.check_connection();
        self.tick_effects(dt);
        self.heartbeat_cue(dt);
        if let Some(view) = self.seat.session().and_then(ClientSession::current) {
            self.derived.trail(view, dt);
            if let Some(target) = self.quick_buy
                && let Some(p) = self.my_slot.and_then(|slot| self.player(slot))
                && crate::shop::held_items(view, p).contains(&bota_proto::ItemId(target))
            {
                self.quick_buy = None;
            }
        }
    }

    /// Asks the front end for a cue, dropped when too many wait.
    pub fn cue(&mut self, kind: CueKind, gain: f32, at: Option<(f32, f32)>) {
        if self.cues.len() < MAX_CUES {
            self.cues.push(Cue { kind, gain, at });
        }
    }

    /// Ages and expires the transient effects.
    pub fn tick_effects(&mut self, dt: f32) {
        for f in &mut self.floaters {
            f.age += dt;
        }
        self.floaters.retain(|f| f.age < FLOATER_LIFE);
        if self.floaters.len() > MAX_FLOATERS {
            let extra = self.floaters.len() - MAX_FLOATERS;
            self.floaters.drain(..extra);
        }
        if self.feed.len() > MAX_FEED {
            let extra = self.feed.len() - MAX_FEED;
            self.feed.drain(..extra);
        }
        for l in &mut self.feed {
            l.age += dt;
        }
        self.feed.retain(|l| l.age < 8.0);
        count_down(&mut self.reject, dt);
        count_down(&mut self.refused, dt);
        count_down(&mut self.tapped, dt);
    }
}

/// Takes `dt` seconds off a timed notice, and clears it once its time is up.
fn count_down<T>(notice: &mut Option<(T, f32)>, dt: f32) {
    if let Some((_, left)) = notice {
        *left -= dt;
        if *left <= 0.0 {
            *notice = None;
        }
    }
}

/// The events of the ticks the latest snapshot says the server skipped.
fn session_skipped(session: Option<&ClientSession>) -> Vec<EventKind> {
    session
        .and_then(ClientSession::current)
        .map(|view| {
            view.skipped_ticks
                .iter()
                .flat_map(|skipped| skipped.events.iter().cloned())
                .collect()
        })
        .unwrap_or_default()
}
