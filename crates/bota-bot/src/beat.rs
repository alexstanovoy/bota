//! The attack cycle, and how fast bodies are falling.
//!
//! Neither is on the wire. A snapshot says how long a unit takes between
//! swings but not where in that interval it stands, and it says what a body's
//! health is now but not what it will be when a blow arrives. Both are needed
//! to take a last hit: an order given a second early is a creep somebody else
//! kills.
//!
//! What the wire does carry is every blow that lands. One of the hero's own
//! says the cycle came round a wind-up ago; a run of snapshots says how fast
//! a creep is losing health.

use bota_proto::{DamageKind, EntityId, EventKind, HeroId, UnitView, WorldView};

use crate::{BITTEN_TICKS, FORECAST_TICKS, HISTORY_TICKS, Swing, swing_of};

/// What is known of the cycle and of the fall of everything in sight.
#[derive(Clone, Debug)]
pub struct Beat {
    /// What the hero's own swing is made of.
    swing: Swing,
    /// Simulation ticks in a second.
    rate: f32,
    /// The tick the last blow of the hero's own landed.
    struck_at: Option<u32>,
    /// Ticks between the start of one of the hero's swings and the next, as
    /// the last snapshot had it.
    interval: u32,
    /// The tick last seen.
    tick: u32,
    /// Health seen on each body over the last few ticks, oldest first.
    traces: Vec<Trace>,
    /// Who has struck the hero lately, and when.
    struck_by: Vec<(u32, EntityId)>,
}

/// The health of one body over the last few ticks.
#[derive(Clone, Debug)]
struct Trace {
    /// Which body.
    id: EntityId,
    /// The tick it was last seen on.
    seen: u32,
    /// Health readings, oldest first.
    samples: Vec<(u32, i32)>,
}

impl Beat {
    /// A beat that has seen nothing yet.
    pub fn new(hero: HeroId, tick_rate: u16) -> Beat {
        Beat {
            swing: swing_of(hero),
            rate: f32::from(tick_rate.max(1)),
            struck_at: None,
            interval: 0,
            tick: 0,
            traces: Vec::new(),
            struck_by: Vec::new(),
        }
    }

    /// Whether a body has struck the hero within the last few ticks.
    ///
    /// The wire says who each blow came from but not who anybody is set on,
    /// so this is the only way to tell which creeps are chewing on the hero.
    pub fn struck_me(&self, who: EntityId) -> bool {
        self.struck_by.iter().any(|(_, had)| *had == who)
    }

    /// Takes one snapshot in.
    pub fn watch(&mut self, view: &WorldView, me: Option<&UnitView>) {
        self.tick = view.tick;
        if let Some(me) = me {
            self.interval = me.attack_interval;
        }
        for unit in &view.units {
            match self.traces.iter_mut().find(|trace| trace.id == unit.id) {
                Some(trace) => {
                    trace.seen = view.tick;
                    trace.samples.push((view.tick, unit.hp));
                    let oldest = view.tick.saturating_sub(HISTORY_TICKS);
                    trace.samples.retain(|(at, _)| *at >= oldest);
                }
                None => self.traces.push(Trace {
                    id: unit.id,
                    seen: view.tick,
                    samples: vec![(view.tick, unit.hp)],
                }),
            }
        }
        let oldest = view.tick.saturating_sub(HISTORY_TICKS);
        self.traces.retain(|trace| trace.seen >= oldest);
    }

    /// Takes one tick's events in.
    pub fn saw(&mut self, tick: u32, events: &[EventKind], me: Option<EntityId>) {
        let Some(me) = me else {
            return;
        };
        for event in events {
            let EventKind::Damaged {
                source: Some(source),
                target,
                kind,
                ..
            } = event
            else {
                continue;
            };
            if *source == me && *kind == DamageKind::Physical {
                self.struck_at = Some(tick);
            }
            if *target == me {
                self.struck_by.push((tick, *source));
            }
        }
        let oldest = tick.saturating_sub(BITTEN_TICKS);
        self.struck_by.retain(|(at, _)| *at >= oldest);
    }

    /// Ticks between a swing beginning and its blow arriving over a gap.
    pub fn flight_over(&self, gap: f32) -> u32 {
        if self.swing.missile_speed <= 0 {
            return self.swing.point;
        }
        let seconds = gap.max(0.0) / self.swing.missile_speed as f32;
        self.swing.point + (seconds * self.rate).ceil() as u32
    }

    /// Ticks from now until the hero's next blow could arrive over a gap.
    ///
    /// The cycle is followed from the last blow that landed: the swing behind
    /// it began a flight before it, and the next may begin an interval after
    /// that.
    pub fn blow_lands_in(&self, gap: f32) -> u32 {
        let flight = self.flight_over(gap);
        let ready = match self.struck_at {
            None => self.tick,
            Some(struck) => struck.saturating_sub(flight) + self.interval,
        };
        ready.saturating_sub(self.tick) + flight
    }

    /// Health a body is losing each tick, from the last few ticks of it.
    ///
    /// Nought for a body that is not falling, and for one seen too briefly
    /// to tell.
    pub fn falling(&self, id: EntityId) -> f32 {
        let Some(trace) = self.traces.iter().find(|trace| trace.id == id) else {
            return 0.0;
        };
        let (Some((first_at, first_hp)), Some((last_at, last_hp))) =
            (trace.samples.first(), trace.samples.last())
        else {
            return 0.0;
        };
        let over = last_at.saturating_sub(*first_at);
        if over == 0 {
            return 0.0;
        }
        let lost = (first_hp - last_hp) as f32;
        (lost / over as f32).max(0.0)
    }

    /// What a body's health will be in so many ticks, if it keeps falling as
    /// it has been.
    ///
    /// Past [`FORECAST_TICKS`] the fall is not carried forward: what a creep
    /// is taking now says little about what it will be taking in two seconds.
    pub fn health_in(&self, unit: &UnitView, ticks: u32) -> i32 {
        let carried = ticks.min(FORECAST_TICKS) as f32;
        let left = unit.hp as f32 - self.falling(unit.id) * carried;
        left.max(0.0).round() as i32
    }
}
