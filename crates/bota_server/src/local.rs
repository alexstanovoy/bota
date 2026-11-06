//! A lockstep match inside the caller's process.
//!
//! Every [`LocalMatch::step`] takes each seat's orders, advances the world
//! one tick and returns what each seat's connection would have been
//! sent, through the same side view and event filter the network loop uses:
//! a seat learns nothing here it would not learn from a server.

use core::fmt;

use bota_proto::{EventKind, OutgoingOrder, ReplayRecord, ServerMsg, SlotId, Team, WorldView};

use crate::game::perspective::{side_events, side_view};
use crate::game::{
    Command, DelayLine, Entity, Event, Handles, MatchConfig, OrderDelay, RefusedOrder, World,
};

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
    /// The records of every tick so far that a server's replay would hold, until
    /// [`Self::take_records`]; absent unless the match was made by
    /// [`Self::new_recorded`].
    tape: Option<Vec<ReplayRecord>>,
}

impl LocalMatch {
    /// Creates a match and advances it to its first visible tick (tick one),
    /// returning each seat's opening stream: `MatchStart`, `Snapshot`,
    /// `Events`.
    pub fn new(
        cfg: &MatchConfig,
        delays: &[OrderDelay],
    ) -> Result<(Self, Vec<Vec<ServerMsg>>), LocalError> {
        Self::start(cfg, delays, false)
    }

    /// Like [`Self::new`], and keeps what a server's replay of the match would
    /// hold after its `MatchStart` (which is in the opening stream): the
    /// orders applied, the whole map's snapshot and the events of every tick,
    /// and `MatchOver`. [`Self::take_records`] hands them out.
    pub fn new_recorded(
        cfg: &MatchConfig,
        delays: &[OrderDelay],
    ) -> Result<(Self, Vec<Vec<ServerMsg>>), LocalError> {
        Self::start(cfg, delays, true)
    }

    fn start(
        cfg: &MatchConfig,
        delays: &[OrderDelay],
        recorded: bool,
    ) -> Result<(Self, Vec<Vec<ServerMsg>>), LocalError> {
        let refused = |error: &dyn fmt::Display| LocalError::Config {
            message: error.to_string(),
        };
        cfg.validate().map_err(|e| refused(&e))?;
        let line = DelayLine::new(cfg, delays).map_err(|e| refused(&e))?;
        let mut world = World::for_match(cfg, cfg.rng());
        let handles = Handles::new(cfg, &world);
        let before = world.tick;
        let events = world.advance_trusted(&[]);
        let mut local = Self {
            world,
            seats: cfg.picks.iter().map(|pick| pick.slot).collect(),
            teams: cfg.picks.iter().map(|pick| pick.team).collect(),
            line,
            refused: Vec::with_capacity(cfg.picks.len()),
            handles,
            tape: recorded.then(Vec::new),
        };
        local.record_tick(before, &[], &events);
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
        let before = self.world.tick;
        let (commands, events) = self.line.advance(&mut self.world, &mut self.refused);
        self.record_tick(before, &commands, &events);
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
        if self.world.tick != before {
            self.publish(&mut streams, &events);
        } else {
            self.publish_over(&mut streams);
        }
        Ok(streams)
    }

    /// The replay records kept since the last call, in the order a server
    /// writes them; empty unless the match was made by [`Self::new_recorded`].
    pub fn take_records(&mut self) -> Vec<ReplayRecord> {
        self.tape.as_mut().map(std::mem::take).unwrap_or_default()
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

    /// The hero a seat plays, while it stands.
    pub fn hero_of(&self, seat: usize) -> Option<Entity> {
        let slot = *self.seats.get(seat)?;
        self.world
            .seats
            .iter()
            .find(|held| held.slot == slot)
            .and_then(|held| held.unit)
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

    /// What a side would be told were it told again now, on a copy of the
    /// names: the check that a repeat tells the same.
    fn told_again(&self, team: Team, events: &[Event]) -> (WorldView, Vec<EventKind>) {
        let mut handles = self.handles.clone();
        let view = side_view(&self.world, &self.line, &mut handles, team);
        (
            view,
            side_events(events, &mut handles, team, self.world.tick),
        )
    }

    /// Keeps the finished tick for the replay, as the server's loop records it.
    fn record_tick(&mut self, before: u32, commands: &[Command], events: &[Event]) {
        let Some(tape) = self.tape.as_mut() else {
            return;
        };
        let tick = self.world.tick;
        if tick != before || !commands.is_empty() {
            tape.push(ReplayRecord::Orders {
                tick,
                orders: commands.iter().map(Command::told).collect(),
            });
        }
        if tick != before {
            let mut full = self.world.view_full();
            full.order_delays = self.line.views(None);
            tape.push(ReplayRecord::Msg(ServerMsg::Snapshot { view: full }));
            if !events.is_empty() {
                tape.push(ReplayRecord::Msg(ServerMsg::Events {
                    tick,
                    events: events.iter().map(|event| event.kind()).collect(),
                }));
            }
        }
        if let Some(winner) = self.world.victor() {
            tape.push(ReplayRecord::Msg(ServerMsg::MatchOver {
                winner,
                stats: self.world.match_stats(),
            }));
        }
    }

    /// Appends the finished tick as each seat sees it.
    ///
    /// Every seat of a side is told the same: the side's view and events are
    /// worked out for its first seat and handed to the others as they are.
    /// Telling a side again on the same tick names nothing anew, so it
    /// would give the same.
    fn publish(&mut self, streams: &mut [Vec<ServerMsg>], events: &[Event]) {
        let tick = self.world.tick;
        let mut told: Vec<(Team, WorldView, Vec<EventKind>)> = Vec::new();
        for (seat, stream) in streams.iter_mut().enumerate() {
            let team = self.teams[seat];
            let (view, side_told) = match told.iter().find(|(side, ..)| *side == team) {
                Some((_, view, side_told)) => {
                    debug_assert!(
                        self.told_again(team, events) == (view.clone(), side_told.clone()),
                        "a side told again on a tick is told the same"
                    );
                    (view.clone(), side_told.clone())
                }
                None => {
                    let view = side_view(&self.world, &self.line, &mut self.handles, team);
                    let side_told = side_events(events, &mut self.handles, team, tick);
                    if self.teams[seat + 1..].contains(&team) {
                        told.push((team, view.clone(), side_told.clone()));
                    }
                    (view, side_told)
                }
            };
            stream.push(ServerMsg::Snapshot { view });
            stream.push(ServerMsg::Events {
                tick,
                events: side_told,
            });
        }
        self.publish_over(streams);
    }

    fn publish_over(&self, streams: &mut [Vec<ServerMsg>]) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use bota_proto::{ItemId, Order};

    #[test]
    fn an_applied_batch_is_recorded_even_without_another_tick_at_the_time_cap() {
        let map = bota_core::map_of(bota_core::MAP2_ID);
        let cfg = MatchConfig {
            map: map.id,
            objectives: map.default_objectives,
            runes: map.default_runes,
            ..crate::game::tests::support::config()
        };
        let (mut game, _) = LocalMatch::new_recorded(&cfg, &[]).expect("recorded match");
        game.take_records();
        let purchase = Command {
            slot: SlotId(0),
            unit: None,
            order: Order::Buy {
                item: ItemId::CIRCLET,
            },
            queue: false,
            seq: 42,
        };
        let gold = game.world.seats[0].gold;
        let seen = game.tick();
        game.line
            .give(&game.world, purchase, Some(seen))
            .expect("admitted purchase");
        let (commands, events) = game.line.advance(&mut game.world, &mut game.refused);
        assert!(game.refused.is_empty());
        assert!(game.world.seats[0].gold < gold, "the purchase was applied");
        assert_eq!(commands.len(), 1);
        // Current cheats cannot jump past the cap. Pin the recorder's contract
        // for an applied batch whose recording phase advances no further tick.
        let tick = game.tick();
        game.world.clock_shift.game = bota_core::MAP2_TICK_CAP - tick;
        assert!(game.world.step().is_empty());
        assert_eq!(game.tick(), tick);
        game.record_tick(tick, &commands, &events);
        let records = game.take_records();
        assert!(
            matches!(
                records.as_slice(),
                [ReplayRecord::Orders { tick: at, orders }, ReplayRecord::Msg(ServerMsg::MatchOver { .. })]
                    if *at == tick && orders == &vec![purchase.told()]
            ),
            "an applied batch keeps its authoritative tick, without repeating a snapshot: {records:?}"
        );
    }
}
