//! The client's picture of a match, kept from the messages of one stream.
//!
//! A [`ClientSession`] is a reducer: its state is a function of the match
//! description, its options and the messages it was given, in order. It has
//! no clock, no randomness and no I/O; it never sends. Orders and
//! acknowledgements are returned for the caller to transmit, and a client's
//! own state rides along through an [`Observer`].

mod error;
mod memory;
mod missiles;
mod tracks;
mod validate;

use std::collections::VecDeque;
use std::sync::Arc;

use bota_core::{MapDef, map_of};
use bota_proto::{
    EntityId, EventKind, HeroId, MatchInfo, MatchStats, OutgoingOrder, PlayerView, RejectReason,
    ServerMsg, SlotId, Talents, Team, TickMode, UnitView, WorldView,
};

pub use error::{IngestError, SessionError};
pub use missiles::{LootTrack, MissileTrack};
pub use tracks::{
    AbilityCastObservation, DamageObservation, DeathObservation, EntityTrack, HealObservation,
    ManaRestorationObservation, ObservedEvent, PossibleAttackLanded, VelocityEstimate,
};

use crate::handshake::{Seated, check_match};
use crate::limits::{MAX_MESSAGES_WITHOUT_SNAPSHOT, MAX_RECENT_EVENTS};
use crate::match_static::{MatchStatic, seat_team};
use crate::observer::{EventSource, Observer};
use crate::orders::{IssuedOrder, OrderLedger};
use crate::readiness::{SharedWaits, wait_started_by};
use memory::BodyMemory;
use missiles::Missiles;
use tracks::{Owner, Tracks};

/// When a session hands out the acknowledgement a lockstep server waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AckPolicy {
    /// A client that decides its orders per tick: [`ClientSession::end_turn`]
    /// returns the tick to acknowledge once it has decided.
    AfterTurn,
    /// An interactive client: the snapshot's own ingest result carries it.
    OnSnapshot,
    /// A caller that drives the match itself and acknowledges nothing.
    Never,
}

/// Ticks a unit out of view stays tracked by default: sixteen seconds at the
/// usual rate, long enough to name what just left the view.
pub const DEFAULT_FORGET_AFTER_TICKS: u32 = 480;

/// How a session is set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionOptions {
    pub ack: AckPolicy,
    /// Ticks an unseen unit stays tracked.
    pub forget_after_ticks: u32,
}

impl Default for SessionOptions {
    /// A bot's: it acknowledges each tick once it has decided on it.
    fn default() -> Self {
        SessionOptions {
            ack: AckPolicy::AfterTurn,
            forget_after_ticks: DEFAULT_FORGET_AFTER_TICKS,
        }
    }
}

/// What one message did to a session.
#[derive(Clone, Debug, PartialEq)]
pub enum Ingested {
    /// The match started: a [`Seat`](crate::Seat) built its session. A
    /// spectator has no slot, and sees as [`ClientSession::team`] tells.
    Started { slot: Option<SlotId>, team: Team },
    /// A message that is not part of the match's state: the welcome, the
    /// lobby, the orders told to a spectator, a departure.
    Passed(Box<ServerMsg>),
    /// The server refused an order; the session rolled it back.
    Rejected { seq: u32, reason: RejectReason },
    /// A snapshot was folded.
    Snapshot {
        tick: u32,
        /// The seat's hero is another unit than before (it died and came back).
        body_changed: bool,
        /// The tick to acknowledge now, under [`AckPolicy::OnSnapshot`].
        ack: Option<u32>,
    },
    /// The events of a tick were folded: the tick is whole.
    TickComplete { tick: u32 },
    /// The match is over; a neutral winner is a draw.
    Over { winner: Team, stats: MatchStats },
}

/// The rules of the stream's order, from the perspective.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct StreamState {
    /// The tick of a snapshot whose events have not come yet.
    pending_tick: Option<u32>,
    without_snapshot: usize,
    /// The snapshot tick not yet handed out for acknowledgement.
    unacked: Option<u32>,
}

/// The state of one client's match.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientSession {
    statics: Arc<MatchStatic>,
    options: SessionOptions,
    owner: Option<Owner>,
    /// The hero the seat must play, when it must play one.
    hero: Option<HeroId>,
    current: Option<WorldView>,
    previous: Option<WorldView>,
    tracks: Tracks,
    memory: BodyMemory,
    missiles: Missiles,
    recent_events: VecDeque<ObservedEvent>,
    last_event_tick: Option<u32>,
    skipped_ticks: u32,
    ledger: OrderLedger,
    waits: SharedWaits,
    next_seq: u32,
    stream: StreamState,
}

impl ClientSession {
    /// The session of one seat's own stream, held to the server's rules for
    /// it: the viewer is the seat's side, a snapshot is followed by its tick's
    /// events before the next, and the match does not end between the two.
    /// With a `hero`, the seat must play it.
    pub fn seat(
        statics: Arc<MatchStatic>,
        slot: SlotId,
        hero: Option<HeroId>,
        options: SessionOptions,
    ) -> Result<Self, SessionError> {
        let owner = Owner {
            slot,
            team: seat_team(&statics, slot, hero)?,
        };
        Ok(Self::folding(statics, Some(owner), hero, options))
    }

    /// The session of a stream any viewer may watch, who may change over it
    /// (a spectator's eyes, a replay): a new snapshot completes the tick
    /// before it, since a replay keeps events only when there are some.
    pub fn spectator(statics: Arc<MatchStatic>, options: SessionOptions) -> Self {
        Self::folding(statics, None, None, options)
    }

    /// The session of the seat a welcome assigned, after checking the match
    /// start against the welcome and the seat's pick.
    pub fn start(
        seated: &Seated,
        info: &MatchInfo,
        options: SessionOptions,
    ) -> Result<Self, SessionError> {
        check_match(seated, info)?;
        Self::seat(MatchStatic::new(info)?, seated.slot, seated.hero, options)
    }

    fn folding(
        statics: Arc<MatchStatic>,
        owner: Option<Owner>,
        hero: Option<HeroId>,
        options: SessionOptions,
    ) -> Self {
        Self {
            statics,
            options,
            owner,
            hero,
            current: None,
            previous: None,
            tracks: Tracks::default(),
            memory: BodyMemory::default(),
            missiles: Missiles::default(),
            recent_events: VecDeque::with_capacity(MAX_RECENT_EVENTS),
            last_event_tick: None,
            skipped_ticks: 0,
            ledger: OrderLedger::default(),
            waits: SharedWaits::new(),
            next_seq: 0,
            stream: StreamState::default(),
        }
    }

    /// Folds one server message in.
    pub fn ingest<O: Observer>(
        &mut self,
        msg: ServerMsg,
        obs: &mut O,
    ) -> Result<Ingested, IngestError<O::Error>> {
        self.count_message(&msg)?;
        match msg {
            ServerMsg::Snapshot { view } => self.ingest_snapshot(view, obs),
            ServerMsg::Events { tick, events } => self.ingest_events(tick, &events, obs),
            ServerMsg::OrderRejected { seq, reason } => Ok(self.ingest_rejection(seq, reason)),
            ServerMsg::MatchOver { winner, stats } => {
                if self.strict() && self.stream.pending_tick.is_some() {
                    return Err(SessionError::Stream(
                        "server ended the match before completing the snapshot tick",
                    )
                    .into());
                }
                Ok(Ingested::Over { winner, stats })
            }
            ServerMsg::MatchStart { .. } => {
                Err(SessionError::Stream("server sent repeated MatchStart").into())
            }
            ServerMsg::WireMismatch { server_fingerprint } => Err(SessionError::WireMismatch {
                server: server_fingerprint,
            }
            .into()),
            other @ (ServerMsg::Welcome { .. }
            | ServerMsg::LobbyState { .. }
            | ServerMsg::Orders { .. }
            | ServerMsg::ParticipantLeft { .. }) => Ok(Ingested::Passed(Box::new(other))),
        }
    }

    /// Folds one snapshot in, moving it into the session.
    ///
    /// The fold runs in this order, which the observer's hooks sit in: the
    /// stream check, validation, `prepare`, the position check,
    /// `before_snapshot`, the events of each skipped tick (`on_events`, then
    /// folded), the entities, the remembered bodies, projectiles and loot,
    /// the body-change check on the request ledger, the swap of current and
    /// previous view, and `after_snapshot`.
    pub fn ingest_snapshot<O: Observer>(
        &mut self,
        mut view: WorldView,
        obs: &mut O,
    ) -> Result<Ingested, IngestError<O::Error>> {
        if self.strict() && self.stream.pending_tick.is_some() {
            return Err(SessionError::Stream(
                "server sent Snapshot before completing the previous tick",
            )
            .into());
        }
        validate::snapshot(&self.expectations(), &view)?;
        obs.prepare(&mut view).map_err(IngestError::Observer)?;
        validate::position_deltas(self.tracks.all(), &view)?;
        obs.before_snapshot(self, &view)
            .map_err(IngestError::Observer)?;
        for skipped in &view.skipped_ticks {
            obs.on_events(self, skipped.tick, &skipped.events, EventSource::Skipped)
                .map_err(IngestError::Observer)?;
            self.record_events(skipped.tick, &skipped.events);
        }
        let hero_before = self.own_hero_id();
        self.fold_snapshot(&view);
        let tick = view.tick;
        self.skipped_ticks = self
            .skipped_ticks
            .saturating_add(u32::try_from(view.skipped_ticks.len()).unwrap_or(u32::MAX));
        self.previous = self.current.replace(view);
        let body_changed = self.owner.is_some() && hero_before != self.own_hero_id();
        if body_changed {
            self.ledger.clear(None);
        }
        let ack = self.settle_stream(tick);
        obs.after_snapshot(self).map_err(IngestError::Observer)?;
        Ok(Ingested::Snapshot {
            tick,
            body_changed,
            ack,
        })
    }

    /// Folds one tick's events in.
    pub fn ingest_events<O: Observer>(
        &mut self,
        tick: u32,
        events: &[EventKind],
        obs: &mut O,
    ) -> Result<Ingested, IngestError<O::Error>> {
        validate::events(tick, events, self.last_event_tick)?;
        if self.strict() && self.stream.pending_tick != Some(tick) {
            return Err(SessionError::Stream(
                "server Events did not complete the pending snapshot tick",
            )
            .into());
        }
        obs.on_events(self, tick, events, EventSource::Live)
            .map_err(IngestError::Observer)?;
        self.record_events(tick, events);
        self.stream.pending_tick = None;
        Ok(Ingested::TickComplete { tick })
    }

    /// Rolls back an order the server refused.
    pub fn ingest_rejection(&mut self, seq: u32, reason: RejectReason) -> Ingested {
        self.ledger.reject(seq);
        self.waits.reject(seq);
        Ingested::Rejected { seq, reason }
    }

    /// Numbers an order and records it as given on the current snapshot. The
    /// caller transmits the result.
    pub fn issue(&mut self, order: IssuedOrder) -> Result<OutgoingOrder, SessionError> {
        let seen = self.current.as_ref().ok_or(SessionError::NoSnapshot)?.tick;
        let seq = self
            .next_seq
            .checked_add(1)
            .ok_or(SessionError::SequenceExhausted)?;
        let ends_body = crate::ends_body(self, order);
        self.ledger.record(seq, order, ends_body)?;
        if let Some(body) = wait_started_by(self, order) {
            self.waits.start(body, seq, seen);
        }
        self.next_seq = seq;
        Ok(OutgoingOrder {
            seq,
            unit: order.unit,
            order: order.order,
            seen,
        })
    }

    /// Under [`AckPolicy::AfterTurn`], the tick to acknowledge now that the
    /// client has decided on it; nothing the second time.
    pub fn end_turn(&mut self) -> Option<u32> {
        self.stream.unacked.take()
    }

    #[inline]
    fn strict(&self) -> bool {
        self.owner.is_some()
    }

    fn expectations(&self) -> validate::Expect<'_> {
        validate::Expect {
            statics: &self.statics,
            owner: self.owner,
            hero: self.hero,
            current_tick: self.current.as_ref().map(|view| view.tick),
            last_event_tick: self.last_event_tick,
        }
    }

    /// Counts a message against the bound on a stream without snapshots.
    fn count_message(&mut self, msg: &ServerMsg) -> Result<(), SessionError> {
        if !self.strict() {
            return Ok(());
        }
        if matches!(msg, ServerMsg::Snapshot { .. }) {
            self.stream.without_snapshot = 0;
            return Ok(());
        }
        self.stream.without_snapshot = self.stream.without_snapshot.saturating_add(1);
        if self.stream.without_snapshot > MAX_MESSAGES_WITHOUT_SNAPSHOT {
            return Err(SessionError::Stream(
                "server sent too many messages without a snapshot",
            ));
        }
        Ok(())
    }

    fn fold_snapshot(&mut self, view: &WorldView) {
        let viewer = self.owner.map_or(view.viewer, |owner| Some(owner.team));
        self.tracks
            .fold_snapshot(view, self.owner, viewer, self.options.forget_after_ticks);
        self.memory.fold(view);
        self.missiles.fold(view.tick, &view.projectiles, &view.loot);
    }

    /// Notes the folded snapshot in the stream's state and answers the
    /// acknowledgement due now.
    fn settle_stream(&mut self, tick: u32) -> Option<u32> {
        self.stream.pending_tick = self.strict().then_some(tick);
        let lockstep = self.statics.metadata().mode == TickMode::Lockstep;
        if self.owner.is_none() || !lockstep {
            return None;
        }
        match self.options.ack {
            AckPolicy::AfterTurn => {
                self.stream.unacked = Some(tick);
                None
            }
            AckPolicy::OnSnapshot => Some(tick),
            AckPolicy::Never => None,
        }
    }

    /// Folds one tick's events in, its own or a skipped one's.
    fn record_events(&mut self, tick: u32, events: &[EventKind]) {
        assert!(self.last_event_tick.is_none_or(|previous| previous < tick));
        self.tracks.fold_events(tick, events);
        for event in events {
            if self.recent_events.len() == MAX_RECENT_EVENTS {
                self.recent_events.pop_front();
            }
            self.recent_events.push_back(ObservedEvent {
                tick,
                kind: event.clone(),
            });
        }
        self.last_event_tick = Some(tick);
    }

    #[inline]
    fn own_hero_id(&self) -> Option<EntityId> {
        self.own_player()?.unit
    }
}

/// What the session reads out.
impl ClientSession {
    /// The match's immutable inputs.
    #[inline]
    pub fn statics(&self) -> &Arc<MatchStatic> {
        &self.statics
    }

    pub const fn options(&self) -> &SessionOptions {
        &self.options
    }

    /// The seat the stream is for, if it is a seat's.
    #[inline]
    pub fn slot(&self) -> Option<SlotId> {
        self.owner.map(|owner| owner.slot)
    }

    /// The side the stream is seen from: a seat's own, or the side whose eyes
    /// a spectator watches through; [`Team::Neutral`] for one who sees all.
    #[inline]
    pub fn team(&self) -> Team {
        self.viewer().unwrap_or(Team::Neutral)
    }

    /// The side across the map from [`Self::team`].
    #[inline]
    pub fn enemy(&self) -> Team {
        self.team().enemy()
    }

    /// The map the match is played on.
    #[inline]
    pub fn map(&self) -> &'static MapDef {
        map_of(self.statics.metadata().map)
    }

    /// The side whose sight the stream is told; none for one who sees all.
    fn viewer(&self) -> Option<Team> {
        match self.owner {
            Some(owner) => Some(owner.team),
            None => self.current.as_ref()?.viewer,
        }
    }

    /// The latest folded snapshot.
    #[inline]
    pub fn current(&self) -> Option<&WorldView> {
        self.current.as_ref()
    }

    /// The snapshot before the latest.
    #[inline]
    pub fn previous(&self) -> Option<&WorldView> {
        self.previous.as_ref()
    }

    /// Own scoreboard row from the current snapshot.
    #[inline]
    pub fn own_player(&self) -> Option<&PlayerView> {
        let slot = self.slot()?;
        self.current
            .as_ref()?
            .players
            .iter()
            .find(|player| player.slot == slot)
    }

    /// Own live hero body from the current snapshot.
    #[inline]
    pub fn own_hero(&self) -> Option<&UnitView> {
        let id = self.own_player()?.unit?;
        let current = self.current.as_ref()?;
        let index = current
            .units
            .binary_search_by_key(&id, |unit| unit.id)
            .ok()?;
        Some(&current.units[index])
    }

    /// Own currently visible courier.
    pub fn own_courier(&self) -> Option<&UnitView> {
        let owner = self.owner?;
        self.current
            .as_ref()?
            .units
            .iter()
            .find(|unit| owner.owns_courier(unit))
    }

    /// The learned talents last told of a tracked hero; empty for an untracked entity.
    #[inline]
    pub fn talents_of(&self, id: EntityId) -> Talents {
        self.tracks
            .get(id)
            .map_or(Talents::default(), |track| track.talents)
    }

    /// One remembered entity by its full opaque handle.
    #[inline]
    pub fn entity(&self, id: EntityId) -> Option<&EntityTrack> {
        self.tracks.get(id)
    }

    /// All remembered entities, sorted by full opaque handle.
    #[inline]
    pub fn entities(&self) -> &[EntityTrack] {
        self.tracks.all()
    }

    /// What is known of a unit and how many ticks ago that was true: it in
    /// view, else the entity's track, else the body its seat last had.
    #[inline]
    pub fn known(&self, id: EntityId) -> Option<(&UnitView, u32)> {
        let view = self.current.as_ref()?;
        if let Ok(index) = view.units.binary_search_by_key(&id, |unit| unit.id) {
            return Some((&view.units[index], 0));
        }
        if let Some(track) = self.tracks.get(id) {
            return Some((&track.unit, view.tick.saturating_sub(track.last_seen_tick)));
        }
        let held = self.memory.get(id)?;
        Some((&held.unit, view.tick.saturating_sub(held.tick)))
    }

    /// The body a seat last stood in, whether or not it still stands.
    pub fn body_of(&self, slot: SlotId) -> Option<EntityId> {
        let view = self.current.as_ref()?;
        view.players
            .iter()
            .find(|player| player.slot == slot)
            .and_then(|player| player.unit)
            .or_else(|| self.memory.hero_of(slot))
    }

    /// Recent visible events in stream order.
    #[inline]
    pub fn recent_events(&self) -> &VecDeque<ObservedEvent> {
        &self.recent_events
    }

    /// Live projectiles, sorted by handle.
    #[inline]
    pub fn missiles(&self) -> &[MissileTrack] {
        self.missiles.missiles()
    }

    /// Items on the ground, sorted by handle.
    #[inline]
    pub fn loot(&self) -> &[LootTrack] {
        self.missiles.loot()
    }

    /// Ticks the latest applied order of the seat took effect past the tick
    /// after the view it was given on, as the current snapshot tells.
    pub fn order_delay(&self) -> Option<u32> {
        let slot = self.slot()?;
        self.current
            .as_ref()?
            .order_delays
            .iter()
            .find(|delay| delay.slot == slot)
            .map(|delay| delay.ticks)
    }

    /// The same delay in milliseconds: the server's measure of the round trip.
    pub fn order_delay_ms(&self) -> Option<u32> {
        let ticks = self.order_delay()?;
        Some(ticks * 1000 / u32::from(self.statics.metadata().tick_rate.max(1)))
    }

    /// The tick of the latest events folded in, a skipped tick's included.
    pub const fn last_event_tick(&self) -> Option<u32> {
        self.last_event_tick
    }

    /// Ticks the server skipped for this client so far, too slow for every one.
    pub const fn skipped_ticks(&self) -> u32 {
        self.skipped_ticks
    }

    /// The orders standing after the server's answers.
    pub const fn ledger(&self) -> &OrderLedger {
        &self.ledger
    }

    /// The waits the orders given started.
    pub const fn waits(&self) -> &SharedWaits {
        &self.waits
    }

    /// Whether a live unit of the stream's own side stands in the target's
    /// terrain cell or one next to it: the conservative proof that a change
    /// of trees there is one the seat could see.
    pub fn observes_locally(&self, target: bota_proto::Vec2) -> bool {
        let (Some(view), Some(team)) = (self.current.as_ref(), self.viewer()) else {
            return false;
        };
        let Some(target_cell) = self.statics.cell_of(target) else {
            return false;
        };
        view.units.iter().any(|unit| {
            unit.team == team
                && unit.hp > 0
                && self.statics.cell_of(unit.pos).is_some_and(|cell| {
                    cell.0.abs_diff(target_cell.0) <= 1 && cell.1.abs_diff(target_cell.1) <= 1
                })
        })
    }
}
