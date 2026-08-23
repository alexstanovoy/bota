//! Turning a lesson into its own function, and keeping the running total.
//!
//! [`score`] is the one place in the crate that branches on which lesson is
//! being taught. Everything below it is a lesson's own file and knows about no
//! other lesson.
//!
//! Because a lesson's marks depend on nothing an earlier lesson taught, every
//! number can be read off one match: each lesson counts the ticks inside its
//! own window and stops. So a match run to the longest lesson's clock scores
//! the whole ladder, and what comes back describes one game rather than seven.
//!
//! Marks are not the score of a match. A lesson is a ladder to be kicked away:
//! what is paid for is a habit worth having early, not what winning is. A bot
//! that has learned the first lesson perfectly walks beautifully to its lane and
//! farms nothing.

use super::{
    find_the_lane, grow_rich, grow_strong, hold_the_lane, keep_it_legal, meet_the_wave, stock_up,
    take_the_towers, work_the_lane,
};
use crate::{Card, Carried, LADDER, LESSONS, Lesson, Moment};

/// What one tick was worth to one lesson.
///
/// The one branch on which lesson is which. A lesson's own function is handed
/// the tick and what that lesson remembers, and answers with one number.
pub fn score(lesson: Lesson, now: &Moment, carried: &mut Carried) -> f32 {
    match lesson {
        Lesson::StockUp => stock_up::score(now, carried),
        Lesson::FindTheLane => find_the_lane::score(now, carried),
        Lesson::HoldTheLane => hold_the_lane::score(now, carried),
        Lesson::MeetTheWave => meet_the_wave::score(now, carried),
        Lesson::WorkTheLane => work_the_lane::score(now, carried),
        Lesson::TakeTheTowers => take_the_towers::score(now, carried),
        Lesson::GrowRich => grow_rich::score(now, carried),
        Lesson::GrowStrong => grow_strong::score(now, carried),
        Lesson::KeepItLegal => keep_it_legal::score(now, carried),
    }
}

/// The running marks of every lesson over one match.
#[derive(Clone, Copy, Debug)]
pub struct Marker {
    card: Card,
    carried: [Carried; LESSONS],
    /// The tick each lesson stops being paid on, indexed as the ladder is.
    until: [u32; LESSONS],
}

impl Default for Marker {
    fn default() -> Marker {
        Marker::new()
    }
}

impl Marker {
    /// A marker with nothing counted yet, every lesson paid for as long as its
    /// rung of the ladder says.
    pub fn new() -> Marker {
        let mut until = [0; LESSONS];
        for rung in &LADDER {
            until[rung.lesson.at()] = rung.ticks;
        }
        Marker {
            card: Card::new(),
            carried: [Carried::default(); LESSONS],
            until,
        }
    }

    /// The same, with one lesson paid for a clock of its own rather than its
    /// rung's.
    pub fn paid_until(lesson: Lesson, ticks: u32) -> Marker {
        let mut marker = Marker::new();
        marker.until[lesson.at()] = ticks;
        marker
    }

    /// What every lesson has paid so far.
    pub fn card(&self) -> Card {
        self.card
    }

    /// Scores one whole tick, and says what it paid lesson by lesson.
    ///
    /// Called once a tick with everything that happened during it, so that a
    /// lesson is one function rather than one for standing and another for
    /// blows.
    pub fn tick(&mut self, now: &Moment) -> Card {
        let mut paid = Card::new();
        for rung in &LADDER {
            let at = rung.lesson.at();
            if now.tick() < self.until[at] {
                paid.marks[at] = score(rung.lesson, now, &mut self.carried[at]);
            }
        }
        self.card.add(&paid);
        paid
    }
}
