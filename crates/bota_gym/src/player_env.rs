//! A ready-made [`Episode`]: every seat's client session folded on the workers.

use bota_client_core::{AckPolicy, ClientSession, IssuedOrder, MatchStatic, SessionOptions};
use bota_proto::{MatchStats, OutgoingOrder, ServerMsg};
use bota_replay::meta::Meta;

use crate::game::{Access, Advance, MAX_ORDERS_PER_TICK, MAX_SEATS, Player, Privileged, Truth};
use crate::{Episode, Game, GameId, GymError, MatchSpec};

/// One order per seat, or none: what a [`PlayerEnv`] step takes by default.
pub type SeatOrders = [Option<IssuedOrder>; MAX_SEATS];

/// What one [`PlayerEnv`] step hands its seats: each seat's orders for the turn, in the order
/// its client issues them.
pub trait SeatActions: Send {
    /// Most orders one seat issues in one turn; at most [`MAX_ORDERS_PER_TICK`].
    const PER_SEAT: usize;

    /// Seat `seat`'s orders; `None` entries are skipped.
    fn seat(&self, seat: usize) -> &[Option<IssuedOrder>];
}

impl SeatActions for SeatOrders {
    const PER_SEAT: usize = 1;

    fn seat(&self, seat: usize) -> &[Option<IssuedOrder>] {
        core::slice::from_ref(&self[seat])
    }
}

/// Up to `N` orders per seat for one turn, in the order given: clients may issue several orders
/// per turn (stop, learn an ability, buy, then attack). `N` is at most [`MAX_ORDERS_PER_TICK`],
/// what the server lets take effect for one seat on one tick; the storage is fixed, nothing is
/// allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatBatches<const N: usize> {
    orders: [[Option<IssuedOrder>; N]; MAX_SEATS],
}

impl<const N: usize> SeatBatches<N> {
    const FITS: () = assert!(N >= 1 && N <= MAX_ORDERS_PER_TICK);

    /// No orders for any seat.
    pub const fn new() -> Self {
        let () = Self::FITS;
        Self {
            orders: [[None; N]; MAX_SEATS],
        }
    }

    /// Adds `order` after seat `seat`'s others; refuses a seat beyond [`MAX_SEATS`] or a
    /// full batch.
    pub fn push(&mut self, seat: usize, order: IssuedOrder) -> Result<(), GymError> {
        let batch = self
            .orders
            .get_mut(seat)
            .ok_or(GymError::Config("a seat must be below MAX_SEATS"))?;
        let free = batch
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(GymError::OrderBatch { seat, len: N + 1 })?;
        *free = Some(order);
        Ok(())
    }
}

impl<const N: usize> Default for SeatBatches<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> SeatActions for SeatBatches<N> {
    const PER_SEAT: usize = N;

    fn seat(&self, seat: usize) -> &[Option<IssuedOrder>] {
        &self.orders[seat]
    }
}

/// How a [`PlayerEnv`] run makes the match of a game from its id and plan.
pub type SpecFn<P> = dyn Fn(GameId, &P) -> MatchSpec + Send + Sync;

/// What every [`PlayerEnv`] of a run shares.
pub struct PlayerEnvConfig<P> {
    /// Ticks between decisions, `1..=MAX_ADVANCE_TICKS` (bota runs 30 ticks a second).
    pub decision_ticks: u32,
    /// The match of each game, from its id and plan; derive its seed from the id
    /// ([`GameId::seed`]) for a run that replays.
    pub spec: Box<SpecFn<P>>,
}

/// How one game of a [`PlayerEnv`] went.
#[derive(Clone, Debug)]
pub struct Finished<P> {
    pub game: GameId,
    pub plan: P,
    /// How the last step ended: the winner, or the cut at the tick limit.
    pub end: Advance,
    /// The tick the game ended on.
    pub tick: u32,
    /// The final statistics every seat was told in `MatchOver`; none for a game cut at its tick
    /// limit, which tells no seat.
    pub result: Option<MatchStats>,
    /// The match metadata, when the spec recorded it: a `PlayerEnv<_, Privileged>` only.
    pub meta: Option<Meta>,
}

/// One game seen through its seats' own clients: each seat's [`ClientSession`] folds exactly the
/// stream a connection to a server would carry, on the worker, and the caller's policy reads
/// those sessions between steps and answers with the seats' orders: one per seat with
/// [`SeatOrders`] (the default), or up to `N` per seat with `PlayerEnv<P, A, SeatBatches<N>>`.
///
/// The caller plays every seat, so it holds every seat's session: a policy acting for one seat
/// should read that seat's session alone, as its client would. `PlayerEnv<P, Privileged>` also
/// offers [`PlayerEnv::truth`] for training signals that may see everything;
/// `PlayerEnv<P>` (= `PlayerEnv<P, Player>`) has no way to it, and its finished games carry only
/// what every seat was told.
pub struct PlayerEnv<P, A: Access = Player, Act: SeatActions = SeatOrders> {
    id: GameId,
    plan: P,
    game: Game<A>,
    sessions: Vec<ClientSession>,
    /// The turn's orders as sent, sized once for every seat's most orders.
    outgoing: Vec<OutgoingOrder>,
    actions: core::marker::PhantomData<fn(Act)>,
}

impl<P, A: Access, Act: SeatActions> PlayerEnv<P, A, Act> {
    /// What seat `seat`'s client knows: its side's current view, the tracks of what it saw,
    /// its own orders.
    pub fn seat(&self, seat: usize) -> &ClientSession {
        &self.sessions[seat]
    }

    pub fn seat_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn plan(&self) -> &P {
        &self.plan
    }

    pub fn tick(&self) -> u32 {
        self.game.tick()
    }
}

impl<P, Act: SeatActions> PlayerEnv<P, Privileged, Act> {
    /// The whole truth of the match: for training signals and measurements, never a policy's
    /// input.
    pub fn truth(&self) -> Truth<'_> {
        self.game.truth()
    }
}

impl<P: Clone + Send, A: Access, Act: SeatActions> Episode for PlayerEnv<P, A, Act> {
    type Shared = PlayerEnvConfig<P>;
    type Plan = P;
    type Action = Act;
    type Finished = Finished<P>;
    type Error = GymError;

    fn start(shared: &Self::Shared, id: GameId, plan: P) -> Result<Self, GymError> {
        let spec = (shared.spec)(id, &plan);
        let (game, opening) = Game::<A>::new(&spec)?;
        let mut sessions = Vec::with_capacity(opening.len());
        for (seat, stream) in opening.into_iter().enumerate() {
            let mut stream = stream.into_iter();
            let Some(ServerMsg::MatchStart { info }) = stream.next() else {
                return Err(GymError::Stream {
                    seat,
                    problem: "the opening does not begin with MatchStart",
                });
            };
            let refused = |error: &dyn core::fmt::Display| GymError::Session {
                seat,
                error: error.to_string(),
            };
            let statics = MatchStatic::new(&info).map_err(|error| refused(&error))?;
            // The gym steps the match itself: nothing is acknowledged.
            let options = SessionOptions {
                ack: AckPolicy::Never,
                ..SessionOptions::default()
            };
            // The seat's own slot, whatever ids the roster uses.
            let slot = info
                .picks
                .get(seat)
                .map(|pick| pick.slot)
                .ok_or_else(|| refused(&"the roster has no pick for this seat"))?;
            let mut session = ClientSession::seat(statics, slot, None, options)
                .map_err(|error| refused(&error))?;
            for message in stream {
                ingest(&mut session, seat, message)?;
            }
            sessions.push(session);
        }
        let outgoing = Vec::with_capacity(sessions.len() * Act::PER_SEAT);
        Ok(Self {
            id,
            plan,
            game,
            sessions,
            outgoing,
            actions: core::marker::PhantomData,
        })
    }

    fn step(
        &mut self,
        shared: &Self::Shared,
        action: Act,
    ) -> Result<Option<Finished<P>>, GymError> {
        // Every seat's orders, issued by its session in the order given, then each seat's run.
        self.outgoing.clear();
        let mut ends = [0; MAX_SEATS];
        for (seat, session) in self.sessions.iter_mut().enumerate() {
            for order in action.seat(seat).iter().flatten() {
                let sent = session.issue(*order).map_err(|error| GymError::Session {
                    seat,
                    error: error.to_string(),
                })?;
                self.outgoing.push(sent);
            }
            ends[seat] = self.outgoing.len();
        }
        let mut batches: [&[OutgoingOrder]; MAX_SEATS] = [&[]; MAX_SEATS];
        let mut begin = 0;
        for (batch, &end) in batches.iter_mut().zip(&ends).take(self.sessions.len()) {
            *batch = &self.outgoing[begin..end];
            begin = end;
        }
        let sessions = &mut self.sessions;
        let seats = sessions.len();
        let end =
            self.game
                .advance_batches(&batches[..seats], shared.decision_ticks, |streams| {
                    for (seat, (session, stream)) in sessions.iter_mut().zip(streams).enumerate() {
                        for message in stream {
                            ingest(session, seat, message)?;
                        }
                    }
                    Ok::<(), GymError>(())
                })?;
        if !end.done() {
            return Ok(None);
        }
        Ok(Some(Finished {
            game: self.id,
            plan: self.plan.clone(),
            end,
            tick: self.game.tick(),
            result: self.game.result().map(|(_, stats)| stats.clone()),
            meta: A::take_meta(&mut self.game)?,
        }))
    }

    fn game(&self) -> GameId {
        self.id
    }
}

fn ingest(session: &mut ClientSession, seat: usize, message: ServerMsg) -> Result<(), GymError> {
    session
        .ingest(message, &mut ())
        .map(drop)
        .map_err(|error| GymError::Session {
            seat,
            error: format!("{error:?}"),
        })
}
