//! A lockstep match inside the caller's process.
//!
//! Every [`LocalMatch::step`] takes at most one order per seat, advances the
//! world one tick and returns what each seat's connection would have been
//! sent, through the same side view and event filter the network loop uses:
//! a seat learns nothing here it would not learn from a server.

use core::fmt;

use bota_proto::{OutgoingOrder, ServerMsg, SlotId, Team, WorldView};

use crate::game::{DelayLine, Event, MatchConfig, OrderDelay, RefusedOrder, World};
use crate::game_loop::{side_view, visible_events};

/// Invalid input or lifecycle use of a local match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalError {
    /// The match description or its order delays were refused.
    Config { message: String },
    /// The order slice does not hold one entry per seat.
    OrderCount { expected: usize, got: usize },
    /// The match already sent `MatchOver`.
    MatchOver,
}

impl fmt::Display for LocalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config { message } => write!(formatter, "match setup was refused: {message}"),
            Self::OrderCount { expected, got } => write!(
                formatter,
                "order count must equal seat count {expected}, got {got}"
            ),
            Self::MatchOver => formatter.write_str("the match cannot step after MatchOver"),
        }
    }
}

impl std::error::Error for LocalError {}

/// A synchronous lockstep match, seats in slot order.
pub struct LocalMatch {
    world: World,
    seats: Vec<SlotId>,
    /// The side each seat sees as, in seat order.
    teams: Vec<Team>,
    /// Every seat's orders on their way to the world.
    line: DelayLine,
    /// Orders refused on the tick they came due; drained every step.
    refused: Vec<RefusedOrder>,
}

impl LocalMatch {
    /// Creates a match and advances it to its first visible tick (tick one),
    /// returning each seat's opening stream: `MatchStart`, `Snapshot`,
    /// `Events`.
    pub fn new(
        cfg: &MatchConfig,
        delays: &[OrderDelay],
    ) -> Result<(Self, Vec<Vec<ServerMsg>>), LocalError> {
        let refused = |error: &dyn fmt::Display| LocalError::Config {
            message: error.to_string(),
        };
        cfg.validate().map_err(|e| refused(&e))?;
        let line = DelayLine::new(cfg, delays).map_err(|e| refused(&e))?;
        let mut world = World::for_match(cfg, cfg.rng());
        let events = world.advance(&[]);
        let local = Self {
            world,
            seats: cfg.picks.iter().map(|pick| pick.slot).collect(),
            teams: cfg.picks.iter().map(|pick| pick.team).collect(),
            line,
            refused: Vec::with_capacity(cfg.picks.len()),
        };
        let start = ServerMsg::MatchStart { info: cfg.info() };
        let mut streams: Vec<Vec<ServerMsg>> = (0..local.seats.len())
            .map(|_| {
                let mut stream = Vec::with_capacity(4);
                stream.push(start.clone());
                stream
            })
            .collect();
        local.publish(&mut streams, &events);
        Ok((local, streams))
    }

    /// Gives one optional order per seat to the lines, advances one tick and
    /// returns each seat's messages for it: rejections of orders given now,
    /// then of orders that came due and no longer held, the snapshot, the
    /// events, and `MatchOver` when the match ended (a neutral winner is a
    /// draw).
    pub fn step(
        &mut self,
        orders: &[Option<OutgoingOrder>],
    ) -> Result<Vec<Vec<ServerMsg>>, LocalError> {
        if self.world.victor().is_some() {
            return Err(LocalError::MatchOver);
        }
        if orders.len() != self.seats.len() {
            return Err(LocalError::OrderCount {
                expected: self.seats.len(),
                got: orders.len(),
            });
        }
        let mut streams: Vec<Vec<ServerMsg>> =
            orders.iter().map(|_| Vec::with_capacity(4)).collect();
        for ((stream, &slot), order) in streams.iter_mut().zip(&self.seats).zip(orders) {
            let Some(order) = order else {
                continue;
            };
            debug_assert!(
                order.seen <= self.world.tick,
                "a seat cannot have seen the future"
            );
            let given = self.line.give(
                &self.world,
                slot,
                order.seq,
                order.unit,
                order.order,
                Some(order.seen),
            );
            if let Err(reason) = given {
                stream.push(ServerMsg::OrderRejected {
                    seq: order.seq,
                    reason,
                });
            }
        }
        let commands = self.line.release(&self.world, &mut self.refused);
        for refusal in self.refused.drain(..) {
            let seat = self
                .seats
                .iter()
                .position(|&slot| slot == refusal.slot)
                .expect("the line holds only seats of the match");
            streams[seat].push(ServerMsg::OrderRejected {
                seq: refusal.seq,
                reason: refusal.reason,
            });
        }
        let events = self.world.advance(&commands);
        self.publish(&mut streams, &events);
        Ok(streams)
    }

    /// The number of seats.
    pub fn seat_count(&self) -> usize {
        self.seats.len()
    }

    /// The tick the world stands on.
    pub fn tick(&self) -> u32 {
        self.world.tick
    }

    /// The world, for a scenario to read.
    pub const fn world(&self) -> &World {
        &self.world
    }

    /// The world, for a scenario to arrange before the next step; the seats are
    /// told of it by [`Self::restate`].
    pub const fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Each seat's snapshot of the current tick with no events, as after a
    /// scenario rearranged the world.
    pub fn restate(&self) -> Vec<Vec<ServerMsg>> {
        let mut streams: Vec<Vec<ServerMsg>> =
            self.seats.iter().map(|_| Vec::with_capacity(3)).collect();
        self.publish(&mut streams, &[]);
        streams
    }

    /// Appends the finished tick as each seat sees it.
    fn publish(&self, streams: &mut [Vec<ServerMsg>], events: &[Event]) {
        for (stream, &team) in streams.iter_mut().zip(&self.teams) {
            let view: WorldView = side_view(&self.world, &self.line, team);
            stream.push(ServerMsg::Snapshot { view });
            stream.push(ServerMsg::Events {
                tick: self.world.tick,
                events: visible_events(events, Some(team)),
            });
        }
        if let Some(winner) = self.world.victor() {
            let over = ServerMsg::MatchOver {
                winner,
                stats: self.world.match_stats(),
            };
            for stream in streams {
                stream.push(over.clone());
            }
        }
    }
}
