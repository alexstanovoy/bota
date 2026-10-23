//! A lockstep match inside the caller's process.
//!
//! Every [`LocalMatch::step`] takes each seat's orders, advances the world
//! one tick and returns what each seat's connection would have been
//! sent, through the same side view and event filter the network loop uses:
//! a seat learns nothing here it would not learn from a server.

use core::fmt;

use bota_proto::{OutgoingOrder, ServerMsg, SlotId, Team, WorldView};

use crate::game::{
    Command, DelayLine, Entity, Event, Handles, MatchConfig, OrderDelay, RefusedOrder, World,
};
use crate::game_loop::{side_events, side_view};

/// Invalid input or lifecycle use of a local match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalError {
    /// The match description or its order delays were refused.
    Config { message: String },
    /// The orders do not hold one slice per seat.
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
#[derive(Clone)]
pub struct LocalMatch {
    world: World,
    seats: Vec<SlotId>,
    /// The side each seat sees as, in seat order.
    teams: Vec<Team>,
    /// Every seat's orders on their way to the world.
    line: DelayLine,
    /// Orders refused on the tick they came due; drained every step.
    refused: Vec<RefusedOrder>,
    /// What each side calls the entities it is told of.
    handles: Handles,
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
        let handles = Handles::new(cfg, &world);
        let events = world.advance(&[]);
        let mut local = Self {
            world,
            seats: cfg.picks.iter().map(|pick| pick.slot).collect(),
            teams: cfg.picks.iter().map(|pick| pick.team).collect(),
            line,
            refused: Vec::with_capacity(cfg.picks.len()),
            handles,
        };
        let start = ServerMsg::MatchStart { info: cfg.info() };
        let mut streams = vec![vec![start]; local.seats.len()];
        local.publish(&mut streams, &events);
        Ok((local, streams))
    }

    /// Gives each seat's orders, in the order given, to the lines, advances
    /// one tick and returns each seat's messages for it: rejections of
    /// orders given now, then of orders that came due or came up in a queue
    /// and no longer held, the snapshot, the events, and `MatchOver` when the
    /// match ended (a neutral winner is a draw).
    pub fn step(&mut self, orders: &[&[OutgoingOrder]]) -> Result<Vec<Vec<ServerMsg>>, LocalError> {
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
        let seats = streams.iter_mut().zip(self.seats.iter().zip(&self.teams));
        for ((stream, (&slot, &team)), given) in seats.zip(orders) {
            for order in given.iter() {
                let (unit, order_given) = self.handles.heard(team, order.unit, order.order);
                debug_assert!(
                    order.seen <= self.world.tick,
                    "a seat cannot have seen the future"
                );
                let cmd = Command {
                    slot,
                    unit,
                    order: order_given,
                    queue: order.queue,
                    seq: order.seq,
                };
                if let Err(reason) = self.line.give(&self.world, cmd, Some(order.seen)) {
                    stream.push(ServerMsg::OrderRejected {
                        seq: order.seq,
                        reason,
                    });
                }
            }
        }
        let commands = self.line.release(&self.world, &mut self.refused);
        let events = self.world.advance(&commands);
        self.refused.append(&mut self.world.refused);
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

    /// The handle a seat's connection knows an entity by, once the seat was
    /// told of it.
    pub fn handle_of(&self, seat: usize, entity: Entity) -> Option<bota_proto::EntityId> {
        let team = *self.teams.get(seat)?;
        self.handles.name_of(team, crate::game::wire_id(entity))
    }

    /// Each seat's snapshot of the current tick with no events, as after a
    /// scenario rearranged the world.
    pub fn restate(&mut self) -> Vec<Vec<ServerMsg>> {
        let mut streams: Vec<Vec<ServerMsg>> =
            self.seats.iter().map(|_| Vec::with_capacity(3)).collect();
        self.publish(&mut streams, &[]);
        streams
    }

    /// Appends the finished tick as each seat sees it.
    fn publish(&mut self, streams: &mut [Vec<ServerMsg>], events: &[Event]) {
        for (stream, &team) in streams.iter_mut().zip(&self.teams) {
            let view: WorldView = side_view(&self.world, &self.line, &mut self.handles, team);
            stream.push(ServerMsg::Snapshot { view });
            stream.push(ServerMsg::Events {
                tick: self.world.tick,
                events: side_events(events, &mut self.handles, team, self.world.tick),
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
