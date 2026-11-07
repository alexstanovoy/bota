//! Search state follows committed queue removal; unanimous voting may still fail formation.
//! The `server_sequence_*` tests follow radovid's real emission order: ProposalUpdate with
//! accepted == of, then QueueLeft(Matched) for each matched entry, then MatchQueued.

use super::Searches;
use radovid_client::proto::{
    Activities, Event, MatchId, Proposal, ProposalId, QueueEntry, QueueEntryId, QueueLeftReason,
    RulesetId,
};

fn entry(id: u32, ruleset: &str) -> QueueEntry {
    QueueEntry {
        entry: QueueEntryId(id),
        ruleset: RulesetId(ruleset.to_owned()),
        audience: radovid_client::proto::Audience::Everyone,
        search: QueueEntryId(id),
        window_capped: false,
        for_ms: 0,
        position: 1,
        waiting: 2,
        overloaded: false,
        games_waiting: 0,
    }
}

fn proposal(id: u32, entry: u32, accepted: u8) -> Proposal {
    Proposal {
        proposal: ProposalId(id),
        entry: QueueEntryId(entry),
        ruleset: RulesetId("solo_mid".to_owned()),
        audience: radovid_client::proto::Audience::Everyone,
        search: QueueEntryId(entry),
        accept_by_ms: 30_000,
        accepted,
        of: 2,
    }
}

fn two_accepted_queues() -> Searches {
    let mut searches = Searches::default();
    searches.start(&Activities {
        queued: vec![entry(1, "solo_mid"), entry(2, "all_pick")],
        proposals: vec![proposal(10, 1, 0), proposal(20, 2, 0)],
        ..Default::default()
    });
    searches.answered(ProposalId(10));
    searches.answered(ProposalId(20));
    searches
}

fn entry_ids(searches: &Searches) -> Vec<u32> {
    searches.entries().iter().map(|s| s.entry.0).collect()
}

fn open_proposals(searches: &Searches) -> Vec<u32> {
    searches.offers().map(|(_, o)| o.proposal.0).collect()
}

fn completed(proposal: u32) -> Event {
    Event::ProposalUpdate {
        proposal: ProposalId(proposal),
        accepted: 2,
        of: 2,
    }
}

fn matched(entry: u32) -> Event {
    Event::QueueLeft {
        entry: QueueEntryId(entry),
        reason: QueueLeftReason::Matched,
        cooldown_ms: 0,
    }
}

fn match_queued(ahead: u32, overloaded: bool) -> Event {
    Event::MatchQueued {
        match_id: MatchId(7),
        ahead,
        overloaded,
    }
}

#[test]
fn server_sequence_matched_entry_leaves_and_the_other_search_stays() {
    let mut searches = two_accepted_queues();
    searches.fold_event(&completed(10));
    searches.fold_event(&matched(1));
    searches.fold_event(&match_queued(0, false));
    assert_eq!(entry_ids(&searches), vec![2]);
    assert_eq!(open_proposals(&searches), vec![20]);
}

#[test]
fn server_sequence_single_search_leaves_no_entry_after_match() {
    let mut searches = Searches::default();
    searches.start(&Activities {
        queued: vec![entry(1, "solo_mid")],
        proposals: vec![proposal(10, 1, 0)],
        ..Default::default()
    });
    searches.answered(ProposalId(10));
    searches.fold_event(&completed(10));
    searches.fold_event(&matched(1));
    searches.fold_event(&match_queued(0, false));
    assert_eq!(entry_ids(&searches), Vec::<u32>::new());
}

#[test]
fn server_sequence_reconnect_then_match_leaves_no_entry() {
    let mut searches = Searches::default();
    searches.start(&Activities {
        queued: vec![entry(1, "solo_mid")],
        proposals: vec![proposal(10, 1, 1)],
        ..Default::default()
    });
    searches.fold_event(&completed(10));
    searches.fold_event(&matched(1));
    searches.fold_event(&match_queued(0, false));
    assert_eq!(entry_ids(&searches), Vec::<u32>::new());
}

#[test]
fn server_sequence_repeated_match_queued_keeps_the_second_proposal_open() {
    let mut searches = two_accepted_queues();
    searches.fold_event(&completed(10));
    searches.fold_event(&matched(1));
    searches.fold_event(&match_queued(0, false));
    searches.fold_event(&Event::ProposalUpdate {
        proposal: ProposalId(20),
        accepted: 1,
        of: 2,
    });
    searches.fold_event(&match_queued(0, true));
    assert_eq!(open_proposals(&searches), vec![20]);
}

#[test]
fn completion_then_failed_match_formation_keeps_the_entry_queued() {
    let mut searches = two_accepted_queues();
    searches.fold_event(&completed(10));
    searches.fold_event(&Event::ProposalFailed {
        proposal: ProposalId(10),
        back_in_queue: true,
    });
    assert_eq!(entry_ids(&searches), vec![1, 2]);
}

#[test]
fn completion_then_failed_match_formation_then_new_offer_restores_the_entry() {
    let mut searches = two_accepted_queues();
    searches.fold_event(&completed(10));
    searches.fold_event(&Event::ProposalFailed {
        proposal: ProposalId(10),
        back_in_queue: true,
    });
    searches.fold_event(&Event::MatchProposed {
        proposal: proposal(11, 1, 0),
    });
    let mut ids = entry_ids(&searches);
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2]);
    assert!(open_proposals(&searches).contains(&11));
}
