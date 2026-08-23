//! The lessons, and everything settled about each of them.
//!
//! A match pays almost nothing almost all of the time, and a policy started
//! from nothing has to stumble into a creep kill by accident before it learns
//! anything at all. So it is not asked to learn the game at once. Seven
//! lessons, each a longer match than the last, each paid for something
//! narrower than winning.
//!
//! How long a lesson runs and what it is called are here. What it pays for is
//! one file, named after it. Nothing else in the crate branches on which lesson
//! is being taught except [`score`](crate::score), which is the one place that
//! turns a lesson into its own function.

/// Ticks in a minute.
pub const MINUTE: u32 = 30 * 60;

/// What the bot is being taught just now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lesson {
    /// Spend the gold it starts with.
    StockUp,
    /// Walk out to the spot the waves will meet at, before they do.
    FindTheLane,
    /// Stand on the line and up with the far end of its own wave.
    HoldTheLane,
    /// Be where their wave is, and hit it.
    MeetTheWave,
    /// The same, over the early game.
    WorkTheLane,
    /// Take their towers, kill them, and keep itself whole.
    TakeTheTowers,
    /// Be worth as much as possible.
    GrowRich,
    /// The same, with what is worn counted above what is hoarded.
    GrowStrong,
}

/// One rung: how long a lesson runs, and what it is called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rung {
    /// Which lesson.
    pub lesson: Lesson,
    /// How long its match runs.
    pub ticks: u32,
    /// What it is called.
    pub name: &'static str,
    /// Which file holds what it pays for.
    pub scored_in: &'static str,
}

/// How many lessons there are.
pub const LESSONS: usize = 8;

/// The lessons in order.
///
/// The second stops at nine hundred because the first wave walks out on that
/// tick: it is the whole of the pregame and nothing in it can turn on a creep
/// existing. How long a lesson's matches are is most of what a round of it
/// costs, which is why they start at ten seconds rather than at twenty minutes.
pub const LADDER: [Rung; LESSONS] = [
    Rung {
        lesson: Lesson::StockUp,
        ticks: 300,
        name: "stock up",
        scored_in: "marks/stock_up.rs",
    },
    Rung {
        lesson: Lesson::FindTheLane,
        ticks: 900,
        name: "find the lane",
        scored_in: "marks/find_the_lane.rs",
    },
    Rung {
        lesson: Lesson::HoldTheLane,
        ticks: 1200,
        name: "hold the lane",
        scored_in: "marks/hold_the_lane.rs",
    },
    Rung {
        lesson: Lesson::MeetTheWave,
        ticks: 3000,
        name: "meet the wave",
        scored_in: "marks/meet_the_wave.rs",
    },
    Rung {
        lesson: Lesson::WorkTheLane,
        ticks: 7 * MINUTE,
        name: "work the lane",
        scored_in: "marks/work_the_lane.rs",
    },
    Rung {
        lesson: Lesson::TakeTheTowers,
        ticks: 20 * MINUTE,
        name: "take the towers",
        scored_in: "marks/take_the_towers.rs",
    },
    Rung {
        lesson: Lesson::GrowRich,
        ticks: 30 * MINUTE,
        name: "grow rich",
        scored_in: "marks/grow_rich.rs",
    },
    Rung {
        lesson: Lesson::GrowStrong,
        ticks: 35 * MINUTE,
        name: "grow strong",
        scored_in: "marks/grow_strong.rs",
    },
];

impl Lesson {
    /// Where it sits on the ladder, counting from nought.
    pub fn at(self) -> usize {
        match self {
            Lesson::StockUp => 0,
            Lesson::FindTheLane => 1,
            Lesson::HoldTheLane => 2,
            Lesson::MeetTheWave => 3,
            Lesson::WorkTheLane => 4,
            Lesson::TakeTheTowers => 5,
            Lesson::GrowRich => 6,
            Lesson::GrowStrong => 7,
        }
    }

    /// Its rung of the ladder.
    pub fn rung(self) -> &'static Rung {
        &LADDER[self.at()]
    }

    /// How long a match of this lesson runs.
    pub fn ticks(self) -> u32 {
        self.rung().ticks
    }

    /// What it is called.
    pub fn name(self) -> &'static str {
        self.rung().name
    }

    /// The lesson a number names, counting from one.
    pub fn of(number: u8) -> Option<Lesson> {
        let at = usize::from(number).checked_sub(1)?;
        LADDER.get(at).map(|rung| rung.lesson)
    }

    /// How a plan spells it: its name with the spaces written as underscores.
    pub fn spelling(self) -> String {
        self.name().replace(' ', "_")
    }

    /// The lesson a spelling names.
    pub fn named(name: &str) -> Option<Lesson> {
        LADDER
            .iter()
            .map(|rung| rung.lesson)
            .find(|lesson| lesson.spelling() == name)
    }

    /// Whether a tick of this number still falls inside the lesson.
    pub fn covers(self, tick: u32) -> bool {
        tick < self.ticks()
    }

    /// The longest lesson, whose clock a match has to run to for every lesson
    /// to have been scored.
    pub fn longest() -> Lesson {
        LADDER
            .iter()
            .map(|rung| rung.lesson)
            .max_by_key(|lesson| lesson.ticks())
            .unwrap_or(Lesson::GrowStrong)
    }
}
