//! The gym end to end: real matches through the drivers, the workers and the type split.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};

use bota_gym::{
    Episode, Finished, Game, GameId, GymError, MatchSpec, PlayerEnv, Privileged, SetupCheat,
    Tournament, VecEnv, WorkerConfig, Workers, paired_games,
};
use bota_proto::{Cheat, ItemId, RejectReason, ServerMsg, Team};
use bota_replay::meta::{MetaRecord, stat};

use support::{config, digest, duel, random_orders, to_the_middle};

fn worker_config(threads: usize, queue: usize) -> WorkerConfig {
    WorkerConfig {
        threads,
        queue,
        cpus: None,
        name: "test".to_owned(),
    }
}

/// Everything a VecEnv run showed: every seat's digest after every step, and every game that
/// finished, by position.
type Trace = (Vec<u64>, Vec<(usize, GameId, u32, Option<Team>)>);

fn play(threads: usize, queue: usize, slots: usize, decisions: usize, seconds: u32) -> Trace {
    let config = config(seconds);
    std::thread::scope(|scope| {
        let workers =
            Workers::spawn(scope, worker_config(threads, queue), &config).expect("workers");
        let mut env: VecEnv<PlayerEnv<()>> =
            VecEnv::start(workers, (0..slots).collect(), &()).expect("the first games");
        let (mut digests, mut finished) = (Vec::new(), Vec::new());
        for _ in 0..decisions {
            let actions: Vec<_> = env
                .episodes()
                .iter()
                .map(|episode| random_orders(episode, episode.game()))
                .collect();
            env.step(actions, &(), |position, game: Finished<()>| {
                finished.push((position, game.game, game.tick, game.end.winner));
            })
            .expect("a step");
            for episode in env.episodes() {
                for seat in 0..episode.seat_count() {
                    digests.push(digest(episode.seat(seat)));
                }
            }
        }
        (digests, finished)
    })
}

#[test]
fn trajectories_do_not_depend_on_worker_count_or_queue_size() {
    // Games cut ten seconds after the horn: every slot finishes one and starts the next.
    let alone = play(1, 1, 5, 420, 10);
    let shared = play(4, 5, 5, 420, 10);
    assert!(
        alone.1.len() >= 5,
        "every slot finished a game: {:?}",
        alone.1
    );
    assert_eq!(alone, shared);
}

#[test]
fn a_cut_game_reports_its_end_and_its_slot_plays_the_next_game() {
    let config = config(5);
    let limit = support::after_horn(5);
    std::thread::scope(|scope| {
        let workers = Workers::spawn(scope, worker_config(1, 1), &config).expect("workers");
        let mut env: VecEnv<PlayerEnv<()>> = VecEnv::start(workers, vec![7], &()).expect("a game");
        let mut ended = None;
        while ended.is_none() {
            let actions: Vec<_> = env.episodes().iter().map(|e| to_the_middle(e)).collect();
            env.step(actions, &(), |_, game| ended = Some(game))
                .expect("a step");
        }
        let ended = ended.expect("a finished game");
        assert_eq!(ended.game, GameId { slot: 7, game: 0 });
        assert!(
            ended.end.limited && ended.end.winner.is_none(),
            "{:?}",
            ended.end
        );
        assert_eq!(ended.tick, limit);
        assert_eq!(ended.stats.slots.len(), 2);
        assert_eq!(env.episodes()[0].game(), GameId { slot: 7, game: 1 });
        assert!(
            env.episodes()[0].tick() < limit,
            "the next game starts afresh"
        );
    });
}

#[test]
fn a_tournament_gives_every_planned_game_the_same_result_at_any_batch_size() {
    let config = seeded(8);
    let plans: Vec<_> = paired_games(40, 2, 1)
        .expect("a plan")
        .map(|pairing| pairing.seed)
        .collect();
    let play = |parallel| {
        std::thread::scope(|scope| {
            let workers = Workers::spawn(scope, worker_config(3, 4), &config).expect("workers");
            let mut tournament: Tournament<PlayerEnv<u64>> =
                Tournament::new(workers.clone(), 0, plans.clone(), parallel).expect("started");
            let mut results = Vec::new();
            while !tournament.is_over() {
                let actions: Vec<_> = tournament
                    .live()
                    .iter()
                    .map(|episode| {
                        random_orders(
                            episode,
                            GameId {
                                slot: 0,
                                game: *episode.plan(),
                            },
                        )
                    })
                    .collect();
                tournament
                    .step(actions, |_, game| {
                        results.push((game.plan, game.tick, format!("{:?}", game.stats)));
                        Ok(())
                    })
                    .expect("a round");
            }
            results.sort();
            results
        })
    };
    let one = play(1);
    assert_eq!(one.len(), plans.len());
    assert_eq!(one, play(3));
}

/// Configs of tournament games: the plan is the match seed.
fn seeded(seconds: u32) -> bota_gym::PlayerEnvConfig<u64> {
    bota_gym::PlayerEnvConfig {
        decision_ticks: 3,
        spec: Box::new(move |_, seed: &u64| duel(*seed, seconds)),
    }
}

/// A game in which nothing is simulated: it lasts `plan` steps, and panics on an action of 13.
struct Count {
    game: GameId,
    left: u64,
}

impl Episode for Count {
    type Shared = AtomicBool;
    type Plan = u64;
    type Action = u64;
    type Finished = (GameId, u64);
    type Error = GymError;

    fn start(_: &AtomicBool, game: GameId, plan: u64) -> Result<Self, GymError> {
        Ok(Self { game, left: plan })
    }

    fn step(&mut self, ran: &AtomicBool, action: u64) -> Result<Option<(GameId, u64)>, GymError> {
        ran.store(true, Ordering::Relaxed);
        assert_ne!(action, 13, "caller code fails");
        self.left -= 1;
        Ok((self.left == 0).then_some((self.game, action)))
    }

    fn game(&self) -> GameId {
        self.game
    }
}

#[test]
fn a_panicking_episode_fails_its_own_driver_and_no_other() {
    let ran = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let workers = Workers::spawn(scope, worker_config(2, 2), &ran).expect("workers");
        let mut failing: VecEnv<Count> =
            VecEnv::start(workers.clone(), (0..4).collect(), &100).expect("started");
        let mut healthy: VecEnv<Count> =
            VecEnv::start(workers, (4..8).collect(), &100).expect("started");
        let failed = failing.step([1, 1, 13, 13], &100, |_, _| {});
        assert_eq!(failed, Err(GymError::Panicked { position: 2 }));
        assert_eq!(
            failing.step([1; 4], &100, |_, _| {}),
            Err(GymError::Poisoned)
        );
        for _ in 0..10 {
            healthy
                .step([1; 4], &100, |_, _| {})
                .expect("the other driver goes on");
        }
    });
    assert!(ran.load(Ordering::Relaxed));
}

#[test]
fn a_tournament_and_a_vec_env_sharing_one_slot_of_queue_both_finish() {
    let ran = AtomicBool::new(false);
    // Games of 1..=7 steps; each finished game reports its plan's length through its action.
    let plans: Vec<u64> = (0..60).map(|index| 1 + index % 7).collect();
    let alone = std::thread::scope(|scope| {
        let workers = Workers::spawn(scope, worker_config(1, 1), &ran).expect("workers");
        run_tournament(workers, &plans, 1)
    });
    let shared = std::thread::scope(|scope| {
        let workers = Workers::spawn(scope, worker_config(1, 1), &ran).expect("workers");
        let env_workers = workers.clone();
        let busy = scope.spawn(move || {
            let mut env: VecEnv<Count> =
                VecEnv::start(env_workers, (0..16).collect(), &3).expect("started");
            let mut games = 0;
            for _ in 0..300 {
                env.step([1; 16], &3, |_, _| games += 1).expect("a step");
            }
            games
        });
        let results = run_tournament(workers, &plans, 5);
        assert_eq!(busy.join().expect("the env thread"), 16 * 100);
        results
    });
    assert_eq!(alone, shared);
    assert_eq!(alone.len(), plans.len());
}

fn run_tournament(
    workers: Workers<'_, Count>,
    plans: &[u64],
    parallel: usize,
) -> Vec<(GameId, u64)> {
    let mut tournament =
        Tournament::new(workers, 3, plans.iter().copied(), parallel).expect("started");
    let mut results = Vec::new();
    while !tournament.is_over() {
        // Never 13: the action that fails.
        let actions: Vec<u64> = tournament
            .live()
            .iter()
            .map(|game| 100 + game.game().game)
            .collect();
        tournament
            .step(actions, |_, finished| {
                results.push(finished);
                Ok(())
            })
            .expect("a round");
    }
    results.sort();
    results
}

#[test]
fn setup_cheats_reach_the_seats_before_the_first_decision() {
    let mut spec = duel(5, 10);
    spec.setup = vec![
        SetupCheat {
            seat: 0,
            cheat: Cheat::Gold { amount: 5000 },
        },
        SetupCheat {
            seat: 1,
            cheat: Cheat::Item {
                item: ItemId::CLARITY,
            },
        },
    ];
    let config = bota_gym::PlayerEnvConfig {
        decision_ticks: 3,
        spec: Box::new(move |_, _: &()| spec.clone()),
    };
    let env = PlayerEnv::<()>::start(&config, GameId { slot: 0, game: 0 }, ()).expect("a game");
    let plain =
        PlayerEnv::<()>::start(&seeded_plain(), GameId { slot: 0, game: 0 }, ()).expect("a game");
    let gold = |env: &PlayerEnv<()>, seat| env.seat(seat).own_player().and_then(|p| p.gold);
    assert_eq!(gold(&env, 0), gold(&plain, 0).map(|gold| gold + 5000));
    let carries_clarity = env.seat(1).own_hero().is_some_and(|hero| {
        hero.items
            .iter()
            .flatten()
            .any(|item| item.id == ItemId::CLARITY)
    });
    assert!(
        carries_clarity,
        "seat 1's client sees the item in its hero's bag"
    );
    assert!(
        env.seat(0).statics().metadata().cheats,
        "told it is a cheat lobby"
    );
}

fn seeded_plain() -> bota_gym::PlayerEnvConfig<()> {
    bota_gym::PlayerEnvConfig {
        decision_ticks: 3,
        spec: Box::new(|_, _: &()| duel(5, 10)),
    }
}

#[test]
fn a_rejected_setup_cheat_names_its_seat_and_reason() {
    let mut spec = duel(5, 10);
    spec.setup = vec![SetupCheat {
        seat: 1,
        cheat: Cheat::ApplyModifier {
            target: bota_proto::Target::None,
            spec: bota_proto::ModifierSpec::NOMINAL,
            ticks: 0,
        },
    }];
    let error = Game::<bota_gym::Player>::new(&spec).err().expect("refused");
    assert_eq!(
        error,
        GymError::SetupRejected {
            seat: 1,
            index: 0,
            reason: RejectReason::BadCheat
        }
    );
    assert_eq!(
        error.to_string(),
        "setup cheat 0 of seat 1 was rejected: BadCheat"
    );
}

#[test]
fn recorded_metadata_agrees_with_the_scoreboard() {
    let mut spec: MatchSpec = duel(11, 90);
    spec.record_meta = true;
    let config = bota_gym::PlayerEnvConfig {
        decision_ticks: 3,
        spec: Box::new(move |_, _: &()| spec.clone()),
    };
    let mut env = PlayerEnv::<()>::start(&config, GameId { slot: 0, game: 0 }, ()).expect("a game");
    let finished = loop {
        let orders = to_the_middle(&env);
        if let Some(finished) = env.step(&config, orders).expect("a step") {
            break finished;
        }
    };
    let meta = finished.meta.expect("recorded");
    for scored in &finished.stats.slots {
        let last = meta
            .records
            .iter()
            .rev()
            .find_map(|record| match record {
                MetaRecord::Sample { slot, stats, .. } if *slot == scored.slot.0 => Some(stats),
                _ => None,
            })
            .expect("a final sample of every seat");
        let value = |id| last.iter().find(|(stat, _)| *stat == id).map(|(_, v)| *v);
        assert_eq!(value(stat::LAST_HITS), Some(i64::from(scored.last_hits)));
        assert_eq!(value(stat::DENIES), Some(i64::from(scored.denies)));
        assert_eq!(value(stat::KILLS), Some(i64::from(scored.kills)));
        assert_eq!(value(stat::DEATHS), Some(i64::from(scored.deaths)));
    }
    let farmed: u16 = finished.stats.slots.iter().map(|seat| seat.last_hits).sum();
    assert!(farmed > 0, "ninety seconds in mid farm some creeps");
}

#[test]
fn a_privileged_fork_plays_the_same_future() {
    let (mut game, _) = Game::<Privileged>::new(&duel(3, 20)).expect("a game");
    let mut fork = game.clone();
    let hero = game.truth().hero_of(0);
    assert_eq!(hero, fork.truth().hero_of(0));
    let none = [None, None];
    for _ in 0..200 {
        let ours = game.step(&none).expect("a tick");
        let theirs = fork.step(&none).expect("a tick");
        assert_eq!(ours, theirs);
    }
    assert_eq!(
        format!("{:?}", game.truth().full_view()),
        format!("{:?}", fork.truth().full_view())
    );
}

#[test]
fn a_player_stream_holds_exactly_what_its_seat_is_told() {
    let (mut game, opening) = Game::<bota_gym::Player>::new(&duel(3, 20)).expect("a game");
    assert!(matches!(opening[0][0], ServerMsg::MatchStart { .. }));
    let streams = game.step(&[None, None]).expect("a tick");
    let (ServerMsg::Snapshot { view: radiant }, ServerMsg::Snapshot { view: dire }) =
        (&streams[0][0], &streams[1][0])
    else {
        panic!("each tick opens with the seat's snapshot");
    };
    // Each side's view holds its own player's gold and not the enemy's.
    let gold = |view: &bota_proto::WorldView, slot| {
        view.players
            .iter()
            .find(|p| p.slot.0 == slot)
            .and_then(|p| p.gold)
    };
    assert!(gold(radiant, 0).is_some() && gold(radiant, 1).is_none());
    assert!(gold(dire, 1).is_some() && gold(dire, 0).is_none());
}

#[test]
fn drivers_and_matches_refuse_what_is_out_of_bounds() {
    let ran = AtomicBool::new(false);
    std::thread::scope(|scope| {
        assert_eq!(
            Workers::<Count>::spawn(scope, worker_config(0, 1), &ran).err(),
            Some(GymError::Config("worker threads must be 1..=MAX_THREADS"))
        );
        let workers = Workers::spawn(scope, worker_config(1, 1), &ran).expect("workers");
        assert_eq!(
            VecEnv::<Count>::start(workers.clone(), vec![1, 1], &1).err(),
            Some(GymError::Config(
                "VecEnv slots must be distinct and below MAX_SLOTS"
            ))
        );
        let mut env: VecEnv<Count> = VecEnv::start(workers, vec![0, 1], &5).expect("started");
        assert_eq!(
            env.step([1], &5, |_, _| {}),
            Err(GymError::ActionCount {
                expected: 2,
                got: 1
            })
        );
    });
    let refused = |change: &dyn Fn(&mut MatchSpec)| {
        let mut spec = duel(1, 10);
        change(&mut spec);
        Game::<bota_gym::Player>::new(&spec).err()
    };
    assert_eq!(
        refused(&|spec| spec.tick_limit = 0),
        Some(GymError::Config("tick_limit must be 1..=MAX_TICK_LIMIT"))
    );
    assert_eq!(
        refused(&|spec| spec.picks.clear()),
        Some(GymError::Config("a match needs 1..=MAX_SEATS seats"))
    );
    let eleven = support::picks()
        .into_iter()
        .cycle()
        .take(11)
        .collect::<Vec<_>>();
    assert_eq!(
        refused(&|spec| spec.picks = eleven.clone()),
        Some(GymError::Config("a match needs 1..=MAX_SEATS seats"))
    );
    assert_eq!(
        refused(&|spec| spec.map = bota_proto::MapId(u16::MAX)),
        Some(GymError::Config("the map is not one bota plays"))
    );
    let (mut game, _) = Game::<bota_gym::Player>::new(&duel(1, 10)).expect("a game");
    assert_eq!(
        game.step(&[None]),
        Err(GymError::OrderCount {
            expected: 2,
            got: 1
        })
    );
}

#[test]
fn an_arranged_world_is_told_to_every_seat_as_a_fresh_snapshot() {
    let (mut game, _) = Game::<Privileged>::new(&duel(3, 20)).expect("a game");
    let tick = game.tick();
    let told = game.arrange(|world| world.seats[0].gold = 4321);
    let ServerMsg::Snapshot { view } = &told[0][0] else {
        panic!("a restated tick opens with the snapshot");
    };
    assert_eq!(view.tick, tick);
    let gold = view
        .players
        .iter()
        .find(|p| p.slot.0 == 0)
        .and_then(|p| p.gold);
    assert_eq!(gold, Some(4321));
    assert!(matches!(&told[0][1], ServerMsg::Events { events, .. } if events.is_empty()));
}
