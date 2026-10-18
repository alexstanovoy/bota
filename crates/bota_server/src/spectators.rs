//! Who may watch a match without a seat's key, and the feed of those who
//! watch it late.

use std::collections::VecDeque;
use std::fmt;
use std::str::FromStr;

use bota_proto::{EventKind, SlotOrder, Team};

/// The longest delay a late spectator may be given, in seconds.
pub const MAX_DELAY_SECS: u32 = 600;

/// The most ticks a delayed feed holds back: twenty minutes at 30 a second.
/// Each holds three snapshots, so this bounds the feed's memory.
pub const MAX_DELAYED_TICKS: u32 = 36_000;

/// Who may watch without a seat's key. A spectator with a seat's key always
/// watches live through that seat's eyes and is told its orders, whatever
/// this says; nobody else is ever told an order live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spectators {
    /// Nobody: a spectator's `Hello` without a seat's key is refused.
    Off,
    /// Anyone, this many seconds of game time late, as Dota's `tv_delay`:
    /// the whole map with both seats' orders, or a side's fog without them.
    Delayed { secs: u32 },
    /// Anyone, live: the whole map, or a side's fog, never an order. It shows
    /// a player everything the other side does, so it is for local tools.
    Open,
}

impl Spectators {
    /// How many ticks a late spectator is held back at `tick_rate`; none
    /// unless spectators watch late.
    pub fn delay_ticks(self, tick_rate: u16) -> Option<u32> {
        match self {
            Spectators::Delayed { secs } => Some(secs * u32::from(tick_rate)),
            Spectators::Off | Spectators::Open => None,
        }
    }
}

impl FromStr for Spectators {
    type Err = String;

    fn from_str(value: &str) -> Result<Spectators, String> {
        match value {
            "off" => Ok(Spectators::Off),
            "open" => Ok(Spectators::Open),
            _ => value
                .strip_prefix("delayed=")
                .and_then(|secs| secs.parse().ok())
                .filter(|secs| (1..=MAX_DELAY_SECS).contains(secs))
                .map(|secs| Spectators::Delayed { secs })
                .ok_or_else(|| {
                    format!(
                        "'{value}' is none of off, delayed=<1..={MAX_DELAY_SECS} seconds>, open"
                    )
                }),
        }
    }
}

impl fmt::Display for Spectators {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Spectators::Off => write!(f, "off"),
            Spectators::Delayed { secs } => write!(f, "delayed={secs}"),
            Spectators::Open => write!(f, "open"),
        }
    }
}

/// One tick as the late spectators are shown it.
pub struct HeldTick {
    pub tick: u32,
    /// The snapshot frames: everything, Radiant's and Dire's (see [`sight`]).
    pub frames: [Vec<u8>; 3],
    /// The events each of the three may know.
    pub events: [Vec<EventKind>; 3],
    /// Every order taken on the tick, told with the whole map.
    pub orders: Vec<SlotOrder>,
}

/// Which of a [`HeldTick`]'s three a viewer is shown: everything, or one
/// side's.
pub fn sight(viewing: Option<Team>) -> usize {
    match viewing {
        Some(Team::Radiant) => 1,
        Some(Team::Dire) => 2,
        // Seats never sit on the jungle's side.
        Some(Team::Neutral) | None => 0,
    }
}

/// The ticks held back for late spectators, oldest first.
pub struct DelayedFeed {
    delay: u32,
    held: VecDeque<HeldTick>,
}

impl DelayedFeed {
    /// Holds every tick back `delay` ticks.
    pub fn new(delay: u32) -> DelayedFeed {
        assert!((1..=MAX_DELAYED_TICKS).contains(&delay));
        DelayedFeed {
            delay,
            held: VecDeque::new(),
        }
    }

    /// Holds a tick back and hands out the ticks now due, oldest first.
    pub fn push(&mut self, held: HeldTick) -> Vec<HeldTick> {
        let now = held.tick;
        assert!(self.held.back().is_none_or(|last| last.tick < now));
        self.held.push_back(held);
        let due = self
            .held
            .iter()
            .take_while(|held| held.tick + self.delay <= now)
            .count();
        let due: Vec<HeldTick> = self.held.drain(..due).collect();
        assert!(self.held.len() <= self.delay as usize);
        due
    }

    /// The newest tick held, the rest dropped: what late spectators are
    /// shown once the match is over and nothing is secret any more.
    pub fn take_last(&mut self) -> Option<HeldTick> {
        let last = self.held.pop_back();
        self.held.clear();
        last
    }
}
