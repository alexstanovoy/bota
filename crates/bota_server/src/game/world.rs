//! The world: one table per component, and the order a tick runs in.

use std::collections::VecDeque;

use bota_proto::{HeroId, Inflictor, SlotId, Team, UnitKind, Vec2};

use crate::game::world_tables::world_tables;
use crate::game::{
    AbilityBook, Action, AppliedModifiers, AuraScratch, Backdoor, Bounty, CampHome, Candidates,
    Def, Entity, EntityAllocator, Errand, Expiry, Exposure, Forest, Glyph, Handling, Health, Hit,
    HitCx, Hook, Hull, Inventory, Landed, Lane, LaneAi, Level, Loot, Mana, March, Mark, Missed,
    MissileCx, Modifier, Modifiers, Motion, NeutralAi, Orders, Place, Plan, Projectile, Purpose,
    Rax, RequiemLine, Route, Seat, SightCx, SightScratch, SpawnModifier, Spots, Stats, StatsCx,
    Target, Tier, Transform, UnitOrder, Upgrades, Visibility, derive_stats, hitting_system,
    missile_system, regenerate, visibility_system,
};
use crate::profile::{Phase, ScopeGuard};

world_tables! {
    /// Everything a match is made of.
    ///
    /// Every component lives in a table of its own, and a table is absent for an
    /// entity the component does not apply to. A system takes the tables it needs
    /// and nothing else; [`World::step`] is the one place the order of a tick is
    /// written down.
    #[derive(Clone)]
    pub struct World;

    state {
        /// Ticks since the match began.
        pub tick: u32 = 0;
        /// A place per player.
        pub seats: Vec<Seat> = Vec::new();
        /// What each side's shop has left of every item sold from a limited
        /// stock, by side, then in catalog order.
        pub shop_stock: Vec<crate::game::ShopStock> = crate::game::fresh_stock();
        /// The completed result; `Team::Neutral` denotes a Map2 draw.
        pub winner: Option<Team> = None;
        /// Whether cheat orders are honoured.
        pub cheats: bool = false;
        /// Whether abilities and items cost no mana and leave no cooldown, for
        /// every unit (the `-wtf` cheat).
        pub wtf: bool = false;
        /// Whether lane waves and neutral camps keep from spawning on their
        /// clocks (the `-disablecreepspawn` cheat).
        pub creeps_halted: bool = false;
        /// Whether each side sees what either sees (the `-allvision` cheat).
        pub all_vision: bool = false;
        /// How far cheats moved the game clock and the time of day ahead of the
        /// tick (`-startgame`, the night toggle).
        pub clock_shift: bota_proto::ClockShift = bota_proto::ClockShift::default();
        /// Whether structures keep at least 1 hp and never fall.
        pub immortal_structures: bool = false;
        /// Whether heroes keep at least 1 hp and never die.
        pub immortal_heroes: bool = false;
        /// The map it is played on.
        pub map: &'static bota_core::MapDef = bota_core::map_of(bota_proto::MapId(0));
        /// Where every roll of the dice comes from.
        pub rng: crate::game::MatchRng = crate::game::MatchRng::new(&[0; 32], 0);
        /// Evasion sequences, an uphill miss among their sources, indexed by
        /// target's entity slot.
        pub evasion: Vec<Option<crate::game::PseudoRandom>> = Vec::new();
        /// Accuracy sequences, indexed by attacker's entity slot.
        pub accuracy: Vec<Option<crate::game::PseudoRandom>> = Vec::new();
        /// Pierce sequences, indexed by attacker's entity slot, by the slot of
        /// the item that pierces.
        pub pierce: Vec<Option<crate::game::Rolls>> = Vec::new();
        /// Critical strike sequences, indexed by attacker's entity slot, by the
        /// slot of the ability that strikes.
        pub crit: Vec<Option<crate::game::Rolls>> = Vec::new();
        /// Which roster each camp put out last, so it never draws twice running.
        pub camp_last: Vec<u8> = Vec::new();
        /// The forest as it stands: what is down, and what has been put up.
        pub trees: Forest = Forest::default();
        /// Blows dealt and not yet felt. Filled and emptied inside one tick.
        pub hits: VecDeque<Hit> = VecDeque::new();
        /// The orders refused in the last [`World::advance`]: a queued order
        /// with no room left in its unit's queue, or one that no longer held
        /// when its unit took it up. Emptied as the next one begins.
        pub refused: Vec<crate::game::RefusedOrder> = Vec::new();
        /// Which entities exist.
        pub entities: EntityAllocator = EntityAllocator::new();
        /// Each side's Glyph of Fortification: Radiant's, then Dire's.
        pub glyphs: [Glyph; 2] = [Glyph::fresh(); 2];
        /// Which of the map's objectives the match stands up.
        pub objectives: bota_proto::ObjectiveSet = bota_proto::ObjectiveSet::NONE;
        /// Trusted match setup's modifiers, put on each unit as it is stood up.
        pub spawn_modifiers: Vec<SpawnModifier> = Vec::new();
    }

    tables {
        /// Where each entity stands.
        pub transform: Transform;
        /// The room each entity takes.
        pub hull: Hull;
        /// What kind of thing each entity is.
        pub kind: UnitKind;
        /// Which side each entity is on.
        pub team: Team;
        /// Health, for whatever can be hurt.
        pub health: Health;
        /// Mana, for whatever spends it.
        pub mana: Mana;
        /// Hero levels.
        pub level: Level;
        /// Upgrade intervals a creep spawned after.
        pub upgrades: Upgrades;
        /// What a creep was raised with at its spawn beyond its kind.
        pub stat_bonus: crate::game::StatBonus;
        /// Building tiers.
        pub tier: Tier;
        /// What is on each entity.
        pub modifiers: Modifiers;
        /// Applied stat changes on each entity.
        pub applied: AppliedModifiers;
        /// The hook each entity that is one is flying.
        pub hook: Hook;
        /// What each entity that is an ability's mark shows.
        pub mark: Mark;
        /// The line of a requiem each entity that is one is flying.
        pub requiem_line: RequiemLine;
        /// The chain of damage each entity that is one is leaping along.
        pub chain: crate::game::Chain;
        /// The push each entity being carried is under.
        pub push: crate::game::Push;
        /// The Blood Grenade each entity that is one is flying.
        pub grenade: crate::game::Grenade;
        /// The feast of souls each entity that cast one has on it.
        pub feast: crate::game::Feast;
        /// The errand each courier is on.
        pub errand: Errand;
        /// How long each entity that stands for a time has left.
        pub expiry: Expiry;
        /// The route each walker is on.
        pub route: Route;
        /// The next stretch of each walker's walk.
        pub plan: Plan;
        /// How each walker has been moving.
        pub motion: Motion;
        /// What a creep keeps while marching its lane.
        pub march: March;
        /// The order each entity is following.
        pub orders: Orders;
        /// The orders each entity has queued behind the one it follows.
        pub queue: crate::game::OrderQueue;
        /// The orders each entity was given in a cast point, to carry out once
        /// the cast has gone off (Dota's pseudo-queue): the casts given there in
        /// turn, or the one move, attack or the like that replaced them.
        pub waiting: crate::game::OrderQueue;
        /// Who each entity is set on. Absent when it is set on nobody.
        pub target: Target;
        /// What each entity is doing.
        pub action: Action;
        /// What a lane creep keeps about the fight it is in.
        pub lane_ai: LaneAi;
        /// What a neutral keeps about being drawn away.
        pub neutral_ai: NeutralAi;
        /// Which camp a neutral belongs to.
        pub camp_home: CampHome;
        /// Which kind of neutral each neutral is.
        pub neutral: bota_core::NeutralKind;
        /// Which lane an entity belongs to.
        pub lane: Lane;
        /// Which of its lane's two barracks a building is.
        pub rax: Rax;
        /// Backdoor protection of the structures that have it.
        pub backdoor: Backdoor;
        /// What killing an entity pays.
        pub bounty: Bounty;
        /// Each Outpost's home side.
        pub outpost: crate::game::Outpost;
        /// The capture under way at each Outpost being captured.
        pub capture: crate::game::Capture;
        /// The standing-zone countdown of each entity that keeps one.
        pub zone: crate::game::Zone;
        /// Which seat owns an entity.
        pub owner: SlotId;
        /// Which hero a hero entity is.
        pub hero: HeroId;
        /// Which hero each entity that is an illusion shows as.
        pub illusion: crate::game::Illusion;
        /// What each entity carries.
        pub inventory: Inventory;
        /// What each entity can cast.
        pub abilities: AbilityBook;
        /// The stack each entity that is a ground item holds.
        pub loot: Loot;
        /// The item errand each entity is walking to carry out.
        pub handling: Handling;
        /// Which sides see each entity, worked out afresh every tick.
        pub visibility: Visibility;
        /// Sight each entity showed the sides it struck at.
        pub exposure: Exposure;
        /// Missiles in flight.
        pub projectile: Projectile;
    }

    derived {
        /// The numbers each entity fights by, worked out afresh every tick.
        pub stats: Stats;
        /// Which kind of unit each entity is.
        pub def: Def;
    }

    scratch {
        /// The forest and the ground of [`World::map`].
        pub terrain: &'static crate::game::MapTerrain = crate::game::terrain_of(bota_core::map_of(bota_proto::MapId(0)));
        /// The ground as bodies meet it: what stands where, and the room a body
        /// has anywhere.
        pub clearance: crate::game::Clearance = crate::game::Clearance::open();
        /// The scratch routes are searched in.
        pub planner: crate::game::Planner = crate::game::Planner::new();
        /// The height of the ground everywhere.
        pub ground: crate::game::Ground = crate::game::Ground::of(crate::game::terrain_of(bota_core::map_of(bota_proto::MapId(0))));
        /// Which cells stop a sight line: high ground, trees and the map's own
        /// walls.
        pub sight_block: bota_core::SightBlock = bota_core::SightBlock::of_ground(&|at| ground.tier(at));
        /// Blows felt this tick, for whatever answers to them.
        pub landed: VecDeque<Landed> = VecDeque::new();
        /// Attacks that missed this tick, for whatever answers to them.
        pub missed: VecDeque<Missed> = VecDeque::new();
        /// What abilities and items told of this tick. Drained by the tick.
        pub events: Vec<crate::game::Event> = Vec::new();
        /// The walked route of every lane, by team then lane, on the ground as
        /// it stood when laid. None since the ground last changed.
        pub lane_routes: Option<[[Vec<bota_proto::Vec2>; 3]; 2]> = None;
        /// Reused stable entity snapshot for systems that mutate other world tables.
        pub(crate) entity_scratch: Vec<Entity> = Vec::new();
        /// Whether this tick's systems take the entities from the last slot
        /// down rather than from the first up. Drawn afresh every tick.
        pub(crate) backwards: bool = false;
        /// Reused active-modifier buffer for systems that edit modifiers while
        /// reading them.
        pub(crate) modifier_scratch: Vec<Modifier> = Vec::new();
        /// Units whose timed life ran out this tick, for the expiry pass to take
        /// away. Filled and emptied inside one tick.
        pub(crate) lapsed_lives: Vec<Entity> = Vec::new();
        /// Reused buffers the sight system reads the world into.
        pub(crate) sight_scratch: SightScratch = SightScratch::new();
        /// Reused room targets are chosen among.
        pub(crate) candidates: Candidates = Candidates::default();
        /// Reused room auras are handed out in.
        pub(crate) aura_scratch: AuraScratch = AuraScratch::default();
        /// Reused room sleeping neutrals look for what wakes them in.
        pub(crate) jungle_spots: Spots<Entity> = Spots::default();
        /// Where every body stood at the start of this tick's walking.
        pub bodies: crate::game::BodyIndex = crate::game::BodyIndex::empty();
        /// The scratch local plans are searched in.
        pub local_scratch: crate::game::LocalScratch = crate::game::LocalScratch::new();
        /// Reused room for the bodies the index is laid from.
        pub(crate) body_scratch: Vec<crate::game::Body> = Vec::new();
        /// Reused room for the circles a stalled walker routes round.
        pub(crate) extra_scratch: Vec<(bota_proto::Vec2, bota_proto::Fixed)> = Vec::new();
    }
}

impl Default for World {
    fn default() -> Self {
        World::new()
    }
}

impl World {
    /// Starts timing a region of the tick, for the phase profile.
    pub(crate) fn scope(&self, phase: Phase) -> ScopeGuard {
        ScopeGuard::new(phase, self.tick, self.entities.len())
    }

    /// Where one entity puts another among those it cannot otherwise tell
    /// apart: a rank drawn from the match's seed for the pair of slots, the
    /// same every tick, so ties go to neither side by standing.
    pub(crate) fn tie_rank(&self, seeker: Entity, candidate: Entity) -> u64 {
        let seed = self.rng.seed();
        let mut key = u64::from_le_bytes([
            seed[0], seed[1], seed[2], seed[3], seed[4], seed[5], seed[6], seed[7],
        ]);
        for part in [seeker.index().0, candidate.index().0] {
            key = mix64(key ^ u64::from(part));
        }
        key
    }

    /// Takes a reusable stable snapshot of all entities currently standing,
    /// in the order this tick takes them.
    pub(crate) fn take_entity_snapshot(&mut self) -> Vec<Entity> {
        let mut entities = std::mem::take(&mut self.entity_scratch);
        assert!(entities.is_empty());
        entities.extend(self.entities.iter());
        if self.backwards {
            entities.reverse();
        }
        entities
    }

    /// Returns a consumed entity snapshot for reuse by the next system.
    pub(crate) fn recycle_entity_snapshot(&mut self, mut entities: Vec<Entity>) {
        entities.clear();
        assert!(self.entity_scratch.is_empty());
        self.entity_scratch = entities;
    }

    /// Adds an entity carrying no components.
    pub fn spawn(&mut self) -> Entity {
        self.entities.alloc()
    }

    /// Who an entity is set on, if it is set on anybody.
    pub fn target_of(&self, entity: Entity) -> Option<Entity> {
        self.target.get(entity).map(|Target(on)| *on)
    }

    /// Sets an entity on another.
    pub fn set_target(&mut self, entity: Entity, on: Entity) {
        self.target.insert(entity, Target(on));
    }

    /// Lays a blow `inflictor` deals on the queue, felt and forgotten inside
    /// this tick.
    pub fn push_hit(
        &mut self,
        source: Option<Entity>,
        target: Entity,
        amount: i32,
        kind: bota_proto::DamageKind,
        inflictor: Option<Inflictor>,
    ) {
        self.push_hit_with_effect(
            source,
            target,
            amount,
            kind,
            crate::game::HitEffect::None,
            inflictor,
        );
    }

    /// Leaves a blow carrying amplification captured by an in-flight source.
    pub(crate) fn push_hit_with_amp(
        &mut self,
        source: Option<Entity>,
        target: Entity,
        amount: i32,
        kind: bota_proto::DamageKind,
        damage_amp_bp: i32,
        inflictor: Option<Inflictor>,
    ) {
        self.hits.push_back(Hit {
            source,
            inflictor,
            target,
            amount,
            kind,
            damage_amp_bp,
            attack: false,
            striker: None,
            pierces: false,
            uphill: false,
            effect: crate::game::HitEffect::None,
        });
    }

    /// Lays a blow on the queue with an effect on top of its damage, which
    /// `inflictor` puts on as well.
    pub fn push_hit_with_effect(
        &mut self,
        source: Option<Entity>,
        target: Entity,
        amount: i32,
        kind: bota_proto::DamageKind,
        effect: crate::game::HitEffect,
        inflictor: Option<Inflictor>,
    ) {
        let damage_amp_bp = self.outgoing_damage_amp_bp(source, kind);
        self.hits.push_back(Hit {
            source,
            inflictor,
            target,
            amount,
            kind,
            damage_amp_bp,
            attack: false,
            striker: None,
            pierces: false,
            uphill: false,
            effect,
        });
    }

    /// Tells an entity what to do, leaving what it is waiting on alone.
    pub fn set_order(&mut self, entity: Entity, order: UnitOrder) {
        match self.orders.get_mut(entity) {
            Some(orders) => orders.current = order,
            None => {
                self.orders.insert(
                    entity,
                    Orders {
                        current: order,
                        cooldown: 0,
                        release_cooldown: 0,
                        pending: None,
                        immediate: None,
                    },
                );
            }
        }
    }

    /// Puts an entity on a side, and gives it its row of sight, seen by that
    /// side from this moment.
    pub fn set_team(&mut self, entity: Entity, team: Team) {
        self.team.insert(entity, team);
        match self.visibility.get_mut(entity) {
            Some(seen) => seen.add(team),
            None => {
                let mut seen = Visibility::NONE;
                seen.add(team);
                self.visibility.insert(entity, seen);
            }
        }
    }

    /// Takes an entity out of the world, with its row of sight and its applied
    /// stat changes. False when the handle named nobody live. Its other
    /// components stay in their tables under the old generation.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        self.visibility.remove(entity);
        self.applied.remove(entity);
        self.entities.free(entity)
    }

    /// A world with a map's buildings standing and their pools full.
    pub fn on_map(map: &'static bota_core::MapDef) -> World {
        World::on_terrain(map, crate::game::terrain_of(map))
    }

    /// [`World::on_map`] with a forest and a ground of its own.
    pub fn on_terrain(
        map: &'static bota_core::MapDef,
        terrain: &'static crate::game::MapTerrain,
    ) -> World {
        let mut world = World::new();
        world.map = map;
        world.terrain = terrain;
        world.clearance = crate::game::Clearance::of_map(map);
        world.ground = crate::game::Ground::of(terrain);
        world.trees = Forest::of(map, terrain);
        world.lay_sight_block();
        for (index, team) in [Team::Radiant, Team::Dire].into_iter().enumerate() {
            world.spawn_building(
                &bota_core::FOUNTAIN,
                team,
                map.fountains[index],
                Place::Fountain,
            );
            if let Some(at) = map.ancients[index] {
                world.spawn_building(bota_core::ancient_of(team), team, at, Place::Ancient);
            }
            let towers = if index == 0 {
                map.radiant_towers
            } else {
                map.dire_towers
            };
            for (lane, tier, pos) in towers {
                world.spawn_building(
                    bota_core::tower_def(*tier),
                    team,
                    *pos,
                    Place::Tower {
                        lane: *lane,
                        tier: *tier,
                    },
                );
            }
            for (lane, ranged, pos) in map.barracks[index] {
                let def = if *ranged {
                    &bota_core::BARRACKS_RANGED
                } else {
                    &bota_core::BARRACKS_MELEE
                };
                world.spawn_building(
                    def,
                    team,
                    *pos,
                    Place::Barracks {
                        lane: *lane,
                        ranged: *ranged,
                    },
                );
            }
            for pos in map.fillers[index] {
                world.spawn_building(&bota_core::FILLER, team, *pos, Place::Filler);
            }
        }
        world.settle();
        world.lay_passability();
        world
    }

    /// Lays what stands on the ground afresh: every standing structure and
    /// tree, at its collision size.
    pub fn lay_passability(&mut self) {
        let mut circles = Vec::new();
        for entity in self.entities.iter() {
            let Some(kind) = self.kind.get(entity).copied() else {
                continue;
            };
            if !bota_core::kind_def(kind).structure || !self.alive(entity) {
                continue;
            }
            let (Some(at), Some(hull)) = (self.transform.get(entity), self.hull.get(entity)) else {
                continue;
            };
            circles.push((at.pos, hull.collision));
        }
        let tree_radius = bota_core::rules::units(bota_core::rules::TREE_RADIUS);
        circles.extend(self.standing_trees().map(|at| (at, tree_radius)));
        self.clearance.set_circles(circles);
        self.lane_routes = None;
        let walkers = self.take_entity_snapshot();
        for entity in walkers.iter().copied() {
            self.forget_walk(entity);
        }
        self.recycle_entity_snapshot(walkers);
    }

    /// The walked route of every lane on the ground as it now stands, by
    /// team then lane: laid the first time it is asked for since the ground
    /// changed.
    pub fn walked_lanes(&mut self) -> &[[Vec<bota_proto::Vec2>; 3]; 2] {
        if self.lane_routes.is_none() {
            self.lay_lane_routes();
        }
        self.lane_routes.as_ref().expect("laid above")
    }

    /// Lays the lane routes on the ground as it stands, and puts every
    /// marcher at the waypoint of its new route nearest to where it is.
    fn lay_lane_routes(&mut self) {
        let routes = crate::game::lane_routes_on(self.map, &self.clearance, &mut self.planner);
        for entity in self.entities.iter() {
            let (Some(at), Some(team), Some(lane)) = (
                self.transform.get(entity).map(|t| t.pos),
                self.team.get(entity).copied(),
                self.lane.get(entity).copied(),
            ) else {
                continue;
            };
            let Some(march) = self.march.get_mut(entity) else {
                continue;
            };
            let route = &routes[bota_core::team_index(team)][usize::from(lane.0)];
            let nearest = route
                .iter()
                .enumerate()
                .min_by_key(|(_, spot)| at.distance_squared(**spot))
                .map_or(0, |(step, _)| step);
            march.next = nearest as u16;
        }
        self.lane_routes = Some(routes);
    }

    /// Lays out afresh which cells stop a sight line, from the forest as it
    /// now stands.
    pub fn lay_sight_block(&mut self) {
        self.sight_block = bota_core::SightBlock::lay(
            &|at| self.ground.tier(at),
            &crate::game::build_fow_walls(self.terrain),
            self.rooted_trees(),
            self.trees.planted().iter().map(|tree| tree.at),
        );
    }

    /// Every standing tree: the map's own, then the planted ones.
    fn standing_trees(&self) -> impl Iterator<Item = bota_proto::Vec2> + '_ {
        self.rooted_trees()
            .chain(self.trees.planted().iter().map(|tree| tree.at))
    }

    /// Every one of the map's own trees still standing, in
    /// [`tree_positions`] order.
    ///
    /// [`tree_positions`]: crate::game::tree_positions
    fn rooted_trees(&self) -> impl Iterator<Item = bota_proto::Vec2> + '_ {
        self.trees
            .spots()
            .iter()
            .enumerate()
            .filter(|(index, _)| self.trees.rooted_stands(*index))
            .map(|(_, at)| *at)
    }

    /// Brings stats, pools and who sees what into line with what stands.
    /// Called for a world just built and after anything is put into one.
    /// Pools follow their maxima as [`derive_stats`] moves them.
    pub fn settle(&mut self) {
        let _profile = self.scope(Phase::Settle);
        self.refresh_stats();
        self.refresh_sight();
        self.note_what_is_shown();
    }

    /// One tick, or no change after Map2 completes. The phases run in the
    /// order written, and applied stat changes run down last.
    pub fn step(&mut self) -> Vec<crate::game::Event> {
        if self.map2_finished() {
            return Vec::new();
        }
        let mut events = Vec::new();
        self.tick += 1;
        self.take_up_queues(&mut events);
        // Who acts first within a tick wins every race decided in it: a
        // blow, a step into a gap, a target taken. Slots go to the side
        // spawned first, and in Dota neither side wins by standing, so each
        // tick takes them up or down the slots by a coin.
        self.backwards = self.rng.global(Purpose::TickOrder).below(2) == 1;
        let _profile = self.scope(Phase::Tick);
        self.timed(Phase::Upkeep, World::step_upkeep);
        self.timed(Phase::Effects, World::step_effects);
        self.timed(Phase::Stats, World::refresh_stats);
        self.timed(Phase::Targeting, World::tick_targeting);
        self.timed(Phase::Movement, World::step_movement);
        self.timed(Phase::Visibility, World::refresh_sight);
        self.timed(Phase::Actions, |world| world.step_actions(&mut events));
        self.timed(Phase::Damage, |world| world.step_damage(&mut events));
        self.note_what_is_shown();
        self.tick_applied();
        events
    }

    /// Runs one region of the tick, timed as `phase` for the phase profile.
    fn timed(&mut self, phase: Phase, run: impl FnOnce(&mut World)) {
        let _profile = self.scope(phase);
        run(self);
    }

    /// Waves, camps, the Shrines, cooldowns, item builds, gold, shop stock,
    /// respawns, couriers, item errands, sales and what runs out.
    fn step_upkeep(&mut self) {
        self.spawn_waves();
        self.fill_camps();
        self.tick_shrines();
        self.tick_gear();
        self.assemble_bags();
        self.passive_gold();
        self.restock_shops();
        self.tick_respawns();
        self.tick_couriers();
        self.tick_handling();
        self.settle_sales();
        self.tick_expiries();
    }

    /// Modifiers, hooks, requiem lines, grenades, the forest, backdoor
    /// protection and auras.
    fn step_effects(&mut self) {
        self.tick_modifiers();
        self.tick_hooks();
        self.tick_requiem_lines();
        self.tick_grenades();
        self.tick_feasts();
        self.tick_forest();
        self.tick_backdoor();
        self.tick_auras();
        self.lift_fountain_invulnerability();
    }

    /// Stats worked out afresh, pools following their maxima, and the
    /// structures guarded.
    fn refresh_stats(&mut self) {
        let night = self.time_of_day().night;
        derive_stats(StatsCx {
            entities: &self.entities,
            def: &self.def,
            level: &self.level,
            upgrades: &self.upgrades,
            bonus: &self.stat_bonus,
            inventory: &self.inventory,
            modifiers: &self.modifiers,
            applied: &self.applied,
            abilities: &self.abilities,
            stats: &mut self.stats,
            health: &mut self.health,
            mana: &mut self.mana,
            night,
        });
        self.guard_structures();
        self.guard_cheated_heroes();
        self.wear_backdoor();
        self.wear_capture_guards();
    }

    /// The tick of the game clock: the tick, moved on by `-startgame`. Every
    /// schedule of the match (waves, camps, the passive gold, the day) runs
    /// on it.
    pub fn clock(&self) -> u32 {
        self.tick.saturating_add(self.clock_shift.game)
    }

    /// The time of day now, moved on by the night toggle.
    pub fn time_of_day(&self) -> bota_core::TimeOfDay {
        bota_core::time_of_day(self.clock().saturating_add(self.clock_shift.day))
    }

    /// Who sees what, from where everything stands.
    fn refresh_sight(&mut self) {
        let scroll_sights: Vec<(Team, Vec2)> = self.teleport_sights().collect();
        visibility_system(SightCx {
            entities: &self.entities,
            transform: &self.transform,
            team: &self.team,
            kind: &self.kind,
            stats: &self.stats,
            ground: &self.ground,
            sight_block: &self.sight_block,
            exposure: &self.exposure,
            scroll_sights: &scroll_sights,
            tick: self.tick,
            visibility: &mut self.visibility,
            sight: &mut self.sight_scratch,
        });
        if self.all_vision {
            self.share_vision();
        }
    }

    /// Notes what each side's views show as the tick ended: the forest
    /// where its eyes reach, and the level of every hero they show, for its
    /// scoreboard. Read only by the views.
    fn note_what_is_shown(&mut self) {
        self.judge_what_moved();
        let sight = &self.sight_scratch;
        let (ground, sight_block) = (&self.ground, &self.sight_block);
        for team in [Team::Radiant, Team::Dire] {
            let sees = |cell| sight.sees_cell(team, cell, ground, sight_block);
            self.trees.learn(
                team,
                |at| bota_core::tree_cells(at).any(sees),
                |at| bota_core::CellGrid::cell_of(at).is_some_and(sees),
            );
        }
        self.note_sightings();
    }

    /// Notes on each seat what every side shown its hero now saw of it, for
    /// that side's scoreboard.
    fn note_sightings(&mut self) {
        for index in 0..self.seats.len() {
            let Some(unit) = self.seats[index].unit else {
                continue;
            };
            for team in [Team::Radiant, Team::Dire] {
                let shown = self.shows(team, unit);
                let seat = &mut self.seats[index];
                if let (true, Some(side)) = (shown, bota_core::side_index(team)) {
                    seat.sighted[side] = crate::game::Sighting {
                        level: seat.level,
                        xp: seat.xp,
                        at: Some(self.tick),
                    };
                }
            }
        }
    }

    /// Where everything means to go, the steps taken there, and bodies
    /// eased apart.
    fn step_movement(&mut self) {
        let intent = self.scope(Phase::MovementIntent);
        self.tick_jungle();
        self.tick_neutral_spells();
        self.march_lanes();
        drop(intent);
        self.walk_bodies();
        self.push_apart();
    }

    /// Attack orders tended, pools regenerated, actions run, missiles flown; what they told of goes into the tick's events.
    fn step_actions(&mut self, events: &mut Vec<crate::game::Event>) {
        self.tend_attack_orders();
        regenerate(
            &self.entities,
            &self.stats,
            &mut self.health,
            &mut self.mana,
        );
        self.run_actions();
        missile_system(MissileCx {
            entities: &mut self.entities,
            projectile: &mut self.projectile,
            transform: &mut self.transform,
            team: &mut self.team,
            visibility: &mut self.visibility,
            health: &self.health,
            kind: &self.kind,
            ground: &self.ground,
            hits: &mut self.hits,
        });
        events.append(&mut self.events);
    }

    /// Puts the blows landing this tick in an order drawn afresh.
    ///
    /// They were queued in the order their dealers act, which is slot order,
    /// the side spawned first ahead: whichever blow is felt first takes a
    /// last hit both sides land in the same tick, and in Dota neither side
    /// wins such a race by standing.
    fn shuffle_hits(&mut self) {
        let hits = self.hits.make_contiguous();
        let order = self.rng.global(Purpose::HitOrder);
        for last in (1..hits.len()).rev() {
            let pick = order.below(u32::try_from(last + 1).unwrap_or(u32::MAX)) as usize;
            hits.swap(last, pick);
        }
    }

    /// Blows felt, then what answers to them: drinks and item cooldowns
    /// broken, camps roused, damage credited, events and misses told, the
    /// dead buried, and what their fall told of (a channel it broke off) put
    /// into the tick's events with the rest.
    fn step_damage(&mut self, events: &mut Vec<crate::game::Event>) {
        let mut felt = self.feel_hits();
        // What the blows that landed set off (a splash, a poison's first
        // beat) is felt in the same tick.
        self.land_strikes(&felt);
        if !self.hits.is_empty() {
            felt.extend(self.feel_hits());
        }
        self.burst_bubbles();
        self.break_on_blows(&felt);
        self.rouse_camps(&felt);
        self.owe_backdoor(&felt);
        self.credit_damage(&felt);
        self.tell_of(&felt, events);
        let missed: Vec<Missed> = self.missed.drain(..).collect();
        self.tell_of_misses(&missed, events);
        let fallen: Vec<(Entity, Option<Entity>)> = felt
            .iter()
            .filter(|blow| blow.fatal)
            .map(|blow| (blow.target, blow.source))
            .collect();
        self.avenge(&fallen);
        self.bury(fallen, events);
        events.append(&mut self.events);
    }

    /// Puts the waiting blows in an order drawn afresh and takes them off
    /// the health they land on; answers the ones that were felt.
    fn feel_hits(&mut self) -> Vec<Landed> {
        self.shuffle_hits();
        let amplified = self
            .hits
            .iter()
            .any(|hit| hit.damage_amp_bp != bota_core::rules::NOMINAL_BP);
        let cx = HitCx {
            hits: &mut self.hits,
            landed: &mut self.landed,
            transform: &self.transform,
            team: &self.team,
            stats: &self.stats,
            health: &mut self.health,
            modifiers: &mut self.modifiers,
            kind: &self.kind,
            immortal_structures: self.immortal_structures,
            immortal_heroes: self.immortal_heroes,
            rng: &self.rng,
            evasion: &mut self.evasion,
            missed: &mut self.missed,
        };
        if amplified {
            hitting_system::<true>(cx);
        } else {
            hitting_system::<false>(cx);
        }
        self.landed.drain(..).collect()
    }
}

/// SplitMix64's finaliser: every bit of the input moves every bit of the
/// output. Integer only, the same on every platform.
fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}
