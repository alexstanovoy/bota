//! A finite plan of games played a batch at a time: bot-versus-bot evaluation.

use std::collections::VecDeque;

use crate::workers::{Job, Round};
use crate::{Episode, GameId, GymError, MAX_PLANS, MAX_SLOTS, Workers};

/// Upper bound on the games a tournament keeps live at once.
pub const MAX_PARALLEL: usize = 1024;

/// Plays planned games, at most `parallel` live at once, all stepped together each round.
///
/// Games start in plan order. After every step the finished games leave (handed to the
/// caller), the others keep their order, and the next planned games start at the end of the
/// batch until it is full again. Game `n` of the plan is `GameId { slot: group, game: n }`.
/// Each game's result depends only on its plan, so the results are the same for any
/// `parallel`, any worker count and any number of drivers sharing the workers.
pub struct Tournament<'scope, E: Episode> {
    workers: Workers<'scope, E>,
    group: usize,
    parallel: usize,
    pending: VecDeque<E::Plan>,
    next_game: u64,
    live: Vec<Box<E>>,
    round: Round<E>,
    poisoned: bool,
}

impl<'scope, E: Episode + 'scope> Tournament<'scope, E> {
    /// Plans `plans` (at most [`MAX_PLANS`]) for group `group` (below [`MAX_SLOTS`]) and starts
    /// the first `parallel` (`1..=MAX_PARALLEL`).
    pub fn new(
        workers: Workers<'scope, E>,
        group: usize,
        plans: impl IntoIterator<Item = E::Plan>,
        parallel: usize,
    ) -> Result<Self, E::Error> {
        if group >= MAX_SLOTS {
            return Err(GymError::Config("a tournament group must be below MAX_SLOTS").into());
        }
        if !(1..=MAX_PARALLEL).contains(&parallel) {
            return Err(GymError::Config("tournament parallel must be 1..=MAX_PARALLEL").into());
        }
        let mut pending = VecDeque::new();
        for plan in plans {
            if pending.len() == MAX_PLANS {
                return Err(GymError::Config("a tournament plans at most MAX_PLANS games").into());
            }
            pending.push_back(plan);
        }
        let mut tournament = Self {
            workers,
            group,
            parallel,
            pending,
            next_game: 0,
            live: Vec::with_capacity(parallel),
            round: Round::new(parallel),
            poisoned: false,
        };
        tournament.fill()?;
        Ok(tournament)
    }

    /// Starts planned games, in plan order, until `parallel` are live or the plan is spent.
    fn fill(&mut self) -> Result<(), E::Error> {
        let count = (self.parallel - self.live.len()).min(self.pending.len());
        let mut submitted = 0;
        let mut failure = None;
        for position in 0..count {
            let plan = self.pending.pop_front().expect("counted above");
            let game = GameId {
                slot: self.group,
                game: self.next_game,
            };
            self.next_game += 1;
            if let Err(error) =
                self.workers
                    .submit(position, Job::Start { game, plan }, &self.round.reply)
            {
                failure = Some(error);
                break;
            }
            submitted += 1;
        }
        let received = self.round.receive(submitted);
        for episode in &mut self.round.episodes[..submitted] {
            if let Some(episode) = episode.take() {
                self.live.push(episode);
            }
        }
        if received.is_err() || failure.is_some() {
            self.poisoned = true;
            received?;
            return Err(failure.expect("a failure").into());
        }
        Ok(())
    }

    /// The live games in batch order, for the caller to read and act on.
    pub fn live(&self) -> &[Box<E>] {
        &self.live
    }

    pub fn live_mut(&mut self) -> &mut [Box<E>] {
        &mut self.live
    }

    /// Whether every planned game has finished.
    pub fn is_over(&self) -> bool {
        self.live.is_empty() && self.pending.is_empty()
    }

    /// Steps every live game with its action, in batch order; each finished game is handed,
    /// with what it left, to `finished` in batch order, then the batch is refilled from the
    /// plan. On an error (the first failing position's, or `finished`'s) the tournament is
    /// spent.
    pub fn step(
        &mut self,
        actions: impl IntoIterator<Item = E::Action>,
        mut finished: impl FnMut(Box<E>, E::Finished) -> Result<(), E::Error>,
    ) -> Result<(), E::Error> {
        if self.poisoned {
            return Err(GymError::Poisoned.into());
        }
        let count = self.live.len();
        let mut actions = actions.into_iter();
        let mut submitted = 0;
        let mut failure = None;
        for (position, episode) in self.live.drain(..).enumerate() {
            let Some(action) = actions.next() else {
                failure.get_or_insert(GymError::ActionCount {
                    expected: count,
                    got: position,
                });
                break;
            };
            let job = Job::Step {
                episode,
                action,
                next: None,
            };
            if let Err(error) = self.workers.submit(position, job, &self.round.reply) {
                failure.get_or_insert(error);
                break;
            }
            submitted += 1;
        }
        if actions.next().is_some() {
            failure.get_or_insert(GymError::ActionCount {
                expected: count,
                got: count + 1,
            });
        }
        let received = self.round.receive(submitted);
        if received.is_err() || failure.is_some() {
            self.poisoned = true;
            received?;
            return Err(failure.expect("a failure").into());
        }
        for position in 0..count {
            let episode = self.round.episodes[position]
                .take()
                .expect("every position replied");
            match self.round.finished[position].take() {
                Some(done) => {
                    if let Err(error) = finished(episode, done) {
                        self.poisoned = true;
                        return Err(error);
                    }
                }
                None => self.live.push(episode),
            }
        }
        self.fill()
    }
}
