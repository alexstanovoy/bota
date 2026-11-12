//! One in-process lockstep match and what each seat is told of it.
//!
//! A [`Game`] hands every seat exactly the stream its connection to a server would carry: its
//! side's view and events, the answers to its own orders, `MatchOver`. Nothing hidden leaves
//! through it. The whole truth (the world itself, the whole map's view, forks of the match)
//! is reachable only from a `Game<Privileged>`, so code that holds a `Game<Player>` cannot read
//! what a real client could not:
//!
//! ```compile_fail
//! # fn read(game: &bota_gym::Game) {
//! let _ = game.truth(); // only `Game<Privileged>` has the truth
//! # }
//! ```
//!
//! ```
//! # fn read(game: &bota_gym::Game<bota_gym::Privileged>) {
//! let _ = game.truth().full_view(); // the whole map, fog or not
//! # }
//! ```
//!
//! Use `Privileged` for training signals that may see everything (a critic's inputs, a
//! reward), harness measurements and forks; keep what a policy reads on the player streams.

use core::marker::PhantomData;

use bota_proto::{MatchStats, OutgoingOrder, ReplayRecord, ServerMsg, Team, TickMode, WorldView};
use bota_replay::meta::{Meta, Recorder};
use bota_server::game::{Entity, MatchConfig, World};
use bota_server::local::{LocalError, LocalMatch};

use crate::GymError;
use crate::spec::{MAX_SETUP_CHEATS, MAX_TICK_LIMIT, MatchSpec};

/// Upper bound on the ticks of one [`Game::advance`].
pub const MAX_ADVANCE_TICKS: u32 = 1 << 16;
/// Upper bound on the seats of a match.
pub const MAX_SEATS: usize = bota_proto::limits::MAX_SEATS;

mod sealed {
    pub trait Sealed {}
}

/// What a [`Game`]'s holder may know: [`Player`] or [`Privileged`].
pub trait Access: sealed::Sealed + Send + Sync + 'static {}

/// Only what the seats' connections carry: what a real client could know.
pub enum Player {}
/// The whole truth besides: the world, the whole map's view, forks of the match.
pub enum Privileged {}

impl sealed::Sealed for Player {}
impl sealed::Sealed for Privileged {}
impl Access for Player {}
impl Access for Privileged {}

/// How one [`Game::advance`] went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Advance {
    /// Ticks it stepped.
    pub ticks: u32,
    /// The winner once the match ended; [`Team::Neutral`] is a draw.
    pub winner: Option<Team>,
    /// The match reached its tick limit without ending: a cut, not an outcome.
    pub limited: bool,
}

impl Advance {
    /// Whether the game is over, ended or cut.
    pub const fn done(&self) -> bool {
        self.winner.is_some() || self.limited
    }
}

/// One in-process lockstep match.
pub struct Game<A: Access = Player> {
    local: LocalMatch,
    seats: usize,
    limit: u32,
    over: bool,
    recorder: Option<Recorder>,
    access: PhantomData<A>,
}

/// A fork knows the future of the match: only a privileged game may be forked.
impl Clone for Game<Privileged> {
    fn clone(&self) -> Self {
        Self {
            local: self.local.clone(),
            seats: self.seats,
            limit: self.limit,
            over: self.over,
            recorder: self.recorder.clone(),
            access: PhantomData,
        }
    }
}

/// Sequence numbers of setup cheats, far above any a client's session reaches.
const SETUP_SEQ: u32 = u32::MAX - MAX_SETUP_CHEATS as u32;

impl<A: Access> Game<A> {
    /// Builds the match and advances it to its first visible tick, then through its setup
    /// cheats; returns each seat's stream so far: `MatchStart`, the first `Snapshot` and
    /// `Events`, then every setup tick's stream as the seat's connection would carry it.
    pub fn new(spec: &MatchSpec) -> Result<(Self, Vec<Vec<ServerMsg>>), GymError> {
        validate(spec)?;
        let config = match_config(spec);
        let (local, mut opening) = if spec.record_meta {
            LocalMatch::new_recorded(&config, &spec.order_delays)
        } else {
            LocalMatch::new(&config, &spec.order_delays)
        }
        .map_err(local_error)?;
        let mut recorder = spec.record_meta.then(Recorder::new);
        if let Some(recorder) = &mut recorder {
            recorder.push(&ReplayRecord::Msg(opening[0][0].clone()));
        }
        let mut game = Self {
            seats: spec.picks.len(),
            local,
            limit: spec.tick_limit,
            over: false,
            recorder,
            access: PhantomData,
        };
        game.record();
        for (seat, stream) in opening.iter().enumerate() {
            if !matches!(stream.first(), Some(ServerMsg::MatchStart { .. })) {
                return Err(GymError::Stream {
                    seat,
                    problem: "the opening does not begin with MatchStart",
                });
            }
        }
        let tail: Vec<&[ServerMsg]> = opening.iter().map(|stream| &stream[1..]).collect();
        if validate_tick(&tail)?.is_some() {
            return Err(GymError::SetupEnded);
        }
        if !spec.setup.is_empty() {
            game.set_up(spec, &mut opening)?;
        }
        Ok((game, opening))
    }

    /// Gives every setup cheat at once, steps until each is due and appends those ticks to
    /// the seats' streams.
    fn set_up(&mut self, spec: &MatchSpec, opening: &mut [Vec<ServerMsg>]) -> Result<(), GymError> {
        let seen = self.local.tick();
        let mut given: Vec<Vec<OutgoingOrder>> = vec![Vec::new(); self.seats];
        for (index, setup) in spec.setup.iter().enumerate() {
            given[setup.seat].push(OutgoingOrder {
                seq: SETUP_SEQ + index as u32,
                unit: None,
                order: bota_proto::Order::Cheat { cheat: setup.cheat },
                seen,
                queue: false,
            });
        }
        let empty: Vec<Vec<OutgoingOrder>> = vec![Vec::new(); self.seats];
        for tick in 0..=spec.longest_delay() {
            let orders = if tick == 0 { &given } else { &empty };
            let slices: Vec<&[OutgoingOrder]> = orders.iter().map(Vec::as_slice).collect();
            let streams = self.local.step(&slices).map_err(local_error)?;
            self.record();
            let views: Vec<&[ServerMsg]> = streams.iter().map(Vec::as_slice).collect();
            if validate_tick(&views)?.is_some() {
                return Err(GymError::SetupEnded);
            }
            for (seat, (stream, told)) in opening.iter_mut().zip(streams).enumerate() {
                for message in told {
                    if let ServerMsg::OrderRejected { seq, reason } = message
                        && seq >= SETUP_SEQ
                    {
                        return Err(GymError::SetupRejected {
                            seat,
                            index: (seq - SETUP_SEQ) as usize,
                            reason,
                        });
                    }
                    stream.push(message);
                }
            }
        }
        Ok(())
    }

    /// Gives each seat at most one order and advances one tick; returns each seat's stream of
    /// the tick: rejections of its orders, `Snapshot`, `Events`, and `MatchOver` once the
    /// match ended (a [`Team::Neutral`] winner is a draw).
    pub fn step(
        &mut self,
        orders: &[Option<OutgoingOrder>],
    ) -> Result<Vec<Vec<ServerMsg>>, GymError> {
        self.step_told(orders).map(|(streams, _)| streams)
    }

    fn step_told(
        &mut self,
        orders: &[Option<OutgoingOrder>],
    ) -> Result<(Vec<Vec<ServerMsg>>, Option<Team>), GymError> {
        if self.over {
            return Err(GymError::MatchOver);
        }
        if orders.len() != self.seats {
            return Err(GymError::OrderCount {
                expected: self.seats,
                got: orders.len(),
            });
        }
        let tick = self.local.tick();
        if tick >= self.limit {
            return Err(GymError::TickLimit {
                tick,
                limit: self.limit,
            });
        }
        let mut given: [&[OutgoingOrder]; MAX_SEATS] = [&[]; MAX_SEATS];
        for (slot, order) in given.iter_mut().zip(orders) {
            *slot = order.as_slice();
        }
        let streams = self.local.step(&given[..self.seats]).map_err(local_error)?;
        self.record();
        let views: [&[ServerMsg]; MAX_SEATS] =
            core::array::from_fn(|seat| streams.get(seat).map_or(&[][..], Vec::as_slice));
        let winner = validate_tick(&views[..self.seats])?;
        self.over = winner.is_some();
        Ok((streams, winner))
    }

    /// Steps up to `ticks` ticks, `orders` given on the first, handing each tick's streams to
    /// `each_tick`; stops early when the match ends, and at the tick limit.
    pub fn advance<E: From<GymError>>(
        &mut self,
        orders: &[Option<OutgoingOrder>],
        ticks: u32,
        mut each_tick: impl FnMut(Vec<Vec<ServerMsg>>) -> Result<(), E>,
    ) -> Result<Advance, E> {
        if !(1..=MAX_ADVANCE_TICKS).contains(&ticks) {
            return Err(GymError::Config("advance ticks must be 1..=MAX_ADVANCE_TICKS").into());
        }
        let start = self.local.tick();
        let ticks = ticks.min(self.limit.saturating_sub(start)).max(1);
        let none = [None; MAX_SEATS];
        let mut advance = Advance {
            ticks: 0,
            winner: None,
            limited: false,
        };
        while advance.ticks < ticks {
            let given = if advance.ticks == 0 {
                orders
            } else {
                &none[..self.seats]
            };
            let (streams, winner) = self.step_told(given)?;
            advance.ticks += 1;
            advance.winner = winner;
            each_tick(streams)?;
            if winner.is_some() {
                return Ok(advance);
            }
        }
        advance.limited = self.local.tick() >= self.limit;
        Ok(advance)
    }

    pub fn tick(&self) -> u32 {
        self.local.tick()
    }

    pub fn seat_count(&self) -> usize {
        self.seats
    }

    pub const fn tick_limit(&self) -> u32 {
        self.limit
    }

    /// Whether the match sent `MatchOver`.
    pub const fn is_over(&self) -> bool {
        self.over
    }

    /// The scoreboard now: every seat's kills, deaths, assists, last hits, denies, gold
    /// earned and damage to heroes and buildings (what Dota's scoreboard shows every player).
    pub fn stats(&self) -> MatchStats {
        self.local.world().match_stats()
    }

    /// The match metadata recorded so far, when the spec asked for it: per-minute samples of
    /// every seat (gold, net worth with items, experience, level, kills, last hits, healing…),
    /// kills, purchases, level-ups and fallen buildings. Ends the recording.
    pub fn finish(&mut self) -> Result<Option<Meta>, GymError> {
        let Some(recorder) = self.recorder.take() else {
            return Ok(None);
        };
        bota_replay::meta::decode(&recorder.finish())
            .map(Some)
            .map_err(|error| GymError::Meta(format!("{error:?}")))
    }

    fn record(&mut self) {
        if let Some(recorder) = &mut self.recorder {
            for record in self.local.take_records() {
                recorder.push(&record);
            }
        }
    }
}

impl Game<Privileged> {
    /// The whole truth of the match.
    pub fn truth(&self) -> Truth<'_> {
        Truth { local: &self.local }
    }

    /// Rearranges the world directly, for a scenario no cheat sets up; returns what every seat
    /// is then told: a fresh snapshot of the current tick, with no events.
    pub fn arrange(&mut self, arrange: impl FnOnce(&mut World)) -> Vec<Vec<ServerMsg>> {
        arrange(self.local.world_mut());
        self.local.restate()
    }
}

/// Everything about a match, hidden or not: for training signals and measurements only.
#[derive(Clone, Copy)]
pub struct Truth<'a> {
    local: &'a LocalMatch,
}

impl<'a> Truth<'a> {
    /// The simulated world itself.
    pub fn world(self) -> &'a World {
        self.local.world()
    }

    /// The hero a seat plays, while it stands.
    pub fn hero_of(self, seat: usize) -> Option<Entity> {
        self.local.hero_of(seat)
    }

    /// The handle a seat's client knows a world entity by, once the seat was told of it.
    pub fn handle_of(self, seat: usize, entity: Entity) -> Option<bota_proto::EntityId> {
        self.local.handle_of(seat, entity)
    }

    /// The whole map as a spectator of everything sees it.
    pub fn full_view(self) -> WorldView {
        self.local.world().view_full()
    }
}

fn validate(spec: &MatchSpec) -> Result<(), GymError> {
    if spec.picks.is_empty() || spec.picks.len() > MAX_SEATS {
        return Err(GymError::Config("a match needs 1..=MAX_SEATS seats"));
    }
    if !(1..=MAX_TICK_LIMIT).contains(&spec.tick_limit) {
        return Err(GymError::Config("tick_limit must be 1..=MAX_TICK_LIMIT"));
    }
    if spec.setup.len() > MAX_SETUP_CHEATS {
        return Err(GymError::Config("at most MAX_SETUP_CHEATS setup cheats"));
    }
    if spec
        .setup
        .iter()
        .any(|setup| setup.seat >= spec.picks.len())
    {
        return Err(GymError::Config(
            "a setup cheat names a seat the match lacks",
        ));
    }
    for seat in 0..spec.picks.len() {
        let given = spec.setup.iter().filter(|setup| setup.seat == seat).count();
        if given > bota_proto::limits::MAX_ORDERS_PER_TICK {
            return Err(GymError::Config(
                "a seat's setup cheats exceed MAX_ORDERS_PER_TICK",
            ));
        }
    }
    if !bota_core::MAPS.iter().any(|map| map.id == spec.map) {
        return Err(GymError::Config("the map is not one bota plays"));
    }
    Ok(())
}

fn match_config(spec: &MatchSpec) -> MatchConfig {
    let bytes = spec.seed.to_le_bytes();
    MatchConfig {
        match_id: spec.seed,
        master_key: core::array::from_fn(|index| bytes[index % bytes.len()]),
        stream: spec.seed,
        picks: spec.picks.clone(),
        map: spec.map,
        mode: TickMode::Lockstep,
        // Lockstep in-process: nobody acknowledges, and nothing waits on the clock.
        ack_timeout_ticks: 150,
        cheats: !spec.setup.is_empty(),
        spawn_modifiers: spec.spawn_modifiers.clone(),
        immortal_structures: spec.immortal_structures,
        immortal_heroes: spec.immortal_heroes,
        starting_gold_lost: Vec::new(),
        objectives: spec.objectives,
        runes: spec.runes,
        catalogue: spec.catalogue,
        economy_events: spec.economy_events.clone(),
    }
}

fn local_error(error: LocalError) -> GymError {
    match error {
        LocalError::Config { message } => GymError::Match(message),
        LocalError::OrderCount { expected, got } => GymError::OrderCount { expected, got },
        LocalError::MatchOver => GymError::MatchOver,
    }
}

/// Checks one tick's streams against what a client relies on and returns the winner once the
/// match ended: per seat, its rejections, then `Snapshot` and `Events` of one tick (absent only
/// on the tick that ends the match without advancing it), then `MatchOver` or nothing; every
/// seat told the same ending.
fn validate_tick(streams: &[&[ServerMsg]]) -> Result<Option<Team>, GymError> {
    let mut ending: Option<Option<Team>> = None;
    for (seat, messages) in streams.iter().enumerate() {
        let problem = |problem| GymError::Stream { seat, problem };
        let answered = messages
            .iter()
            .take_while(|message| matches!(message, ServerMsg::OrderRejected { .. }))
            .count();
        let (snapshot, rest) = match &messages[answered..] {
            [
                ServerMsg::Snapshot { view },
                ServerMsg::Events { tick, .. },
                rest @ ..,
            ] if view.tick == *tick => (true, rest),
            rest => (false, rest),
        };
        let winner = match rest {
            [] if snapshot => None,
            [ServerMsg::MatchOver { winner, .. }] => Some(*winner),
            _ => {
                return Err(problem(
                    "not rejections, Snapshot, Events of one tick, MatchOver",
                ));
            }
        };
        match ending {
            None => ending = Some(winner),
            Some(told) if told != winner => {
                return Err(problem("seats were told different endings"));
            }
            Some(_) => {}
        }
    }
    Ok(ending.flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_of(game: &mut Game<Privileged>) -> Vec<ServerMsg> {
        game.step(&[None, None]).expect("a tick").remove(0)
    }

    #[test]
    fn a_stream_out_of_its_contract_is_refused() {
        let picks = vec![
            bota_proto::Pick {
                slot: bota_proto::SlotId(0),
                team: Team::Radiant,
                hero: bota_proto::HeroId::SHADOW_FIEND,
            },
            bota_proto::Pick {
                slot: bota_proto::SlotId(1),
                team: Team::Dire,
                hero: bota_proto::HeroId::PUDGE,
            },
        ];
        let spec = MatchSpec::new(bota_core::MAP2_ID, picks, 5, 1_000);
        let (mut game, _) = Game::<Privileged>::new(&spec).expect("a game");
        let good = tick_of(&mut game);
        assert_eq!(validate_tick(&[&good, &good]), Ok(None));
        let over = ServerMsg::MatchOver {
            winner: Team::Dire,
            stats: game.stats(),
        };
        let mut ended = good.clone();
        ended.push(over.clone());
        assert_eq!(validate_tick(&[&ended, &ended]), Ok(Some(Team::Dire)));
        let refused = |problem| Err(GymError::Stream { seat: 1, problem });
        assert_eq!(
            validate_tick(&[&ended, &good]),
            refused("seats were told different endings")
        );
        let contract = "not rejections, Snapshot, Events of one tick, MatchOver";
        let mut reordered = ended.clone();
        reordered.swap(1, 2);
        assert_eq!(validate_tick(&[&ended, &reordered]), refused(contract));
        assert_eq!(validate_tick(&[&good, &good[..1]]), refused(contract));
        assert_eq!(validate_tick(&[&good, &[]]), refused(contract));
        // The tick that ends a match without advancing it tells only the ending.
        assert_eq!(
            validate_tick(&[std::slice::from_ref(&over); 2]),
            Ok(Some(Team::Dire))
        );
    }
}
