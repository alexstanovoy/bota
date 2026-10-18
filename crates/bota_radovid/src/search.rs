//! What a menu draws while the account searches for games, folded from the
//! service's replies and events.

use radovid_client::proto::{
    Activities, Event, MatchId, Proposal, ProposalId, QueueEntry, QueueEntryId, Reply, RulesetId,
};

use crate::clock::now_ms;

/// A pairing waiting for the answers of its players. The opponents are not
/// named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub proposal: ProposalId,
    /// Players that accepted so far, and players in all.
    pub accepted: u8,
    pub of: u8,
    /// This client has accepted; the rest is up to the others.
    pub answered: bool,
    deadline_ms: u64,
}

impl Offer {
    /// Milliseconds left to answer.
    pub fn left_ms(&self) -> u32 {
        u32::try_from(self.deadline_ms.saturating_sub(now_ms())).unwrap_or(u32::MAX)
    }
}

/// One queue entry and where it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub entry: QueueEntryId,
    pub ruleset: RulesetId,
    /// 1 for the oldest entry of the ruleset's queue.
    pub position: u32,
    /// Entries in the ruleset's queue.
    pub waiting: u32,
    /// Every game slot is taken: nobody is paired until one frees.
    pub overloaded: bool,
    /// Accepted matches waiting for a game slot.
    pub games_waiting: u32,
    /// The open proposal this entry would be consumed by.
    pub offer: Option<Offer>,
    joined_ms: u64,
}

impl Search {
    /// Milliseconds since the entry joined the queue.
    pub fn waited_ms(&self) -> u32 {
        u32::try_from(now_ms().saturating_sub(self.joined_ms)).unwrap_or(u32::MAX)
    }
}

/// An accepted match that waits for a game slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotWait {
    pub match_id: MatchId,
    /// Matches before it.
    pub ahead: u32,
    pub overloaded: bool,
}

/// All of an account's searches: queue entries with their proposals, accepted
/// matches waiting for a slot, and the cooldown after a decline.
#[derive(Clone, Debug, Default)]
pub struct Searches {
    entries: Vec<Search>,
    slot_waits: Vec<SlotWait>,
    cooldown_until_ms: u64,
}

impl Searches {
    pub fn entries(&self) -> &[Search] {
        &self.entries
    }

    /// The open proposals, oldest entry first.
    pub fn offers(&self) -> impl Iterator<Item = (&Search, &Offer)> {
        self.entries
            .iter()
            .filter_map(|search| Some((search, search.offer.as_ref()?)))
    }

    pub fn slot_waits(&self) -> &[SlotWait] {
        &self.slot_waits
    }

    /// Milliseconds before the account may queue again.
    pub fn cooldown_left_ms(&self) -> u32 {
        u32::try_from(self.cooldown_until_ms.saturating_sub(now_ms())).unwrap_or(u32::MAX)
    }

    /// Takes in what a login says is in flight.
    pub(crate) fn start(&mut self, activities: &Activities) {
        let now = now_ms();
        self.entries = activities
            .queued
            .iter()
            .map(|entry| search_of(entry, now))
            .collect();
        for proposal in &activities.proposals {
            self.offer(proposal, now);
        }
        self.slot_waits.clear();
        self.cooldown_until_ms = now + u64::from(activities.cooldown_ms);
    }

    pub(crate) fn fold_reply(&mut self, reply: &Reply) {
        if let Reply::Queued { entry } = reply {
            self.put(entry);
        }
    }

    pub(crate) fn fold_event(&mut self, event: &Event) {
        let now = now_ms();
        match event {
            Event::QueueStatus { entry } => self.put(entry),
            Event::MatchProposed { proposal } => self.offer(proposal, now),
            Event::ProposalUpdate {
                proposal,
                accepted,
                of,
            } => {
                if let Some(offer) = self.offer_mut(*proposal) {
                    (offer.accepted, offer.of) = (*accepted, *of);
                }
            }
            Event::ProposalFailed { proposal, .. } => {
                if let Some(search) = self.entries.iter_mut().find(|search| {
                    search
                        .offer
                        .as_ref()
                        .is_some_and(|o| o.proposal == *proposal)
                }) {
                    search.offer = None;
                }
            }
            Event::QueueLeft {
                entry, cooldown_ms, ..
            } => {
                self.entries.retain(|search| search.entry != *entry);
                if *cooldown_ms > 0 {
                    self.cooldown_until_ms = now + u64::from(*cooldown_ms);
                }
            }
            Event::MatchQueued {
                match_id,
                ahead,
                overloaded,
            } => self.slot_wait(*match_id, *ahead, *overloaded),
            Event::MatchFound { handoff } => {
                self.slot_waits.retain(|w| w.match_id != handoff.match_id)
            }
            Event::MatchResult { result } => {
                self.slot_waits.retain(|w| w.match_id != result.match_id)
            }
            _ => {}
        }
    }

    /// This client accepted `proposal`.
    pub(crate) fn answered(&mut self, proposal: ProposalId) {
        if let Some(offer) = self.offer_mut(proposal) {
            offer.answered = true;
        }
    }

    /// An accepted pairing became a match: its entry is gone without a word,
    /// and which one it was is not told, so one that this client accepted goes (they are all spent).
    fn slot_wait(&mut self, match_id: MatchId, ahead: u32, overloaded: bool) {
        if let Some(at) = self
            .entries
            .iter()
            .position(|s| s.offer.as_ref().is_some_and(|o| o.answered))
        {
            self.entries.remove(at);
        }
        let wait = SlotWait {
            match_id,
            ahead,
            overloaded,
        };
        match self.slot_waits.iter_mut().find(|w| w.match_id == match_id) {
            Some(known) => *known = wait,
            None => self.slot_waits.push(wait),
        }
    }

    fn put(&mut self, entry: &QueueEntry) {
        let now = now_ms();
        match self.entries.iter_mut().find(|s| s.entry == entry.entry) {
            Some(known) => {
                known.position = entry.position;
                known.waiting = entry.waiting;
                known.overloaded = entry.overloaded;
                known.games_waiting = entry.games_waiting;
            }
            None => self.entries.push(search_of(entry, now)),
        }
    }

    fn offer(&mut self, proposal: &Proposal, now: u64) {
        let offer = Offer {
            proposal: proposal.proposal,
            accepted: proposal.accepted,
            of: proposal.of,
            answered: false,
            deadline_ms: now + u64::from(proposal.accept_by_ms),
        };
        if let Some(search) = self.entries.iter_mut().find(|s| s.entry == proposal.entry) {
            search.offer = Some(offer);
            return;
        }
        self.entries.push(Search {
            entry: proposal.entry,
            ruleset: proposal.ruleset.clone(),
            position: 0,
            waiting: 0,
            overloaded: false,
            games_waiting: 0,
            offer: Some(offer),
            joined_ms: now,
        });
    }

    fn offer_mut(&mut self, proposal: ProposalId) -> Option<&mut Offer> {
        self.entries
            .iter_mut()
            .filter_map(|s| s.offer.as_mut())
            .find(|o| o.proposal == proposal)
    }
}

fn search_of(entry: &QueueEntry, now: u64) -> Search {
    Search {
        entry: entry.entry,
        ruleset: entry.ruleset.clone(),
        position: entry.position,
        waiting: entry.waiting,
        overloaded: entry.overloaded,
        games_waiting: entry.games_waiting,
        offer: None,
        joined_ms: now.saturating_sub(u64::from(entry.for_ms)),
    }
}
