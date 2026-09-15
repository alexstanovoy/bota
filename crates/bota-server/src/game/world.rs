//! The world: one table per component, and the order a tick runs in.

use std::collections::VecDeque;

use bota_proto::{HeroId, SlotId, Team, UnitKind};

use crate::game::{
    AbilityBook, Action, AuraCx, Auras, Bounty, CampHome, Def, Entity, EntityAllocator, Errand,
    Expiry, Forest, Handling, Health, Hit, Hook, Hull, Inventory, Landed, Lane, LaneAi, Level,
    Loot, Mana, March, Mark, Missed, Modifiers, Motion, NeutralAi, Orders, Place, Plan, Projectile,
    Rax, RequiemLine, Route, Seat, SightCx, Stacks, Stats, StatsCx, Table, Target, Tier, Transform,
    UnitOrder, Upgrades, Visibility, aura_system, derive_stats, hitting_system, missile_system,
    regenerate, visibility_system,
};
use crate::game::{HitCx, MissileCx};

/// Everything a match is made of.
///
/// Every component lives in a table of its own, and a table is absent for an
/// entity the component does not apply to. A system takes the tables it needs
/// and nothing else; [`World::step`] is the one place the order of a tick is
/// written down.
pub struct World {
    /// Ticks since the match began.
    pub tick: u32,
    /// A place per player.
    pub seats: Vec<Seat>,
    /// The completed result; `Team::Neutral` denotes a Map2 draw.
    pub winner: Option<Team>,
    /// Whether cheat orders are honoured.
    pub cheats: bool,
    /// The map it is played on.
    pub map: &'static crate::game::MapDef,
    /// Where every roll of the dice comes from.
    pub rng: crate::game::MatchRng,
    /// The ground as bodies meet it: what stands where, and the room a body
    /// has anywhere.
    pub clearance: crate::game::Clearance,
    /// The scratch routes are searched in.
    pub planner: crate::game::Planner,
    /// Uphill miss sequences, indexed by attacker's entity slot.
    pub uphill_miss: Vec<Option<crate::game::PseudoRandom25>>,
    /// Critical strike sequences, indexed by attacker's entity slot.
    pub crit: Vec<Option<crate::game::Chance>>,
    /// Evasion sequences, indexed by target's entity slot.
    pub evasion: Vec<Option<crate::game::Chance>>,
    /// Pierce sequences, indexed by attacker's entity slot.
    pub pierce: Vec<Option<crate::game::Chance>>,
    /// Which roster each camp put out last, so it never draws twice running.
    pub camp_last: Vec<u8>,
    /// The height of the ground everywhere.
    pub ground: crate::game::Ground,
    /// Which cells stop a sight line: trees and the map's own walls.
    pub sight_block: crate::game::CellGrid,
    /// The forest as it stands: what is down, and what has been put up.
    pub trees: Forest,
    /// Blows dealt and not yet felt. Filled and emptied inside one tick.
    pub hits: VecDeque<Hit>,
    /// Blows felt this tick, for whatever answers to them.
    pub landed: VecDeque<Landed>,
    /// Attacks that missed this tick, for whatever answers to them.
    pub missed: VecDeque<Missed>,
    /// Missiles that arrived with a bounce still in them, beside what they
    /// arrived on.
    pub bounced: VecDeque<(Entity, Entity)>,
    /// What abilities and items told of this tick. Drained by the tick.
    pub events: Vec<crate::game::Event>,
    /// The walked route of every lane, by team then lane, on the ground as
    /// it stood when laid. None since the ground last changed.
    pub lane_routes: Option<[[Vec<bota_proto::Vec2>; 3]; 2]>,
    /// Which entities exist.
    pub entities: EntityAllocator,
    /// Reused stable entity snapshot for systems that mutate other world tables.
    pub(crate) entity_scratch: Vec<Entity>,

    /// Where each entity stands.
    pub transform: Table<Transform>,
    /// The room each entity takes.
    pub hull: Table<Hull>,
    /// What kind of thing each entity is.
    pub kind: Table<UnitKind>,
    /// Which side each entity is on.
    pub team: Table<Team>,

    /// Health, for whatever can be hurt.
    pub health: Table<Health>,
    /// Mana, for whatever spends it.
    pub mana: Table<Mana>,

    /// Which kind of unit each entity is.
    pub def: Table<Def>,
    /// Hero levels.
    pub level: Table<Level>,
    /// Upgrade intervals a creep spawned after.
    pub upgrades: Table<Upgrades>,
    /// Building tiers.
    pub tier: Table<Tier>,
    /// The numbers each entity fights by, worked out afresh every tick.
    pub stats: Table<Stats>,
    /// What is on each entity.
    pub modifiers: Table<Modifiers>,
    /// The hook each entity that is one is flying.
    pub hook: Table<Hook>,
    /// What each entity that is an ability's mark shows.
    pub mark: Table<Mark>,
    /// The line of a requiem each entity that is one is flying.
    pub requiem_line: Table<RequiemLine>,
    /// What each entity has gathered and keeps.
    pub stacks: Table<Stacks>,
    /// The errand each courier is on.
    pub errand: Table<Errand>,
    /// How long each entity that stands for a time has left.
    pub expiry: Table<Expiry>,
    /// What each entity hands out to those standing near it.
    pub auras: Table<Auras>,

    /// The route each walker is on.
    pub route: Table<Route>,
    /// The next stretch of each walker's walk.
    pub plan: Table<Plan>,
    /// How each walker has been moving.
    pub motion: Table<Motion>,
    /// What a creep keeps while marching its lane.
    pub march: Table<March>,
    /// Where every body stood at the start of this tick's walking.
    pub bodies: crate::game::BodyIndex,
    /// The scratch local plans are searched in.
    pub local_scratch: crate::game::LocalScratch,
    /// Reused room for the bodies the index is laid from.
    pub(crate) body_scratch: Vec<crate::game::Body>,
    /// Reused room for the circles a stalled walker routes round.
    pub(crate) extra_scratch: Vec<(bota_proto::Vec2, bota_proto::Fixed)>,

    /// The order each entity is following.
    pub orders: Table<Orders>,
    /// Who each entity is set on. Absent when it is set on nobody.
    pub target: Table<Target>,
    /// What each entity is doing.
    pub action: Table<Action>,

    /// What a lane creep keeps about the fight it is in.
    pub lane_ai: Table<LaneAi>,
    /// What a neutral keeps about being drawn away.
    pub neutral_ai: Table<NeutralAi>,
    /// Which camp a neutral belongs to.
    pub camp_home: Table<CampHome>,

    /// Which lane an entity belongs to.
    pub lane: Table<Lane>,
    /// Which of its lane's two barracks a building is.
    pub rax: Table<Rax>,
    /// What killing an entity pays.
    pub bounty: Table<Bounty>,
    /// Which seat owns an entity.
    pub owner: Table<SlotId>,
    /// Which hero a hero entity is.
    pub hero: Table<HeroId>,
    /// What each entity carries.
    pub inventory: Table<Inventory>,
    /// What each entity can cast.
    pub abilities: Table<AbilityBook>,
    /// The stack each entity that is a ground item holds.
    pub loot: Table<Loot>,
    /// The item errand each entity is walking to carry out.
    pub handling: Table<Handling>,

    /// Which sides see each entity, worked out afresh every tick.
    pub visibility: Table<Visibility>,

    /// Missiles in flight.
    pub projectile: Table<Projectile>,
}

impl Default for World {
    fn default() -> Self {
        World::new()
    }
}

impl World {
    /// A world at tick zero with nothing in it, on the Dota map.
    pub fn new() -> World {
        World {
            tick: 0,
            seats: Vec::new(),
            winner: None,
            cheats: false,
            map: crate::game::map_of(bota_proto::MapId(0)),
            rng: crate::game::MatchRng::new(&[0; 32], 0),
            clearance: crate::game::Clearance::open(),
            planner: crate::game::Planner::new(),
            uphill_miss: Vec::new(),
            crit: Vec::new(),
            evasion: Vec::new(),
            pierce: Vec::new(),
            camp_last: Vec::new(),
            ground: crate::game::Ground::of(crate::game::map_of(bota_proto::MapId(0))),
            sight_block: crate::game::CellGrid::open(),
            trees: Forest::default(),
            hits: VecDeque::new(),
            landed: VecDeque::new(),
            missed: VecDeque::new(),
            bounced: VecDeque::new(),
            events: Vec::new(),
            lane_routes: None,
            entities: EntityAllocator::new(),
            entity_scratch: Vec::new(),
            transform: Table::new(),
            hull: Table::new(),
            kind: Table::new(),
            team: Table::new(),
            health: Table::new(),
            mana: Table::new(),
            def: Table::new(),
            level: Table::new(),
            upgrades: Table::new(),
            tier: Table::new(),
            stats: Table::new(),
            modifiers: Table::new(),
            hook: Table::new(),
            mark: Table::new(),
            requiem_line: Table::new(),
            stacks: Table::new(),
            errand: Table::new(),
            expiry: Table::new(),
            auras: Table::new(),
            route: Table::new(),
            plan: Table::new(),
            motion: Table::new(),
            bodies: crate::game::BodyIndex::empty(),
            local_scratch: crate::game::LocalScratch::new(),
            body_scratch: Vec::new(),
            extra_scratch: Vec::new(),
            march: Table::new(),
            orders: Table::new(),
            target: Table::new(),
            action: Table::new(),
            lane_ai: Table::new(),
            neutral_ai: Table::new(),
            camp_home: Table::new(),
            lane: Table::new(),
            rax: Table::new(),
            bounty: Table::new(),
            owner: Table::new(),
            hero: Table::new(),
            inventory: Table::new(),
            abilities: Table::new(),
            loot: Table::new(),
            handling: Table::new(),
            visibility: Table::new(),
            projectile: Table::new(),
        }
    }

    /// Takes a reusable stable snapshot of all entities currently standing.
    pub(crate) fn take_entity_snapshot(&mut self) -> Vec<Entity> {
        let mut entities = std::mem::take(&mut self.entity_scratch);
        assert!(entities.is_empty());
        entities.extend(self.entities.iter());
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

    /// Leaves a blow for the next tick of resolving to take off somebody.
    ///
    /// Lays a blow on the queue for this tick.
    ///
    /// A blow is a message rather than a thing standing on the map: it is
    /// felt and forgotten inside the tick that dealt it.
    pub fn push_hit(
        &mut self,
        source: Option<Entity>,
        target: Entity,
        amount: i32,
        kind: bota_proto::DamageKind,
    ) {
        self.hits.push_back(Hit {
            source,
            target,
            amount,
            kind,
            crit: false,
            attack: false,
            pierces: false,
            effect: crate::game::HitEffect::None,
        });
    }

    /// Tells an entity what to do, leaving what it is waiting on alone.
    ///
    /// The order and the wait before it may be re-aimed live in one component;
    /// writing that component whole is how the wait gets lost.
    pub fn set_order(&mut self, entity: Entity, order: UnitOrder) {
        match self.orders.get_mut(entity) {
            Some(orders) => orders.current = order,
            None => {
                self.orders.insert(
                    entity,
                    Orders {
                        current: order,
                        cooldown: 0,
                        pending: None,
                    },
                );
            }
        }
    }

    /// Puts an entity on a side.
    ///
    /// Standing on a side is what makes an entity something sides can see, so
    /// its row in the sight table is made here and nowhere else. That side has
    /// it from this moment rather than from the next pass of sight, which
    /// matters for whatever is stood up mid-tick.
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

    /// Takes an entity out of the world. False when the handle named nobody
    /// live.
    ///
    /// What sides could see of it is given up here. What it held besides stays
    /// where it is; the slot's next tenant carries a raised generation, so none
    /// of it reads back as that tenant's own.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        self.visibility.remove(entity);
        self.entities.free(entity)
    }

    /// A world with a map's buildings standing and their pools full.
    pub fn on_map(map: &'static crate::game::MapDef) -> World {
        let mut world = World::new();
        world.map = map;
        world.clearance = crate::game::Clearance::of_map(map);
        world.ground = crate::game::Ground::of(map);
        world.trees = Forest::of(map);
        world.sight_block = crate::game::build_sight_block(map);
        for (index, team) in [Team::Radiant, Team::Dire].into_iter().enumerate() {
            world.spawn_building(
                &crate::game::FOUNTAIN,
                team,
                map.fountains[index],
                Place::Fountain,
            );
            if let Some(at) = map.ancients[index] {
                world.spawn_building(crate::game::ancient_of(team), team, at, Place::Ancient);
            }
            let towers = if index == 0 {
                map.radiant_towers
            } else {
                map.dire_towers
            };
            for (lane, tier, pos) in towers {
                world.spawn_building(
                    crate::game::tower_def(*tier),
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
                    &crate::game::BARRACKS_RANGED
                } else {
                    &crate::game::BARRACKS_MELEE
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
            if !crate::game::is_structure(kind) || !self.alive(entity) {
                continue;
            }
            let (Some(at), Some(hull)) = (self.transform.get(entity), self.hull.get(entity)) else {
                continue;
            };
            circles.push((at.pos, hull.collision));
        }
        let tree_radius = crate::game::rules::units(crate::game::rules::TREE_RADIUS);
        for (index, at) in crate::game::tree_positions(self.map)
            .into_iter()
            .enumerate()
        {
            if self.trees.rooted_stands(index) {
                circles.push((at, tree_radius));
            }
        }
        for tree in self.trees.planted() {
            circles.push((tree.at, tree_radius));
        }
        self.clearance.set_circles(circles);
        self.lane_routes = None;
        let walkers: Vec<Entity> = self.entities.iter().collect();
        for entity in walkers {
            self.forget_walk(entity);
        }
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
            let route = &routes[crate::game::team_index(team)][usize::from(lane.0)];
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
        let mut grid = crate::game::build_fow_walls(self.map);
        let close = |grid: &mut crate::game::CellGrid, at| {
            if let Some((cx, cy)) = crate::game::CellGrid::cell_of(at) {
                grid.close_cell(cx, cy);
            }
        };
        for (index, at) in crate::game::tree_positions(self.map)
            .into_iter()
            .enumerate()
        {
            if self.trees.rooted_stands(index) {
                close(&mut grid, at);
            }
        }
        for tree in self.trees.planted() {
            close(&mut grid, tree.at);
        }
        self.sight_block = grid;
    }

    /// Brings everything worked out into line with what stands: stats and who
    /// sees what.
    ///
    /// Called for a world just built and after anything is put into one, so
    /// nothing newly stood up is invisible until the next tick. Pools are
    /// left alone: what stands with them is whatever it has left, and filling
    /// them is the business of whoever stood the entity up.
    pub fn settle(&mut self) {
        derive_stats(StatsCx {
            entities: &self.entities,
            def: &self.def,
            level: &self.level,
            upgrades: &self.upgrades,
            inventory: &self.inventory,
            modifiers: &self.modifiers,
            abilities: &self.abilities,
            stacks: &self.stacks,
            stats: &mut self.stats,
            health: &mut self.health,
            mana: &mut self.mana,
        });
        self.guard_structures();
        visibility_system(SightCx {
            entities: &self.entities,
            transform: &self.transform,
            team: &self.team,
            kind: &self.kind,
            stats: &self.stats,
            ground: &self.ground,
            sight_block: &self.sight_block,
            visibility: &mut self.visibility,
        });
    }

    /// One tick, or no change after Map2 completes. Systems run in written order.
    pub fn step(&mut self) -> Vec<crate::game::Event> {
        if self.map2_finished() {
            return Vec::new();
        }
        let mut events = Vec::new();
        self.tick += 1;
        self.spawn_waves();
        self.fill_camps();
        self.tick_gear();
        self.assemble_bags();
        self.passive_gold();
        self.tick_respawns();
        self.tick_couriers();
        self.tick_handling();
        self.settle_sales();
        self.tick_expiries();
        self.tick_modifiers();
        self.tick_hooks();
        self.tick_requiem_lines();
        if self.trees.tick(self.tick) {
            self.lay_sight_block();
            self.lay_passability();
        }
        self.spread_presence();
        aura_system(AuraCx {
            entities: &self.entities,
            transform: &self.transform,
            team: &self.team,
            kind: &self.kind,
            auras: &self.auras,
            modifiers: &mut self.modifiers,
        });
        derive_stats(StatsCx {
            entities: &self.entities,
            def: &self.def,
            level: &self.level,
            upgrades: &self.upgrades,
            inventory: &self.inventory,
            modifiers: &self.modifiers,
            abilities: &self.abilities,
            stacks: &self.stacks,
            stats: &mut self.stats,
            health: &mut self.health,
            mana: &mut self.mana,
        });
        self.guard_structures();
        self.step_combat(&mut events);
        events
    }

    fn step_combat(&mut self, events: &mut Vec<crate::game::Event>) {
        self.tick_targeting();
        self.tick_jungle();
        self.march_lanes();
        self.walk_bodies();
        self.push_apart();
        visibility_system(SightCx {
            entities: &self.entities,
            transform: &self.transform,
            team: &self.team,
            kind: &self.kind,
            stats: &self.stats,
            ground: &self.ground,
            sight_block: &self.sight_block,
            visibility: &mut self.visibility,
        });
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
            rng: &self.rng,
            uphill_miss: &mut self.uphill_miss,
            hits: &mut self.hits,
            bounced: &mut self.bounced,
            missed: &mut self.missed,
        });
        self.bounce_missiles();
        events.append(&mut self.events);
        self.step_damage(events);
    }

    fn step_damage(&mut self, events: &mut Vec<crate::game::Event>) {
        hitting_system(HitCx {
            hits: &mut self.hits,
            landed: &mut self.landed,
            transform: &self.transform,
            team: &self.team,
            stats: &self.stats,
            health: &mut self.health,
            modifiers: &mut self.modifiers,
            rng: &self.rng,
            evasion: &mut self.evasion,
            missed: &mut self.missed,
        });
        let felt: Vec<Landed> = self.landed.drain(..).collect();
        self.break_on_blows(&felt);
        self.rouse_camps(&felt);
        self.tell_of(&felt, events);
        let missed: Vec<Missed> = self.missed.drain(..).collect();
        self.tell_of_misses(&missed, events);
        let fallen = felt
            .iter()
            .filter(|blow| blow.fatal)
            .map(|blow| (blow.target, blow.source))
            .collect();
        self.bury(fallen, events);
    }
}
