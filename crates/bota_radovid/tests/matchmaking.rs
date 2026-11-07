//! bota_radovid against the real radovid service and the real bota_server.

mod common;

use std::thread::{self, JoinHandle};

use bota_client_core::{ClientSession, IssuedOrder, Outcome as Played, Transport, play};
use bota_core::fountain_pos;
use bota_proto::{ClientMsg, HeroId, Order, ServerMsg, Target};
use bota_radovid::proto::*;
use bota_radovid::{Handoff, Tls, spectate, take_seat};
use common::*;

#[test]
fn avatars_cross_the_sdk_service_and_public_profile_and_board_contracts() {
    let env = start();
    let mut alpha = env.account("alpha", AccountKind::Human);
    let mut beta = env.account("beta", AccountKind::Human);
    let (alpha_id, beta_id) = (
        alpha.client.me().expect("alpha").id,
        beta.client.me().expect("beta").id,
    );
    let png = include_bytes!("fixtures/avatar.png");
    let seq = alpha.client.set_avatar(png).expect("send upload");
    let Reply::Avatar {
        account,
        avatar: Some(image),
    } = alpha.reply(seq).expect("upload")
    else {
        panic!("image reply");
    };
    assert_eq!(account, alpha_id);
    assert_eq!(image.png,radovid::avatar::canonicalize(png).expect("canonical fixture"));
    let seq = beta.client.get_avatar(alpha_id).expect("send fetch");
    assert_eq!(
        beta.reply(seq),
        Ok(Reply::Avatar {
            account: alpha_id,
            avatar: Some(image.clone())
        })
    );
    let Reply::Account { account } = beta
        .ask(bota_radovid::proto::ClientMsg::Account { account: alpha_id })
        .expect("profile")
    else {
        panic!("profile");
    };
    assert_eq!(account.avatar, Some(image.version));
    env.rated_fixture(alpha_id, beta_id, RULESET);
    let seq = beta
        .client
        .leaderboard(&solo(), AccountKind::Human, "")
        .expect("board");
    let Reply::LeaderboardPage { page } = beta.reply(seq).expect("board") else {
        panic!("board");
    };
    assert_eq!(
        page.rows
            .iter()
            .find(|row| row.account == alpha_id)
            .expect("row")
            .avatar,
        Some(image.version)
    );
    let mut viewer=env.account("viewer",AccountKind::Human);
    let queries=[AvatarQuery{account:alpha_id,version:None},AvatarQuery{account:beta_id,version:None}];
    let seq=viewer.client.get_avatars(&queries).expect("batch request");
    assert_eq!(viewer.reply(seq),Ok(Reply::Avatars{changes:vec![AvatarChange{account:alpha_id,avatar:Some(image.clone())}]}));
    let cached=[AvatarQuery{account:alpha_id,version:Some(image.version)},queries[1]];
    let seq=viewer.client.get_avatars(&cached).expect("conditional request");
    assert_eq!(viewer.reply(seq),Ok(Reply::Avatars{changes:Vec::new()}));
    assert!(matches!(viewer.client.get_avatars(&[queries[0],queries[0]]),Err(bota_radovid::ClientError::Refused(Error::BadAvatarBatch))));
    let seq = alpha.client.remove_avatar().expect("remove request");
    assert!(
        matches!(alpha.reply(seq), Err(Error::Throttled { .. })),
        "removal shares the upload quota"
    );
    let seq = beta.client.remove_avatar().expect("other account removal");
    assert_eq!(
        beta.reply(seq),
        Ok(Reply::Avatar {
            account: beta_id,
            avatar: None
        })
    );
    assert!(matches!(
        alpha.client.set_avatar(&[0; 513]),
        Err(bota_radovid::ClientError::Refused(Error::BadAvatar))
    ));
    let mut invalid = env.account("invalid", AccountKind::Human);
    let seq = invalid
        .client
        .set_avatar(b"not a PNG")
        .expect("server validates");
    assert_eq!(invalid.reply(seq), Err(Error::BadAvatar));
    let seq = invalid.client.remove_avatar().expect("another attempt");
    assert!(matches!(invalid.reply(seq), Err(Error::Throttled { .. })));
}

#[test]
fn leaderboard_categories_search_own_rank_and_mode_profiles_cross_the_real_service() {
    let env = start_configured(false, |config| {
        config.limits.leaderboard_size = 1;
        let mut other = config.rulesets[0].clone();
        other.id = "other".into();
        config.rulesets.push(other);
    });
    let alpha = env.account("alpha", AccountKind::Human);
    let mut beta = env.account("beta", AccountKind::Human);
    let bot = env.bot("bot");
    let id = |account: &Account| account.client.me().expect("account").id;
    let (alpha_id, beta_id, bot_id) = (id(&alpha), id(&beta), id(&bot));
    env.rated_fixture(alpha_id, beta_id, RULESET);
    env.rated_fixture(bot_id, alpha_id, RULESET);
    env.rated_fixture(beta_id, bot_id, "other");
    for (kind, prefix, expected_id) in [
        (AccountKind::Human, "", alpha_id),
        (AccountKind::Bot, "", bot_id),
        (AccountKind::Human, "AL", alpha_id),
    ] {
        let seq = beta
            .client
            .leaderboard(&solo(), kind, prefix)
            .expect("request");
        let Reply::LeaderboardPage { page } = beta.reply(seq).expect("leaderboard") else {
            panic!("board reply");
        };
        assert_eq!(page.limit, 1);
        assert_eq!(
            beta.client
                .limits(None)
                .expect("served limits")
                .leaderboard_size,
            1
        );
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].account, expected_id);
        assert_eq!(page.rows[0].kind, kind);
        let own = page
            .own
            .expect("own row independent of category, prefix and top N");
        assert_eq!(
            (own.account, own.kind, own.rank),
            (beta_id, AccountKind::Human, 2)
        );
    }
    let Reply::Account { account } = beta
        .ask(bota_radovid::proto::ClientMsg::Account { account: beta_id })
        .expect("profile")
    else {
        panic!("profile reply");
    };
    assert_eq!(
        account
            .ratings
            .iter()
            .find(|(mode, _)| *mode == solo())
            .expect("mid")
            .1
            .rank,
        Some(2)
    );
    assert_eq!(
        account
            .ratings
            .iter()
            .find(|(mode, _)| mode.0 == "other")
            .expect("other mode")
            .1
            .rank,
        Some(1)
    );
    assert!(matches!(
        beta.client.leaderboard(&solo(), AccountKind::Human, "%"),
        Err(bota_radovid::ClientError::Refused(
            Error::BadLeaderboardSearch
        ))
    ));
    assert_eq!(
        beta.ask(bota_radovid::proto::ClientMsg::Leaderboard {
            ruleset: solo(),
            kind: AccountKind::Human,
            prefix: "%".into()
        }),
        Err(Error::BadLeaderboardSearch)
    );
}

#[test]
fn the_shipped_config_pairs_ranked_and_unranked_mid_and_full_queues() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("radovid.example.toml");
    let config = radovid::config::Config::load(&path).expect("shipped config");
    for mode in ["solo_mid", "full"] {
        let ranked = config
            .rulesets
            .iter()
            .find(|r| r.id == mode)
            .expect("ranked mode");
        assert!(ranked.rated, "{mode} supports ranked play");
        let casual = config
            .rulesets
            .iter()
            .find(|r| r.rating_ruleset.as_deref() == Some(mode))
            .expect("unranked counterpart");
        assert!(!casual.rated);
        assert_eq!(casual.teams, ranked.teams);
        assert!(
            casual.launcher.artifact.is_some(),
            "unranked {mode} keeps game replays"
        );
        assert!(
            casual
                .launcher
                .artifact_args
                .iter()
                .any(|arg| arg == "{artifact}")
        );
    }
}

fn attack_move(session: &ClientSession) -> Option<IssuedOrder> {
    let enemy = fountain_pos(session.map(), session.enemy());
    Some(IssuedOrder::hero(Order::Attack {
        target: Target::Pos(enemy),
    }))
}

/// Plays the seat of `handoff` to the end of the match.
fn play_seat(handoff: Handoff, hero: HeroId, tls: Tls) -> JoinHandle<Played> {
    thread::spawn(move || {
        let human = handoff
            .seats
            .iter()
            .any(|seat| seat.slot == handoff.slot && seat.kind == AccountKind::Human);
        let (mut link, seated) = if human {
            let mut link = bota_radovid::connect(&handoff, &tls).expect("human connection");
            let lobby = bota_client_core::greet(
                &mut link,
                bota_proto::Role::Player,
                &bota_radovid::seat_name(&handoff).expect("name"),
                Some(bota_radovid::seat_key(&handoff)),
            )
            .expect("human seat");
            let mut seated = lobby.seated().expect("assigned seat");
            link.send(&ClientMsg::PickHero { hero }).expect("pick");
            link.send(&ClientMsg::SetReady(true)).expect("ready");
            seated.hero = Some(hero);
            (link, seated)
        } else {
            take_seat(&handoff, hero, &tls).expect("bot seat")
        };
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
        (AccountGameLimit::Unlimited, 120_000)
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

fn queue_audiences(account: &mut Account, audiences: &[Audience]) -> Vec<QueueEntry> {
    let seq = account
        .client
        .queue_with_audiences(&solo(), audiences)
        .expect("send search");
    match account.reply(seq).expect("queue admission") {
        Reply::QueuesJoined { entries } => entries,
        other => panic!("queue reply: {other:?}"),
    }
}

#[test]
fn a_human_opted_into_both_queues_plays_a_bot_and_both_entries_are_consumed() {
    let env = start();
    let mut human = env.account("human", AccountKind::Human);
    let mut bot = env.bot("bot");
    let humans = queue_audiences(&mut human, &[Audience::HumansOnly]);
    bot.queue();
    let isolated = env.activities("human");
    assert!(
        isolated.proposals.is_empty(),
        "a humans-only search cannot pair with a waiting bot"
    );
    assert_eq!(isolated.queued[0].audience, Audience::HumansOnly);
    human
        .ask(bota_radovid::proto::ClientMsg::QueueLeave {
            entry: humans[0].entry,
        })
        .expect("leave");
    human.client.set_auto_accept(true);
    let mixed = queue_audiences(&mut human, &[Audience::HumansOnly, Audience::Everyone]);
    assert_eq!(mixed.len(), 2);
    assert_eq!(mixed[0].search, mixed[1].search);
    let handoffs = found(&mut [&mut human, &mut bot]);
    assert_eq!(handoffs[0].match_id, handoffs[1].match_id);
    assert!(human.client.searches().entries().is_empty());
    assert!(
        env.activities("human").queued.is_empty(),
        "the service consumed the sibling atomically too"
    );
    let first = play_seat(handoffs[0].clone(), HeroId::SHADOW_FIEND, env.tls.clone());
    let second = play_seat(handoffs[1].clone(), HeroId::PUDGE, env.tls.clone());
    assert_eq!(
        first.join().expect("human").winner,
        second.join().expect("bot").winner
    );
    settle(&mut [&mut human, &mut bot], |account| {
        account.result().is_some()
    });
    let result = human.result().expect("result");
    assert!(result.rated);
    assert!(
        result.seats.iter().all(|seat| seat.before != seat.after),
        "each account keeps its own updated ladder rating"
    );
}

#[test]
fn a_bot_cannot_enter_humans_and_no_bots_only_queue_exists() {
    let env = start();
    let mut bot = env.bot("bot");
    let request = |audience| bota_radovid::proto::ClientMsg::QueueJoinAudiences {
        ruleset: solo(),
        audiences: vec![audience],
    };
    assert_eq!(
        bot.ask(request(Audience::HumansOnly)),
        Err(Error::AudienceMismatch)
    );
    assert_eq!(
        bot.ask(request(Audience::BotsOnly)),
        Err(Error::BadQueueAudiences)
    );
    assert!(env.activities("bot").queued.is_empty());
}

#[test]
fn consuming_one_linked_search_preserves_another_concurrent_search() {
    let env = start();
    let mut human = env.account("human", AccountKind::Human);
    let (mut first_bot, mut second_bot) = (env.bot("firstbot"), env.bot("secondbot"));
    let first = queue_audiences(&mut human, &[Audience::HumansOnly, Audience::Everyone]);
    let second = queue_audiences(&mut human, &[Audience::HumansOnly, Audience::Everyone]);
    first_bot.queue();
    second_bot.queue();
    settle(
        &mut [&mut human, &mut first_bot, &mut second_bot],
        |account| {
            account.client.searches().offers().count()
                == if account.client.me().expect("signed in").kind == AccountKind::Human {
                    2
                } else {
                    1
                }
        },
    );
    let snapshot = env.activities("human");
    assert_eq!(
        snapshot.proposals.len(),
        2,
        "independent searches may reserve concurrent games"
    );
    let proposal = snapshot
        .proposals
        .iter()
        .find(|proposal| proposal.search == first[0].search)
        .expect("first search")
        .proposal;
    let seq = human.client.accept(proposal).expect("accept first search");
    human.reply(seq).expect("accepted");
    settle(
        &mut [&mut human, &mut first_bot, &mut second_bot],
        |account| {
            account.client.me().expect("signed in").kind != AccountKind::Human
                || account.client.searches().groups().len() == 1
        },
    );
    let snapshot = env.activities("human");
    assert_eq!(snapshot.queued.len(), 2);
    assert!(
        snapshot
            .queued
            .iter()
            .all(|entry| entry.search == second[0].search)
    );
    assert_eq!(
        human.client.searches().entries().len(),
        2,
        "waiting updates consume only their own search"
    );
    assert!(
        human
            .client
            .searches()
            .entries()
            .iter()
            .all(|entry| entry.search == second[0].search)
    );
    let seq = human
        .client
        .leave_queue(second[0].entry)
        .expect("cancel the reserved sibling");
    human.reply(seq).expect("cancelled");
    settle(&mut [&mut human], |account| {
        account.client.searches().entries().is_empty()
    });
    assert!(env.activities("human").queued.is_empty());
}

#[test]
fn a_linked_human_search_plays_a_bot_unranked_with_served_windows_and_no_rating_change() {
    let env = start_configured(false, |config| {
        config.rulesets[0].rating_window = radovid::config::RatingWindowConfig {
            initial: Some(100),
            widen_per_minute: Some(0),
            maximum: Some(100),
        };
        let mut casual = config.rulesets[0].clone();
        casual.id = "casual_mid".into();
        casual.name = "Unranked Mid".into();
        casual.rated = false;
        casual.rating_ruleset = Some(RULESET.into());
        casual.rating_window.initial = Some(700);
        casual.rating_window.widen_per_minute = Some(0);
        casual.rating_window.maximum = Some(700);
        config.rulesets.push(casual);
    });
    let (mut alpha, mut beta) = (env.account("alpha", AccountKind::Human), env.bot("beta"));
    alpha.client.set_auto_accept(true);
    settle(&mut [&mut alpha], |a| a.client.limits(None).is_some());
    let casual = RulesetId("casual_mid".into());
    let info = alpha
        .client
        .rulesets()
        .find(|r| r.id == casual)
        .expect("unranked catalog entry");
    assert!(!info.rated);
    assert_eq!(info.rating_ruleset, Some(solo()));
    assert_eq!(info.rating_window.expect("queue policy").initial, 700);
    let seq = alpha
        .client
        .request_limits(Some(casual.clone()))
        .expect("query policy");
    alpha.reply(seq).expect("limits");
    assert_eq!(
        alpha
            .client
            .limits(Some(&casual))
            .expect("served mode policy")
            .unranked_window
            .initial,
        700
    );
    let seq = alpha
        .client
        .queue_with_audiences(&casual, &[Audience::HumansOnly, Audience::Everyone])
        .expect("linked unranked search");
    let Reply::QueuesJoined { entries } = alpha.reply(seq).expect("unranked queues") else {
        panic!("linked reply");
    };
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.window_capped));
    assert_eq!(alpha.client.searches().entries().len(), 2);
    assert_eq!(alpha.client.searches().groups().len(), 1);
    assert!(
        alpha
            .client
            .searches()
            .entries()
            .iter()
            .all(|search| search.window_capped),
        "the adapter preserves the served cap hint for both places"
    );
    let seq = beta.client.queue(&casual).expect("queue request");
    beta.reply(seq).expect("unranked queue");
    let handoffs = found(&mut [&mut alpha, &mut beta]);
    assert!(handoffs.iter().all(|h| !h.rated && h.ruleset == casual));
    assert!(alpha.client.searches().entries().is_empty());
    assert!(
        env.activities("alpha").queued.is_empty(),
        "both linked places were consumed"
    );
    let first = play_seat(handoffs[0].clone(), HeroId::SHADOW_FIEND, env.tls.clone());
    let second = play_seat(handoffs[1].clone(), HeroId::PUDGE, env.tls.clone());
    assert_eq!(
        first.join().expect("alpha").winner,
        second.join().expect("beta").winner
    );
    settle(&mut [&mut alpha, &mut beta], |a| a.result().is_some());
    let result = alpha.result().expect("result");
    assert!(!result.rated);
    assert!(matches!(result.outcome, Outcome::Winner { .. }));
    assert!(
        result
            .seats
            .iter()
            .all(|s| s.before.is_none() && s.after.is_none())
    );
    let Reply::Account { account } = alpha
        .ask(bota_radovid::proto::ClientMsg::Account {
            account: alpha.client.me().expect("logged in").id,
        })
        .expect("profile")
    else {
        panic!("account reply");
    };
    assert!(
        account.ratings.is_empty(),
        "unranked play creates no ladder record"
    );
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

/// A block another test process has taken is left to it, though nobody listens on its ports
/// yet: its service binds them only when it launches a match.
#[test]
fn a_block_another_test_process_has_taken_is_left_to_it() {
    let block = 2_999;
    let _other = common::take_block(common::first_port(block)).expect("the block is free");
    let (first, _ours) = common::free_ports_from(block);
    assert_ne!(first, common::first_port(block));
}

#[test]
fn the_shipped_matchmaking_rulesets_fit_lobbies_and_bound_practice() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("radovid.example.toml");
    let config = radovid::config::Config::load(&path).expect("shipped matchmaking example");
    assert_eq!(config.lobby_teams, [5, 5]);
    assert_eq!(
        config
            .rulesets
            .iter()
            .map(|rules| rules.id.as_str())
            .collect::<Vec<_>>(),
        [
            "full",
            "solo_mid",
            "demo",
            "full_unranked",
            "solo_mid_unranked"
        ]
    );
    let practice = config
        .rulesets
        .iter()
        .find(|rules| rules.id == "demo")
        .expect("practice");
    assert!(practice.instant_solo);
    assert_eq!(practice.max_duration_secs, 7200);
}
