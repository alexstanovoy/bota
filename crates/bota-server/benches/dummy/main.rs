//! The dummy tick loop and the micro cases under it, measured with criterion.
//!
//! ```text
//! cargo bench -p bota-server --bench dummy
//! ```

use std::hint::black_box;

use bota_proto::{Fixed, SlotId, Vec2};
use bota_server::game::{
    self, Entity, Fnv, Foreseen, LocalAsk, LocalScratch, Obstacles, World, isqrt64, per_tick,
    plan_local, rules,
};
use criterion::measurement::WallTime;
use criterion::{
    BatchSize, BenchmarkGroup, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main,
};

mod scenario;
use scenario::{CAPSULE_RADIUS, Dummy, WARMUP_TICKS};

/// The windows the tick loop is measured over, in ticks.
const TICK_WINDOWS: [u32; 2] = [600, 3_000];

/// The world and view fingerprints of each measured tick window.
const TICK_WINDOW_DIGESTS: [(u32, u64, u64); 2] = [
    (600, 0x66d1_240f_c318_0456, 0x5395_dd38_08cf_f17f),
    (3000, 0xd4d0_5edc_fe49_40e1, 0xc8e3_77ae_2136_3d71),
];

/// The fingerprint of the fixed isqrt64 answers.
const ISQRT_DIGEST: u64 = 0x797c_0b4c_e20e_e1b1;
/// The fingerprint of the fixed capsule answers.
const CAPSULE_DIGEST: u64 = 0x2fb1_bec2_5922_1c04;
/// The fingerprint of the fixed plan-search answers.
const PLAN_DIGEST: u64 = 0x8037_e540_903f_ad69;
/// The fingerprint of the fixed targeting answers.
const TARGETING_DIGEST: u64 = 0x9886_dd66_4506_3527;

/// The spots of the dummy circuit the plan-search case routes to.
const PLAN_GOALS: [usize; 3] = [2, 4, 6];

criterion_group!(benches, tick_loop, micro);
criterion_main!(benches);

/// Measures the whole server tick over fixed windows of the dummy game.
fn tick_loop(c: &mut Criterion) {
    check_tick_windows();
    let mut group = c.benchmark_group("tick_loop");
    for &ticks in &TICK_WINDOWS {
        group.throughput(Throughput::Elements(u64::from(ticks)));
        group.bench_with_input(BenchmarkId::from_parameter(ticks), &ticks, |b, &ticks| {
            b.iter_batched(
                ready,
                |mut dummy| {
                    play(&mut dummy, ticks);
                    dummy.world.tick
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

/// Measures the isolated cases under the tick loop.
fn micro(c: &mut Criterion) {
    let mut group = c.benchmark_group("micro");
    bench_isqrt(&mut group);
    bench_capsule(&mut group);
    bench_plan(&mut group);
    bench_targeting(&mut group);
    group.finish();
}

/// A dummy warmed up and ready for a measured window.
fn ready() -> Dummy {
    let mut dummy = Dummy::build(true);
    play(&mut dummy, WARMUP_TICKS);
    dummy
}

/// Ticks the dummy on.
fn play(dummy: &mut Dummy, ticks: u32) {
    for _ in 0..ticks {
        dummy.step();
    }
}

/// Replays each measured window and checks its fingerprints and stage.
fn check_tick_windows() {
    for &(ticks, world, views) in &TICK_WINDOW_DIGESTS {
        let mut dummy = ready();
        play(&mut dummy, ticks);
        assert_eq!(dummy.digest(), (world, views), "the {ticks}-tick window");
        assert_eq!(dummy.reveal, [true, true], "the observers must be seen");
        assert_eq!(
            dummy.sentries_hidden,
            [true, true],
            "the sentries must stay hidden"
        );
    }
}

/// Measures [`isqrt64`] over a million fixed positive draws.
fn bench_isqrt(group: &mut BenchmarkGroup<'_, WallTime>) {
    let inputs = scenario::isqrt_inputs();
    let answers = scenario::isqrt_answers(&inputs);
    assert_eq!(
        scenario::fingerprint(&answers),
        ISQRT_DIGEST,
        "the isqrt64 answers moved"
    );
    group.throughput(Throughput::Elements(inputs.len() as u64));
    group.bench_function("isqrt64", |b| {
        b.iter(|| {
            let mut sink = 0i64;
            for &n in &inputs {
                sink ^= black_box(isqrt64(black_box(n)));
            }
            black_box(sink)
        });
    });
}

/// Measures the clearance capsule test over fixed one-step lane segments.
fn bench_capsule(group: &mut BenchmarkGroup<'_, WallTime>) {
    let field = scenario::field();
    let radius = Fixed::from_int(CAPSULE_RADIUS);
    let pairs = scenario::capsule_pairs();
    let answers: Vec<u64> = pairs
        .iter()
        .map(|&(from, to)| u64::from(field.capsule_clear(from, to, radius)))
        .collect();
    assert_eq!(
        scenario::fingerprint(&answers),
        CAPSULE_DIGEST,
        "the capsule answers moved"
    );
    group.throughput(Throughput::Elements(pairs.len() as u64));
    group.bench_function("capsule_clear", |b| {
        b.iter(|| {
            let mut sink = false;
            for &(from, to) in &pairs {
                sink ^= black_box(field.capsule_clear(black_box(from), black_box(to), radius));
            }
            black_box(sink)
        });
    });
}

/// Measures the local plan search from a fixed hero ask round fixed bodies.
fn bench_plan(group: &mut BenchmarkGroup<'_, WallTime>) {
    let dummy = Dummy::build(false);
    let hero = dummy
        .world
        .seat(SlotId(0))
        .and_then(|seat| seat.unit)
        .expect("slot 0 has a hero");
    let transform = *dummy
        .world
        .transform
        .get(hero)
        .expect("the hero stands somewhere");
    let stats = *dummy.world.stats.get(hero).expect("the hero is settled");
    let hull = *dummy.world.hull.get(hero).expect("the hero has a body");
    let field = scenario::field();
    let ob = Obstacles {
        field: &field,
        extra: &[],
    };
    let bodies = plan_bodies(transform.pos);
    let asks: Vec<LocalAsk> = PLAN_GOALS
        .iter()
        .map(|&goal| LocalAsk {
            from: transform.pos,
            facing: transform.facing,
            goal: dummy.patrol[goal],
            arrive: rules::units(rules::WAYPOINT_RADIUS),
            radius: hull.collision,
            step: per_tick(stats.move_speed),
            turn_rate: stats.turn_rate,
            now: 0,
        })
        .collect();
    let mut scratch = LocalScratch::new();
    assert_eq!(
        plan_fingerprint(&ob, &bodies, &asks, &mut scratch),
        PLAN_DIGEST,
        "the plan-search answers moved"
    );
    group.throughput(Throughput::Elements(asks.len() as u64));
    group.bench_function("plan_local", |b| {
        b.iter(|| {
            let mut sink = 0u64;
            for ask in &asks {
                sink ^= black_box(plan_local(&ob, &bodies, ask, &mut scratch))
                    .0
                    .len() as u64;
            }
            black_box(sink)
        });
    });
}

/// Measures reach and hostility between the dummy's bodies.
fn bench_targeting(group: &mut BenchmarkGroup<'_, WallTime>) {
    let mut dummy = Dummy::build(false);
    play(&mut dummy, WARMUP_TICKS);
    let world = &dummy.world;
    let asks: Vec<(Entity, Fixed)> = [SlotId(0), SlotId(1)]
        .into_iter()
        .filter_map(|slot| world.seat(slot).and_then(|seat| seat.unit))
        .map(|hero| {
            let reach = world
                .stats
                .get(hero)
                .expect("the hero is settled")
                .acquisition;
            (hero, reach)
        })
        .collect();
    let bodies: Vec<Entity> = world.entities.iter().collect();
    assert_eq!(
        targeting_fingerprint(world, &asks, &bodies),
        TARGETING_DIGEST,
        "the targeting answers moved"
    );
    group.throughput(Throughput::Elements((asks.len() * bodies.len()) as u64));
    group.bench_function("reachable_hostile", |b| {
        b.iter(|| {
            let mut sink = 0u32;
            for &(seeker, reach) in &asks {
                for &other in &bodies {
                    sink += u32::from(black_box(world.hostile(seeker, other)));
                    sink += u32::from(black_box(world.reachable(seeker, reach, other)));
                }
            }
            black_box(sink)
        });
    });
    group.throughput(Throughput::Elements(asks.len() as u64));
    group.bench_function("best_valid_in_range", |b| {
        b.iter(|| {
            let mut sink = 0u32;
            for &(seeker, reach) in &asks {
                sink += u32::from(black_box(world.best_valid_in_range(seeker, reach)).is_some());
            }
            black_box(sink)
        });
    });
}

/// The bodies the plan-search case routes round.
fn plan_bodies(from: Vec2) -> [Foreseen<'static>; 2] {
    [
        Foreseen {
            at: from + Vec2::from_ints(180, 120),
            radius: Fixed::from_int(24),
            delta: Vec2::from_ints(6, 0),
            steps: &[],
            from: 0,
        },
        Foreseen {
            at: from + Vec2::from_ints(-140, 90),
            radius: Fixed::from_int(24),
            delta: Vec2::ZERO,
            steps: &[],
            from: 0,
        },
    ]
}

/// The fingerprint of the plan-search answers over the fixed asks.
fn plan_fingerprint(
    ob: &Obstacles,
    bodies: &[Foreseen],
    asks: &[LocalAsk],
    scratch: &mut LocalScratch,
) -> u64 {
    let mut hash = Fnv::new();
    for ask in asks {
        let (steps, reached) = plan_local(ob, bodies, ask, scratch);
        hash.some(reached);
        hash.u32(steps.len() as u32);
        for &step in &steps {
            hash.vec2(step);
        }
    }
    hash.done()
}

/// The fingerprint of the targeting answers over the fixed bodies.
fn targeting_fingerprint(world: &World, asks: &[(Entity, Fixed)], bodies: &[Entity]) -> u64 {
    let mut hash = Fnv::new();
    for &(seeker, reach) in asks {
        match world.best_valid_in_range(seeker, reach) {
            Some(target) => {
                hash.some(true);
                hash.entity(target);
            }
            None => hash.some(false),
        }
    }
    for &(seeker, reach) in asks {
        for &other in bodies {
            hash.some(world.hostile(seeker, other));
            hash.some(world.reachable(seeker, reach, other));
        }
    }
    hash.done()
}
