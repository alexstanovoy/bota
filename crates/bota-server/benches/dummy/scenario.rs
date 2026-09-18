//! The dummy match the criterion benches play and the fixed inputs of the
//! micro cases under it.

use bota_proto::{HeroId, MapId, Order, Pick, SlotId, Target, Team, TickMode, Vec2, WorldView};

use crate::game::{
    Clearance, Command, Entity, Fnv, MapDef, MatchConfig, OBSERVER_WARD, SENTRY_WARD, World,
    isqrt64, map_of, rules,
};

/// Ticks between two re-issued hero orders.
pub(crate) const PATROL_TICKS: u32 = 45;
/// Ticks run before every measured window.
pub(crate) const WARMUP_TICKS: u32 = 1_800;
/// Identity of the dummy match.
const MATCH_ID: u64 = 7;
/// Seed of the dummy match.
const MASTER_KEY: [u8; 32] = [0x42; 32];
/// Ticks the dummy wards stand.
const WARD_TICKS: u32 = 12_600;
/// The map the dummy is played on.
pub(crate) const DUMMY_MAP: MapId = MapId(1);
/// The radius the capsule case walks a body at, in world units.
pub(crate) const CAPSULE_RADIUS: i32 = 8;
/// Inputs in each fixed micro case.
pub(crate) const MICRO_OPS: u32 = 1_000_000;

/// The dummy game: one world, the script it is driven by and its last views.
pub(crate) struct Dummy {
    /// The world the match is played in.
    pub(crate) world: World,
    /// The circuit the heroes walk, one spot per order.
    pub(crate) patrol: Vec<Vec2>,
    /// The hidden observer of each side, Radiant first.
    pub(crate) observers: [Entity; 2],
    /// The true-sight sentry of each side, Radiant first.
    pub(crate) sentries: [Entity; 2],
    /// Whether each side saw the other's observer when the stage was set.
    pub(crate) reveal: [bool; 2],
    /// Whether each side failed to see the other's sentry when the stage was
    /// set.
    pub(crate) sentries_hidden: [bool; 2],
    /// The views of the last tick, when projection is on.
    views: Option<(WorldView, WorldView, WorldView)>,
    /// Whether a tick projects the spectator and team views.
    project: bool,
}

impl Dummy {
    /// Builds the dummy at tick zero.
    pub(crate) fn build(project: bool) -> Dummy {
        let cfg = match_config();
        let mut world = World::for_match(&cfg, cfg.rng());
        let heroes = [
            hero_at(&world, SlotId(0)).expect("slot 0 has a hero"),
            hero_at(&world, SlotId(1)).expect("slot 1 has a hero"),
        ];
        let stage = stage(world.map);
        place(&mut world, heroes[0], stage.patrol[0]);
        place(&mut world, heroes[1], stage.patrol[stage.patrol.len() / 2]);
        let observers = [
            world.spawn_ward(&OBSERVER_WARD, Team::Radiant, stage.west, WARD_TICKS),
            world.spawn_ward(&OBSERVER_WARD, Team::Dire, stage.east, WARD_TICKS),
        ];
        let sentries = [
            world.spawn_ward(
                &SENTRY_WARD,
                Team::Dire,
                stage.west + Vec2::from_ints(300, 0),
                WARD_TICKS,
            ),
            world.spawn_ward(
                &SENTRY_WARD,
                Team::Radiant,
                stage.east + Vec2::from_ints(300, 0),
                WARD_TICKS,
            ),
        ];
        world.settle();
        let mut dummy = Dummy {
            world,
            patrol: stage.patrol,
            observers,
            sentries,
            reveal: [false; 2],
            sentries_hidden: [false; 2],
            views: None,
            project,
        };
        (dummy.reveal, dummy.sentries_hidden) = dummy.check_reveal();
        dummy
    }

    /// Whether each side sees the observer of the other and fails to see its
    /// sentry, from the sight the stage was settled with.
    fn check_reveal(&self) -> ([bool; 2], [bool; 2]) {
        (
            [
                self.world.can_see(Team::Radiant, self.observers[1]),
                self.world.can_see(Team::Dire, self.observers[0]),
            ],
            [
                !self.world.can_see(Team::Radiant, self.sentries[0]),
                !self.world.can_see(Team::Dire, self.sentries[1]),
            ],
        )
    }

    /// One tick: the orders due now, then the world's own step, then the
    /// views a server tick projects.
    pub(crate) fn step(&mut self) {
        let now = self.world.tick;
        let mut cmds = Vec::new();
        if now.is_multiple_of(PATROL_TICKS) {
            let at = (now / PATROL_TICKS) as usize;
            for (index, slot) in [SlotId(0), SlotId(1)].into_iter().enumerate() {
                let point = self.patrol[(at + index * self.patrol.len() / 2) % self.patrol.len()];
                cmds.push(Command {
                    slot,
                    unit: None,
                    order: Order::Move {
                        target: Target::Pos(point),
                    },
                });
            }
        }
        self.world.advance(&cmds);
        if self.project {
            self.views = Some((
                self.world.view_full(),
                self.world.view(Team::Radiant),
                self.world.view(Team::Dire),
            ));
        }
    }

    /// The world fingerprint and the view fingerprint of the last tick.
    pub(crate) fn digest(&self) -> (u64, u64) {
        let views = self.views.as_ref().map_or(0, view_digest);
        (self.world.hash(), views)
    }
}

/// The spots the dummy stands and walks by.
struct Stage {
    /// The hero patrol circuit.
    patrol: Vec<Vec2>,
    /// The lane's west shoulder, where Radiant hides an observer.
    west: Vec2,
    /// The lane's east shoulder, where Dire hides an observer.
    east: Vec2,
}

/// Lays the dummy's spots over a map's own lane and camps.
fn stage(map: &'static MapDef) -> Stage {
    let from = map.creep_spawns[0][0];
    let to = map.creep_spawns[1][0];
    let mid = lerp(from, to, 1, 2);
    let side = Vec2::from_ints(-568, 768);
    let west = mid - side;
    let east = mid + side;
    let patrol = vec![
        from,
        lerp(from, to, 1, 3),
        map.camps[0].pos,
        west,
        mid,
        east,
        map.camps[1].pos,
        lerp(from, to, 2, 3),
        to,
    ];
    Stage { patrol, west, east }
}

/// A point a fraction of the way from one spot to another.
fn lerp(from: Vec2, to: Vec2, num: i32, den: i32) -> Vec2 {
    let (x0, y0) = (from.x.to_int(), from.y.to_int());
    let (x1, y1) = (to.x.to_int(), to.y.to_int());
    Vec2::from_ints(x0 + (x1 - x0) * num / den, y0 + (y1 - y0) * num / den)
}

/// The configuration of the dummy match.
fn match_config() -> MatchConfig {
    MatchConfig {
        match_id: MATCH_ID,
        master_key: MASTER_KEY,
        picks: vec![
            Pick {
                slot: SlotId(0),
                team: Team::Radiant,
                hero: HeroId(0),
            },
            Pick {
                slot: SlotId(1),
                team: Team::Dire,
                hero: HeroId(0),
            },
        ],
        map: DUMMY_MAP,
        tick_rate: rules::TICKS_PER_SECOND as u16,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 0,
        cheats: false,
    }
}

/// The body of a seat, while one stands.
fn hero_at(world: &World, slot: SlotId) -> Option<Entity> {
    world.seat(slot).and_then(|seat| seat.unit)
}

/// Puts a body somewhere with no walk there.
fn place(world: &mut World, hero: Entity, at: Vec2) {
    if let Some(transform) = world.transform.get_mut(hero) {
        transform.pos = at;
    }
}

/// The fingerprint of one tick's three views.
fn view_digest(views: &(WorldView, WorldView, WorldView)) -> u64 {
    let mut hash = Fnv::new();
    for view in [&views.0, &views.1, &views.2] {
        hash.u32(view.tick);
        match view.viewer {
            Some(team) => {
                hash.some(true);
                hash.team(team);
            }
            None => hash.some(false),
        }
        hash.u32(view.units.len() as u32);
        for unit in &view.units {
            hash.u32(unit.id.idx);
            hash.u32(unit.id.generation);
            hash.vec2(unit.pos);
            hash.i32(unit.hp);
        }
        hash.u32(view.projectiles.len() as u32);
        hash.u32(view.loot.len() as u32);
    }
    hash.done()
}

/// The fixed positives the isqrt64 case takes its roots of.
pub(crate) fn isqrt_inputs() -> Vec<i64> {
    draws(MICRO_OPS)
        .into_iter()
        .map(|n| (n >> 1) as i64)
        .collect()
}

/// The roots of the inputs, for the case's digest.
pub(crate) fn isqrt_answers(inputs: &[i64]) -> Vec<u64> {
    inputs.iter().map(|&n| isqrt64(n) as u64).collect()
}

/// The fixed one-step segments the capsule case walks.
pub(crate) fn capsule_pairs() -> Vec<(Vec2, Vec2)> {
    let map = map_of(DUMMY_MAP);
    let (start, end) = (map.creep_spawns[0][0], map.creep_spawns[1][0]);
    let side = Vec2::from_ints(-568, 768);
    let steps = [
        Vec2::from_ints(48, 0),
        Vec2::from_ints(-48, 0),
        Vec2::from_ints(0, 48),
        Vec2::from_ints(0, -48),
    ];
    let mut numbers = draws(MICRO_OPS.saturating_mul(4)).into_iter();
    let mut out = Vec::with_capacity(MICRO_OPS as usize);
    for _ in 0..MICRO_OPS {
        let along = (numbers.next().expect("four numbers a pair") % 1001) as i32;
        let jitter = (numbers.next().expect("four numbers a pair") % 1001) as i32 - 500;
        let step = steps[(numbers.next().expect("four numbers a pair") % 4) as usize];
        let anchor = Vec2::from_ints(
            start.x.to_int()
                + (end.x.to_int() - start.x.to_int()) * along / 1000
                + side.x.to_int() * jitter / 1000,
            start.y.to_int()
                + (end.y.to_int() - start.y.to_int()) * along / 1000
                + side.y.to_int() * jitter / 1000,
        );
        out.push((anchor, anchor + step));
    }
    out
}

/// The field the fixed micro cases walk.
pub(crate) fn field() -> Clearance {
    Clearance::of_map(map_of(DUMMY_MAP))
}

/// The fingerprint of a list of numbers.
pub(crate) fn fingerprint(numbers: &[u64]) -> u64 {
    let mut hash = Fnv::new();
    for &number in numbers {
        hash.u64(number);
    }
    hash.done()
}

/// A fixed xorshift stream, so the micro cases walk the same inputs always.
fn draws(count: u32) -> Vec<u64> {
    let mut state = 0x1234_5678_9abc_def0u64;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push(state);
    }
    out
}
