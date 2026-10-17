//! Orders in flight between a seat and the simulation.

use std::collections::VecDeque;

use bota_core::rules::MAX_ORDER_DELAY_TICKS;
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
    seq: u32,
    unit: Option<EntityId>,
    order: Order,
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
    /// Due ticks strictly increase from front to back: orders never overtake
    /// one another, and a later order due on the same tick replaces the
    /// earlier one.
    held: VecDeque<HeldOrder>,
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
                measured: None,
            })
            .collect::<Vec<_>>();
        debug_assert!(lines.windows(2).all(|w| w[0].slot < w[1].slot));
        Ok(DelayLine { seats: lines })
    }

    /// Validates an order a seat gives while `world` stands on its tick and,
    /// when it is valid, sends it on its way.
    pub fn give(
        &mut self,
        world: &World,
        slot: SlotId,
        seq: u32,
        unit: Option<EntityId>,
        order: Order,
        seen: Option<u32>,
    ) -> Result<(), RejectReason> {
        world.validate_order(slot, unit, &order)?;
        let Some(line) = self.seats.iter_mut().find(|line| line.slot == slot) else {
            return Err(RejectReason::NotYourUnit);
        };
        let jitter = match line.delay.jitter {
            0 => 0,
            jitter => line.stream.below(jitter + 1),
        };
        let earliest = world.tick + 1 + line.delay.base + jitter;
        let due = line
            .held
            .back()
            .map_or(earliest, |last| earliest.max(last.due));
        if line.held.back().is_some_and(|last| last.due == due) {
            line.held.pop_back();
        }
        line.held.push_back(HeldOrder {
            due,
            seq,
            unit,
            order,
            seen,
        });
        assert!(line.held.len() <= MAX_ORDER_DELAY_TICKS as usize + 1);
        Ok(())
    }

    /// The orders that take effect on the tick after the one `world` stands
    /// on, at most one per seat in slot order, validated again. Those no
    /// longer valid are appended to `refused`.
    pub fn release(&mut self, world: &World, refused: &mut Vec<RefusedOrder>) -> Vec<Command> {
        let next = world.tick + 1;
        let mut commands = Vec::with_capacity(self.seats.len());
        for line in &mut self.seats {
            let Some(front) = line.held.front() else {
                continue;
            };
            assert!(front.due >= next, "an order outlived its tick");
            if front.due != next {
                continue;
            }
            let held = line.held.pop_front().expect("the front was just seen");
            match world.validate_order(line.slot, held.unit, &held.order) {
                Ok(()) => {
                    commands.push(Command {
                        slot: line.slot,
                        unit: held.unit,
                        order: held.order,
                    });
                    if let Some(seen) = held.seen.filter(|&seen| seen < next) {
                        line.measured = Some(next - 1 - seen);
                    }
                }
                Err(reason) => refused.push(RefusedOrder {
                    slot: line.slot,
                    seq: held.seq,
                    reason,
                }),
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
