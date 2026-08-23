//! The world: one table per component, and the order a tick runs in.

use std::collections::VecDeque;

use bota_proto::{HeroId, SlotId, Team, UnitKind};

use crate::game::{
    AbilityBook, AttackCx, Attacking, AuraCx, Auras, Bounty, CampHome, Def, Dismembering, Entity,
    EntityAllocator, Errand, Expiry, Forest, Health, Hit, Hook, Hull, Inventory, Landed, Lane,
    LaneAi, Level, Mana, March, NeutralAi, Orders, PendingCast, Projectile, Rotting, Route, Seat,
    SightCx, Stacks, Stats, StatsCx, Statuses, Table, Target, Teleport, Tier, Transform, UnitOrder,
    Upgrades, Visibility, attacking_system, aura_system, derive_stats, hitting_system,
    missile_system, regenerate, visibility_system,
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
    /// The side that has won, once one has.
    pub winner: Option<Team>,
    /// The map it is played on.
    pub map: &'static crate::game::MapDef,
    /// Where every roll of the dice comes from.
    pub rng: crate::game::MatchRng,
    /// Which cells anything may stand on.
    pub grid: crate::game::PassGrid,
    /// Which roster each camp put out last, so it never draws twice running.
    pub camp_last: Vec<u8>,
    /// The height of the ground everywhere.
    pub ground: crate::game::Ground,
    /// Which cells stop a sight line: trees and the map's own walls.
    pub sight_block: crate::game::PassGrid,
    /// The forest as it stands: what is down, and what has been put up.
    pub trees: Forest,
    /// Blows dealt and not yet felt. Filled and emptied inside one tick.
    pub hits: VecDeque<Hit>,
    /// Blows felt this tick, for whatever answers to them.
    pub landed: VecDeque<Landed>,
    /// Missiles that arrived with a bounce still in them, beside what they
    /// arrived on.
    pub bounced: VecDeque<(Entity, Entity)>,
    /// Which entities exist.
    pub entities: EntityAllocator,

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
    /// What is on each entity and runs out on its own.
    pub statuses: Table<Statuses>,
    /// The hook each entity that is one is flying.
    pub hook: Table<Hook>,
    /// The rot each entity has switched on.
    pub rotting: Table<Rotting>,
    /// The dismember each entity is channelling.
    pub dismember: Table<Dismembering>,
    /// What each entity has gathered and keeps.
    pub stacks: Table<Stacks>,
    /// The errand each courier is on.
    pub errand: Table<Errand>,
    /// How long each entity that stands for a time has left.
    pub expiry: Table<Expiry>,
    /// The teleport each entity is channelling.
    pub teleport: Table<Teleport>,
    /// What each entity hands out to those standing near it.
    pub auras: Table<Auras>,

    /// Routes being walked by whoever a player drives.
    pub route: Table<Route>,
    /// What a creep keeps while marching its lane.
    pub march: Table<March>,

    /// The order each entity is following.
    pub orders: Table<Orders>,
    /// Who each entity is set on. Absent when it is set on nobody.
    pub target: Table<Target>,
    /// Where each entity is in its attack cycle.
    pub attacking: Table<Attacking>,
    /// Casts ordered and not yet started.
    pub casting: Table<PendingCast>,

    /// What a lane creep keeps about the fight it is in.
    pub lane_ai: Table<LaneAi>,
    /// What a neutral keeps about being drawn away.
    pub neutral_ai: Table<NeutralAi>,
    /// Which camp a neutral belongs to.
    pub camp_home: Table<CampHome>,

    /// Which lane an entity belongs to.
    pub lane: Table<Lane>,
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
            map: crate::game::map_of(bota_proto::MapId(0)),
            rng: crate::game::MatchRng::new(&[0; 32], 0),
            grid: crate::game::PassGrid::open(),
            camp_last: Vec::new(),
            ground: crate::game::Ground::of(crate::game::map_of(bota_proto::MapId(0))),
            sight_block: crate::game::PassGrid::open(),
            trees: Forest::default(),
            hits: VecDeque::new(),
            landed: VecDeque::new(),
            bounced: VecDeque::new(),
            entities: EntityAllocator::new(),
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
            statuses: Table::new(),
            hook: Table::new(),
            rotting: Table::new(),
            dismember: Table::new(),
            stacks: Table::new(),
            errand: Table::new(),
            expiry: Table::new(),
            teleport: Table::new(),
            auras: Table::new(),
            route: Table::new(),
            march: Table::new(),
            orders: Table::new(),
            target: Table::new(),
            attacking: Table::new(),
            casting: Table::new(),
            lane_ai: Table::new(),
            neutral_ai: Table::new(),
            camp_home: Table::new(),
            lane: Table::new(),
            bounty: Table::new(),
            owner: Table::new(),
            hero: Table::new(),
            inventory: Table::new(),
            abilities: Table::new(),
            visibility: Table::new(),
            projectile: Table::new(),
        }
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
        world.grid = crate::game::build_grid(map);
        world.ground = crate::game::Ground::of(map);
        world.trees = Forest::of(map);
        world.sight_block = crate::game::build_sight_block(map);
        for (index, team) in [Team::Radiant, Team::Dire].into_iter().enumerate() {
            world.spawn_unit(&crate::game::FOUNTAIN, team, map.fountains[index]);
            world.spawn_unit(&crate::game::ANCIENT, team, map.ancients[index]);
            let towers = if index == 0 {
                map.radiant_towers
            } else {
                map.dire_towers
            };
            for (lane, tier, pos) in towers {
                let entity = world.spawn_unit(crate::game::tower_def(*tier), team, *pos);
                world.lane.insert(entity, Lane(*lane));
                world.tier.insert(entity, Tier(*tier));
            }
        }
        world.settle();
        world
    }

    /// Lays out afresh which cells stop a sight line, from the forest as it
    /// now stands.
    pub fn lay_sight_block(&mut self) {
        let mut grid = crate::game::build_fow_walls(self.map);
        let close = |grid: &mut crate::game::PassGrid, at| {
            if let Some((cx, cy)) = crate::game::PassGrid::cell_of(at) {
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
            statuses: &self.statuses,
            abilities: &self.abilities,
            stacks: &self.stacks,
            stats: &mut self.stats,
            health: &mut self.health,
            mana: &mut self.mana,
        });
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

    /// One tick. Systems run in the order they are written here.
    pub fn step(&mut self) -> Vec<crate::game::Event> {
        let mut events = Vec::new();
        self.tick += 1;
        self.spawn_waves();
        self.fill_camps();
        self.tick_gear();
        self.assemble_bags();
        self.passive_gold();
        self.tick_respawns();
        self.tick_couriers();
        self.tick_teleports();
        self.tick_expiries();
        self.tick_burning();
        self.tick_hooks();
        self.tick_rot();
        self.tick_dismembers();
        if self.trees.tick(self.tick) {
            self.lay_sight_block();
        }
        aura_system(AuraCx {
            entities: &self.entities,
            transform: &self.transform,
            team: &self.team,
            auras: &self.auras,
            statuses: &mut self.statuses,
        });
        derive_stats(StatsCx {
            entities: &self.entities,
            def: &self.def,
            level: &self.level,
            upgrades: &self.upgrades,
            inventory: &self.inventory,
            statuses: &self.statuses,
            abilities: &self.abilities,
            stacks: &self.stacks,
            stats: &mut self.stats,
            health: &mut self.health,
            mana: &mut self.mana,
        });
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
        regenerate(
            &self.entities,
            &self.stats,
            &mut self.health,
            &mut self.mana,
        );
        attacking_system(AttackCx {
            entities: &mut self.entities,
            transform: &mut self.transform,
            hull: &self.hull,
            kind: &self.kind,
            team: &mut self.team,
            health: &self.health,
            stats: &self.stats,
            visibility: &mut self.visibility,
            target: &self.target,
            statuses: &self.statuses,
            attacking: &mut self.attacking,
            hits: &mut self.hits,
            projectile: &mut self.projectile,
        });
        missile_system(MissileCx {
            entities: &mut self.entities,
            projectile: &mut self.projectile,
            transform: &mut self.transform,
            team: &mut self.team,
            visibility: &mut self.visibility,
            health: &self.health,
            hits: &mut self.hits,
            bounced: &mut self.bounced,
        });
        self.bounce_missiles();
        self.run_casts(&mut events);
        hitting_system(HitCx {
            hits: &mut self.hits,
            landed: &mut self.landed,
            transform: &self.transform,
            team: &self.team,
            stats: &self.stats,
            health: &mut self.health,
        });
        let felt: Vec<Landed> = self.landed.drain(..).collect();
        self.break_on_blows(&felt);
        self.rouse_camps(&felt);
        self.tell_of(&felt, &mut events);
        let fallen = felt
            .iter()
            .filter(|blow| blow.fatal)
            .map(|blow| (blow.target, blow.source))
            .collect();
        self.bury(fallen, &mut events);
        events
    }
}
