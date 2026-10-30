//! bota_radovid against the real radovid service and the real bota_server.

mod common;

use std::thread::{self, JoinHandle};

use bota_client_core::{ClientSession, IssuedOrder, Outcome as Played, Transport, play};
use bota_core::fountain_pos;
use bota_proto::{ClientMsg, HeroId, Order, ServerMsg, Target};
use bota_radovid::proto::*;
use bota_radovid::{Handoff, Tls, spectate, take_seat};
use common::*;

fn attack_move(session: &ClientSession) -> Option<IssuedOrder> {
    let enemy = fountain_pos(session.map(), session.enemy());
    Some(IssuedOrder::hero(Order::Attack {
        target: Target::Pos(enemy),
    }))
}

/// Plays the seat of `handoff` to the end of the match.
fn play_seat(handoff: Handoff, hero: HeroId, tls: Tls) -> JoinHandle<Played> {
    thread::spawn(move || {
        let (mut link, seated) = take_seat(&handoff, hero, &tls).expect("a seat");
        play(&mut link, seated, &mut (), attack_move).expect("the match is played to its end")
    })
}

#[test]
fn two_bots_queue_accept_play_and_are_rated() {
    let env = start();
    let (mut alpha, mut beta) = (env.bot("alpha"), env.bot("beta"));

    // Before anything is asked the client knows what it may do.
    settle(&mut [&mut alpha], |a| a.client.limits(None).is_some());
    let limits = alpha.client.limits(None).expect("limits");
    assert_eq!(
        (limits.games_per_account, limits.decline_cooldown_ms),
        (8, 120_000)
    );
    let rulesets: Vec<_> = alpha.client.rulesets().map(|r| r.id.clone()).collect();
    assert_eq!(rulesets, [solo()]);
    assert!(alpha.client.auto_accept(), "a bot accepts by itself");

    let first = alpha.queue();
    assert_eq!((first.position, first.waiting), (1, 1));
    assert_eq!(alpha.client.searches().entries()[0].entry, first.entry);
    beta.queue();

    let handoffs = found(&mut [&mut alpha, &mut beta]);
    assert!(alpha.client.searches().entries().is_empty());
    assert_eq!(handoffs[0].match_id, handoffs[1].match_id);
    assert_eq!((handoffs[0].slot, handoffs[1].slot), (0, 1));
    assert!(handoffs[0].game_url.starts_with("ws://127.0.0.1:"));

    let tls = env.tls.clone();
    let first = play_seat(handoffs[0].clone(), HeroId::SHADOW_FIEND, tls.clone());
    let second = play_seat(handoffs[1].clone(), HeroId::PUDGE, tls);
    let (first, second) = (first.join().expect("alpha"), second.join().expect("beta"));
    assert_eq!(first.winner, second.winner);

    settle(&mut [&mut alpha, &mut beta], |a| a.result().is_some());
    let result = alpha.result().expect("a result");
    assert_eq!(beta.result().expect("a result").match_id, result.match_id);
    assert_eq!(result.reason, EndReason::Decided);
    assert!(result.rated);
    assert_eq!(
        heroes(&result),
        [HeroId::SHADOW_FIEND, HeroId::PUDGE],
        "the report names the picks, by slot"
    );
    let Outcome::Winner { team } = result.outcome else {
        panic!("{:?}", result.outcome);
    };
    let moved = |slot: usize| {
        let seat = &result.seats[slot];
        let (before, after) = (seat.before.expect("rated"), seat.after.expect("rated"));
        after.rating - before.rating
    };
    let (won, lost) = (moved(usize::from(team)), moved(usize::from(1 - team)));
    assert!(won > 0.0 && lost < 0.0, "ratings moved: {won} {lost}");
}

#[test]
fn a_decline_returns_the_other_to_its_place_and_costs_the_decliner_the_cooldown() {
    let env = start();
    let (mut alpha, mut beta, mut gamma) = (env.bot("alpha"), env.bot("beta"), env.bot("gamma"));
    beta.client.set_auto_accept(false);
    let first = alpha.queue();
    beta.queue();
    settle(&mut [&mut alpha, &mut beta], |a| {
        a.client.searches().offers().next().is_some()
    });
    assert_eq!(
        beta.client
            .searches()
            .offers()
            .next()
            .map(|(_, o)| o.answered),
        Some(false)
    );
    let proposal = beta.proposal().expect("a proposal").proposal;
    beta.client.decline(proposal).expect("sent");

    settle(&mut [&mut alpha, &mut beta], |a| {
        a.log
            .iter()
            .any(|e| matches!(e, Event::ProposalFailed { .. } | Event::QueueLeft { .. }))
    });
    // The accepter is where it was; the decliner is out and cooling down.
    let entries = alpha.client.searches().entries();
    assert_eq!(entries.len(), 1);
    let back = &entries[0];
    assert_eq!(
        (back.entry, back.position, back.offer.is_none()),
        (first.entry, 1, true)
    );
    assert!(beta.client.searches().entries().is_empty());
    let cooldown = beta.client.searches().cooldown_left_ms();
    assert!((100_000..=120_000).contains(&cooldown), "{cooldown}");
    let seq = beta.client.queue(&solo()).expect("sent");
    assert!(matches!(beta.reply(seq), Err(Error::QueueCooldown { .. })));

    // The next player is paired with the one who waited.
    gamma.queue();
    let handoffs = found(&mut [&mut alpha, &mut gamma]);
    assert_eq!(handoffs[0].match_id, handoffs[1].match_id);
    assert!(beta.handoff().is_none());
}

#[test]
fn a_ticket_reconnects_mid_match_and_a_spectator_watches_with_it() {
    let env = start();
    let (mut alpha, mut beta) = (env.bot("alpha"), env.bot("beta"));
    alpha.queue();
    beta.queue();
    let handoffs = found(&mut [&mut alpha, &mut beta]);
    let tls = env.tls.clone();
    let other = play_seat(handoffs[1].clone(), HeroId::PUDGE, tls.clone());

    // Alpha sits, sees the match run, and drops off; a spectator with its
    // ticket sees the match meanwhile.
    let (mut link, _) = take_seat(&handoffs[0], HeroId::SHADOW_FIEND, &tls).expect("a seat");
    let mut watcher = spectate(&handoffs[0], &tls).expect("a spectator");
    let mut ticks = 0;
    while ticks < 20 {
        // A lockstep match advances as its seats acknowledge ticks.
        if let Some(ServerMsg::Snapshot { view }) = link.recv().expect("link") {
            link.send(&ClientMsg::Ack { tick: view.tick }).expect("ack");
            ticks += 1;
        }
    }
    let mut seen = 0;
    while seen < 5 {
        if let Some(ServerMsg::Snapshot { .. }) = watcher.recv().expect("watcher") {
            seen += 1;
        }
    }
    drop(link);
    drop(watcher);

    // The service hands the same seat out again for as long as the match runs.
    let seq = alpha.client.handoff(handoffs[0].match_id).expect("sent");
    let Ok(Reply::Handoff { handoff }) = alpha.reply(seq) else {
        panic!("a handoff");
    };
    assert_eq!(handoff, handoffs[0]);
    let back = play_seat(handoff, HeroId::SHADOW_FIEND, tls);
    let (back, other) = (back.join().expect("alpha"), other.join().expect("beta"));
    assert_eq!(back.winner, other.winner);
    settle(&mut [&mut alpha, &mut beta], |a| a.result().is_some());
    assert_eq!(alpha.result().expect("result").reason, EndReason::Decided);
}

/// A service that keeps replays hands the seats of a finished match the file bota_server
/// recorded (a Zstandard stream the replay player reads to the match's end); one that does not
/// (the default) answers "not kept", and the client reads that answer.
#[test]
fn a_finished_matchs_replay_is_served_only_by_a_service_that_keeps_replays() {
    for keeping in [true, false] {
        let env = start_with(keeping);
        let (mut alpha, mut beta) = (env.bot("alpha"), env.bot("beta"));
        alpha.queue();
        beta.queue();
        let handoffs = found(&mut [&mut alpha, &mut beta]);
        let tls = env.tls.clone();
        let first = play_seat(handoffs[0].clone(), HeroId::SHADOW_FIEND, tls.clone());
        let second = play_seat(handoffs[1].clone(), HeroId::PUDGE, tls);
        let _ = (first.join().expect("alpha"), second.join().expect("beta"));
        settle(&mut [&mut alpha, &mut beta], |a| a.result().is_some());
        let match_id = alpha.result().expect("a result").match_id;
        let mut bytes = Vec::new();
        let fetched = loop {
            let offset = bytes.len() as u64;
            let seq = alpha
                .client
                .request(bota_radovid::proto::ClientMsg::MatchArtifact { match_id, offset })
                .expect("sent");
            match alpha.reply(seq) {
                Ok(Reply::Artifact { total, data, .. }) => {
                    bytes.extend_from_slice(&data);
                    if bytes.len() as u64 >= total || data.is_empty() {
                        break Ok(());
                    }
                }
                Ok(other) => panic!("{other:?}"),
                Err(error) => break Err(error),
            }
        };
        if keeping {
            assert_eq!(fetched, Ok(()));
            assert_eq!(bytes[..4], [0x28, 0xb5, 0x2f, 0xfd], "a Zstandard stream");
            // The whole stream unpacks: the server finished the file before the service kept it.
            let mut frames = Vec::new();
            std::io::Read::read_to_end(
                &mut bota_client_core::UnpackedReplay::new(std::io::Cursor::new(bytes)),
                &mut frames,
            )
            .expect("an unbroken stream, ended");
            assert!(!frames.is_empty());
        } else {
            assert_eq!(fetched, Err(Error::ArtifactsNotKept));
        }
    }
}
