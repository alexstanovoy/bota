//! A vectorised environment: a fixed set of slots, each playing its own endless sequence of
//! games, all stepped at once on the [`Workers`].

use crate::workers::{Job, Round};
use crate::{Episode, GameId, GymError, MAX_SLOTS, Workers};

/// Slots stepped together: one action each, one round on the workers, results in slot order.
///
/// Slot `s` plays games `GameId { slot: s, game: 0 }, { game: 1 }, …`; when a step ends a game,
/// the same worker job starts the slot's next game from the plan given to that step (reset
/// costs no round of its own), so a slot never idles. The caller reads observations straight
/// from the episodes between steps ([`Self::episodes_mut`]); nothing is copied.
///
/// One `VecEnv` belongs to one thread; several, each on its own thread, may share one
/// `Workers`. A step allocates nothing: the episodes move to the workers and back.
pub struct VecEnv<'scope, E: Episode> {
    workers: Workers<'scope, E>,
    slots: Vec<usize>,
    episodes: Vec<Box<E>>,
    round: Round<E>,
    poisoned: bool,
}

impl<'scope, E: Episode + 'scope> VecEnv<'scope, E> {
    /// Starts game 0 on each of `slots` (distinct, each below [`MAX_SLOTS`]; at most
    /// `MAX_SLOTS` of them), every one from a copy of `plan`.
    pub fn start(
        workers: Workers<'scope, E>,
        slots: Vec<usize>,
        plan: &E::Plan,
    ) -> Result<Self, E::Error>
    where
        E::Plan: Clone,
    {
        let mut env = Self::empty(workers, slots)?;
        for (position, &slot) in env.slots.iter().enumerate() {
            let job = Job::Start {
                game: GameId { slot, game: 0 },
                plan: plan.clone(),
            };
            env.workers.submit(position, job, &env.round.reply)?;
        }
        env.collect_started()?;
        Ok(env)
    }

    /// Rebuilds each of `slots`' game in flight from its snapshot with `resume`, which runs on
    /// the workers and must return the game of that slot.
    pub fn resume<S: Send + 'scope>(
        workers: Workers<'scope, E>,
        slots: Vec<usize>,
        snapshots: Vec<S>,
        resume: fn(&E::Shared, S) -> Result<E, E::Error>,
    ) -> Result<Self, E::Error> {
        if snapshots.len() != slots.len() {
            return Err(GymError::Config("one snapshot per slot").into());
        }
        let mut env = Self::empty(workers, slots)?;
        for (position, (snapshot, &slot)) in snapshots.into_iter().zip(&env.slots).enumerate() {
            let build = Box::new(move |shared: &E::Shared| resume(shared, snapshot));
            env.workers
                .submit(position, Job::Build { slot, build }, &env.round.reply)?;
        }
        env.collect_started()?;
        Ok(env)
    }

    fn empty(workers: Workers<'scope, E>, slots: Vec<usize>) -> Result<Self, GymError> {
        if slots.is_empty() || slots.len() > MAX_SLOTS {
            return Err(GymError::Config("a VecEnv needs 1..=MAX_SLOTS slots"));
        }
        for (index, &slot) in slots.iter().enumerate() {
            if slot >= MAX_SLOTS || slots[..index].contains(&slot) {
                return Err(GymError::Config(
                    "VecEnv slots must be distinct and below MAX_SLOTS",
                ));
            }
        }
        let round = Round::new(slots.len());
        Ok(Self {
            workers,
            episodes: Vec::with_capacity(slots.len()),
            slots,
            round,
            poisoned: false,
        })
    }

    fn collect_started(&mut self) -> Result<(), E::Error> {
        self.round.receive(self.slots.len())?;
        for episode in &mut self.round.episodes {
            self.episodes
                .push(episode.take().expect("every position replied"));
        }
        Ok(())
    }

    /// The slot each position plays.
    pub fn slots(&self) -> &[usize] {
        &self.slots
    }

    /// The episodes in slot order, for the caller to read observations and act.
    pub fn episodes(&self) -> &[Box<E>] {
        &self.episodes
    }

    pub fn episodes_mut(&mut self) -> &mut [Box<E>] {
        &mut self.episodes
    }

    /// Steps every episode with its action, in slot order; a game that ends hands what it left
    /// to `finished` (with its position, in position order) and the slot's next game starts
    /// from `next`. A wrong number of actions is refused before any episode moves, and the env
    /// stays usable. Any failure after that (the first failing position's) spends the env:
    /// every later call returns [`GymError::Poisoned`].
    pub fn step(
        &mut self,
        actions: impl IntoIterator<Item = E::Action>,
        next: &E::Plan,
        mut finished: impl FnMut(usize, E::Finished),
    ) -> Result<(), E::Error>
    where
        E::Plan: Clone,
    {
        if self.poisoned {
            return Err(GymError::Poisoned.into());
        }
        let count = self.episodes.len();
        // Every action is in hand before any episode leaves: a wrong count moves none of them.
        self.round.stage(actions, count)?;
        let mut submitted = 0;
        let mut stopped = None;
        for (position, episode) in self.episodes.drain(..).enumerate() {
            let action = self.round.actions[position].take().expect("staged");
            if stopped.is_some() {
                self.round.episodes[position] = Some(episode);
                continue;
            }
            let job = Job::Step {
                episode,
                action,
                next: Some(next.clone()),
            };
            match self.workers.submit(position, job, &self.round.reply) {
                Ok(()) => submitted += 1,
                Err(error) => stopped = Some(error),
            }
        }
        let received = self.round.receive(submitted);
        for (position, episode) in self.round.episodes[..count].iter_mut().enumerate() {
            if let Some(episode) = episode.take() {
                self.episodes.push(episode);
            }
            if let Some(done) = self.round.finished[position].take()
                && received.is_ok()
                && stopped.is_none()
            {
                finished(position, done);
            }
        }
        // A step that failed part-way moved some episodes and lost others: nothing after it
        // could be trusted.
        if received.is_err() || stopped.is_some() {
            self.poisoned = true;
        }
        received?;
        if let Some(error) = stopped {
            return Err(error.into());
        }
        Ok(())
    }
}
