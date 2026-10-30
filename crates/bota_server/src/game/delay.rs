//! Orders in flight between a seat and the simulation.

use std::collections::VecDeque;

use bota_core::rules::{MAX_ORDER_DELAY_TICKS, MAX_ORDERS_PER_TICK};
use bota_proto::{EntityId, Order, OrderDelayView, RejectReason, SlotId, Team};

use crate::game::{Command, MatchConfig, Stream, World};

/// How late one seat's orders take effect, in ticks past the earliest tick
/// each one could.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct OrderDelay {
    /// Ticks every order waits.
    pub base: u32,
    /// Most ticks one order waits on top of `base`, drawn evenly from
    /// `0..=jitter` on the seat's own hidden stream. Zero draws nothing.
    pub jitter: u32,
}

impl OrderDelay {
    /// Every order takes effect on the earliest tick it can.
    pub const NONE: OrderDelay = OrderDelay { base: 0, jitter: 0 };
}

/// Why the order delays of a match were refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderDelayError {
    /// Base and jitter together exceed [`MAX_ORDER_DELAY_TICKS`].
    TooLong { base: u32, jitter: u32 },
    /// Neither one delay for every seat nor one per seat.
    SeatCount { seats: usize, delays: usize },
}

impl core::fmt::Display for OrderDelayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            OrderDelayError::TooLong { base, jitter } => write!(
                f,
                "order delay of {base} ticks plus {jitter} of jitter exceeds {MAX_ORDER_DELAY_TICKS}"
            ),
            OrderDelayError::SeatCount { seats, delays } => write!(
                f,
                "{delays} order delays for {seats} seats; give one for all or one per seat"
            ),
        }
    }
}

impl std::error::Error for OrderDelayError {}

/// Whether `delays` may hold back the orders of a match of `seats` seats:
/// nothing, one delay for every seat or one per seat, each within
/// [`MAX_ORDER_DELAY_TICKS`].
pub fn check_order_delays(delays: &[OrderDelay], seats: usize) -> Result<(), OrderDelayError> {
    if !(delays.len() <= 1 || delays.len() == seats) {
        return Err(OrderDelayError::SeatCount {
            seats,
            delays: delays.len(),
        });
    }
    for delay in delays {
        if delay.base.saturating_add(delay.jitter) > MAX_ORDER_DELAY_TICKS {
            return Err(OrderDelayError::TooLong {
                base: delay.base,
                jitter: delay.jitter,
            });
        }
    }
    Ok(())
}

/// An order that did not take effect when its tick came.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefusedOrder {
    pub slot: SlotId,
    pub seq: u32,
    pub reason: RejectReason,
}

/// An accepted order on its way.
#[derive(Clone, Copy, Debug)]
struct HeldOrder {
    /// The tick it takes effect on.
    due: u32,
    /// The unit it is for, as the world named it when it was given; nothing
    /// for an order that needs no body.
    key: Option<EntityId>,
    cmd: Command,
    /// The tick of the view it was given on, as its sender said.
    seen: Option<u32>,
}

/// One seat's orders in flight.
#[derive(Clone, Debug)]
struct SeatLine {
    slot: SlotId,
    team: Team,
    delay: OrderDelay,
    stream: Stream,
    /// Due ticks never decrease from front to back: orders never overtake
    /// one another. Those due on one tick stand in the order given.
    held: VecDeque<HeldOrder>,
    /// The tick the world stood on when the seat last gave an order, and
    /// the tick that order was due: every order given while the world
    /// stands on one tick is due together.
    burst: Option<(u32, u32)>,
    /// The delay the latest applied order that named its view took effect
    /// with. Absent until one does.
    measured: Option<u32>,
}

/// Every seat's orders in flight, and how late each seat's took effect.
///
/// Every order is validated when it is given and again on the tick it takes
/// effect. With no delay the two see the same world.
#[derive(Clone, Debug)]
pub struct DelayLine {
    /// One line per seat, in slot order.
    seats: Vec<SeatLine>,
}

impl DelayLine {
    /// The lines of a match: `delays` holds nothing for no delay, one delay
    /// for every seat, or one per seat in slot order.
    pub fn new(cfg: &MatchConfig, delays: &[OrderDelay]) -> Result<DelayLine, OrderDelayError> {
        check_order_delays(delays, cfg.picks.len())?;
        let rng = cfg.rng();
        let lines = cfg
            .picks
            .iter()
            .enumerate()
            .map(|(i, pick)| SeatLine {
                slot: pick.slot,
                team: pick.team,
                delay: match delays.len() {
                    0 => OrderDelay::NONE,
                    1 => delays[0],
                    _ => delays[i],
                },
                stream: rng.for_order_delay(pick.slot),
                held: VecDeque::with_capacity(MAX_ORDER_DELAY_TICKS as usize + 1),
                burst: None,
                measured: None,
            })
            .collect::<Vec<_>>();
        debug_assert!(lines.windows(2).all(|w| w[0].slot < w[1].slot));
        Ok(DelayLine { seats: lines })
    }

    /// Validates an order a seat gives while `world` stands on its tick and,
    /// when it is valid, sends it on its way. An unqueued order to a body
    /// replaces the body's orders due on the same tick.
    pub fn give(
        &mut self,
        world: &World,
        cmd: Command,
        seen: Option<u32>,
    ) -> Result<(), RejectReason> {
        world.check_order(cmd.slot, cmd.unit, &cmd.order, cmd.queue)?;
        let Some(line) = self.seats.iter_mut().find(|line| line.slot == cmd.slot) else {
            return Err(RejectReason::NotYourUnit);
        };
        let due = line.due_of(world.tick);
        line.burst = Some((world.tick, due));
        let key = world.order_key(cmd.slot, cmd.unit, &cmd.order);
        // What the body carries out (a move, an attack, a cast...) given
        // unqueued replaces what was given it on the same tick: the last
        // command of a tick is what the body does. What is carried out at
        // once (a purchase, a sale, a swap, learning, a cheat, the Glyph)
        // is no order to a body, and neither replaces nor is replaced:
        // several on a tick all go through, as Dota's purchases do.
        let body = |order: &Order| bota_core::order_queues(*order);
        if !cmd.queued() && body(&cmd.order) {
            line.held
                .retain(|held| !(held.due == due && held.key == key && body(&held.cmd.order)));
        }
        if line.held.iter().filter(|held| held.due == due).count() >= MAX_ORDERS_PER_TICK {
            return Err(RejectReason::QueueFull);
        }
        line.held.push_back(HeldOrder {
            due,
            key,
            cmd,
            seen,
        });
        assert!(line.held.len() <= (MAX_ORDER_DELAY_TICKS as usize + 1) * MAX_ORDERS_PER_TICK);
        Ok(())
    }

    /// The orders that take effect on the tick after the one `world` stands
    /// on, seat by seat in slot order and each seat's in the order given,
    /// validated again. Those no longer valid are appended to `refused`.
    pub fn release(&mut self, world: &World, refused: &mut Vec<RefusedOrder>) -> Vec<Command> {
        let next = world.tick + 1;
        let mut commands = Vec::with_capacity(self.seats.len());
        for line in &mut self.seats {
            while let Some(front) = line.held.front() {
                assert!(front.due >= next, "an order outlived its tick");
                if front.due != next {
                    break;
                }
                let held = line.held.pop_front().expect("the front was just seen");
                let cmd = held.cmd;
                match world.check_order(cmd.slot, cmd.unit, &cmd.order, cmd.queue) {
                    Ok(()) => {
                        commands.push(cmd);
                        if let Some(seen) = held.seen.filter(|&seen| seen < next) {
                            line.measured = Some(next - 1 - seen);
                        }
                    }
                    Err(reason) => refused.push(RefusedOrder {
                        slot: line.slot,
                        seq: cmd.seq,
                        reason,
                    }),
                }
            }
        }
        commands
    }

    /// The measured delays a view through `viewer` carries: every seat of
    /// that side, or of every side when absent, in slot order.
    pub fn views(&self, viewer: Option<Team>) -> Vec<OrderDelayView> {
        self.seats
            .iter()
            .filter(|line| viewer.is_none_or(|team| team == line.team))
            .filter_map(|line| {
                line.measured.map(|ticks| OrderDelayView {
                    slot: line.slot,
                    ticks,
                })
            })
            .collect()
    }
}

impl SeatLine {
    /// The tick an order given while the world stands on `tick` is due: the
    /// one of the burst it belongs to, or the earliest the seat's delay and a
    /// fresh jitter allow, never before the orders given earlier.
    fn due_of(&mut self, tick: u32) -> u32 {
        if let Some((given, due)) = self.burst
            && given == tick
        {
            return due;
        }
        let jitter = match self.delay.jitter {
            0 => 0,
            jitter => self.stream.below(jitter + 1),
        };
        let earliest = tick + 1 + self.delay.base + jitter;
        self.held
            .back()
            .map_or(earliest, |last| earliest.max(last.due))
    }
}
