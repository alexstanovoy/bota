//! Putting an entity into the world with everything its kind needs.

use bota_proto::{Angle, Fixed, HeroId, SlotId, Team, Vec2};

use crate::game::{
    Action, ActionState, Auras, Bounty, CampHome, Def, Entity, Errand, Expiry, Health, Hull,
    Inventory, Lane, LaneAi, Level, Mana, March, Modifiers, NeutralAi, Orders, Rax, Tier,
    Transform, UnitDef, UnitOrder, Upgrades, World, rules,
};

/// Which building of a side one is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Fountain,
    Ancient,
    Tower { lane: u8, tier: u8 },
    Barracks { lane: u8, ranged: bool },
}

impl World {
    /// What every kind stands on: where it is, what it is, whose it is, and
    /// the pools it fills once its stats are known.
    ///
    /// Health and mana start empty; the system that works out stats fills
    /// them on the tick after, since the maximum is its to decide.
    fn spawn_body(&mut self, def: &'static UnitDef, team: Team, pos: Vec2) -> Entity {
        let entity = self.spawn();
        self.def.insert(entity, Def(def));
        self.kind.insert(entity, def.kind);
        self.set_team(entity, team);
        self.transform.insert(
            entity,
            Transform {
                pos,
                facing: Angle::default(),
            },
        );
        self.health.insert(entity, Health { hp: Fixed::ZERO });
        self.modifiers.insert(entity, Modifiers(Vec::new()));
        if def.max_mana > 0 {
            self.mana.insert(entity, Mana { mana: Fixed::ZERO });
        }
        if def.bounty_gold > 0 || def.bounty_xp > 0 {
            self.bounty.insert(
                entity,
                Bounty {
                    gold: def.bounty_gold,
                    xp: def.bounty_xp,
                },
            );
        }
        if !def.auras.is_empty() {
            self.auras.insert(entity, Auras(def.auras));
        }
        entity
    }

    /// The room a body takes on the ground.
    fn give_hull(&mut self, entity: Entity, def: &UnitDef) {
        self.hull.insert(
            entity,
            Hull {
                radius: Fixed::from_int(def.radius),
            },
        );
    }

    /// A body that does things, standing ready.
    fn give_action(&mut self, entity: Entity) {
        self.action.insert(
            entity,
            Action {
                state: ActionState::Ready,
                attack_cooldown: 0,
            },
        );
    }

    /// A body that takes orders, told nothing yet.
    fn give_orders(&mut self, entity: Entity) {
        self.orders.insert(
            entity,
            Orders {
                current: UnitOrder::Idle,
                cooldown: 0,
                pending: None,
            },
        );
    }

    /// Puts a hero on the map for a seat.
    pub fn spawn_hero(&mut self, team: Team, pos: Vec2, slot: SlotId, hero: HeroId) -> Entity {
        let body = crate::game::hero_def(hero).map_or(&crate::game::HERO, |def| def.unit);
        let entity = self.spawn_body(body, team, pos);
        self.give_hull(entity, body);
        self.give_action(entity);
        self.give_orders(entity);
        self.owner.insert(entity, slot);
        self.hero.insert(entity, hero);
        self.level.insert(entity, Level(1));
        self.abilities.insert(entity, crate::game::hero_kit(hero));
        self.inventory.insert(
            entity,
            Inventory::empty(rules::INVENTORY_SLOTS + rules::BACKPACK_SLOTS),
        );
        entity
    }

    /// Puts a lane creep on the map, to march its lane from where it stands.
    pub fn spawn_creep(
        &mut self,
        def: &'static UnitDef,
        team: Team,
        pos: Vec2,
        lane: u8,
        upgrades: u32,
    ) -> Entity {
        let entity = self.spawn_body(def, team, pos);
        self.give_hull(entity, def);
        self.give_action(entity);
        self.give_orders(entity);
        self.lane.insert(entity, Lane(lane));
        self.upgrades.insert(entity, Upgrades(upgrades));
        self.march.insert(
            entity,
            March {
                route_step: 0,
                trace: None,
                shove: 0,
            },
        );
        self.lane_ai.insert(
            entity,
            LaneAi {
                anchor: None,
                last_seen: None,
                keep_until: 0,
                roused_by: None,
                roused_at_own: false,
                chase_until: 0,
            },
        );
        entity
    }

    /// Puts a neutral on the map at its camp, asleep.
    pub fn spawn_neutral(
        &mut self,
        def: &'static UnitDef,
        pos: Vec2,
        camp: u8,
        upgrades: u32,
    ) -> Entity {
        let entity = self.spawn_body(def, Team::Neutral, pos);
        self.give_hull(entity, def);
        self.give_action(entity);
        self.give_orders(entity);
        self.upgrades.insert(entity, Upgrades(upgrades));
        self.camp_home.insert(entity, CampHome { camp, home: pos });
        self.neutral_ai.insert(
            entity,
            NeutralAi {
                leash_left: rules::NEUTRAL_AGGRO_WINDOW,
                reaggro_block: 0,
                next_window: rules::NEUTRAL_AGGRO_WINDOW,
                going_home: false,
                roused_by: None,
                awake: false,
            },
        );
        entity
    }

    /// Puts a building on the map at its place.
    pub fn spawn_building(
        &mut self,
        def: &'static UnitDef,
        team: Team,
        pos: Vec2,
        place: Place,
    ) -> Entity {
        let entity = self.spawn_body(def, team, pos);
        self.give_hull(entity, def);
        self.give_action(entity);
        match place {
            Place::Fountain | Place::Ancient => {}
            Place::Tower { lane, tier } => {
                self.lane.insert(entity, Lane(lane));
                self.tier.insert(entity, Tier(tier));
            }
            Place::Barracks { lane, ranged } => {
                self.lane.insert(entity, Lane(lane));
                self.rax.insert(entity, Rax { ranged });
            }
        }
        entity
    }

    /// Puts a courier on the map for a seat, carrying what it is handed.
    pub fn spawn_courier(
        &mut self,
        team: Team,
        pos: Vec2,
        slot: SlotId,
        load: Inventory,
    ) -> Entity {
        let entity = self.spawn_body(&crate::game::COURIER, team, pos);
        self.give_action(entity);
        self.give_orders(entity);
        self.owner.insert(entity, slot);
        self.inventory.insert(entity, load);
        self.abilities.insert(
            entity,
            crate::game::AbilityBook {
                slots: [
                    crate::game::ability::TAKE_STASH,
                    crate::game::ability::RETURN_ITEMS,
                    crate::game::ability::BURST,
                    crate::game::ability::DELIVER,
                    crate::game::ability::SHIELD,
                ]
                .into_iter()
                .map(|id| crate::game::AbilityState {
                    id,
                    level: 1,
                    cooldown: 0,
                })
                .collect(),
            },
        );
        self.errand.insert(entity, Errand::None);
        entity
    }

    /// Stands a ward at a spot for so many ticks.
    pub fn spawn_ward(
        &mut self,
        def: &'static UnitDef,
        team: Team,
        pos: Vec2,
        ticks: u32,
    ) -> Entity {
        let entity = self.spawn_body(def, team, pos);
        self.expiry.insert(entity, Expiry { ticks_left: ticks });
        entity
    }

    /// Puts one unit of any kind on the map with what its numbers call for:
    /// a hull for a radius, an action for damage, orders for a speed, a march
    /// for a lane creep kind.
    #[cfg(test)]
    pub fn spawn_unit(&mut self, def: &'static UnitDef, team: Team, pos: Vec2) -> Entity {
        let entity = self.spawn_body(def, team, pos);
        if def.radius > 0 {
            self.give_hull(entity, def);
        }
        if def.damage > 0 {
            self.give_action(entity);
        }
        if def.move_speed > 0 {
            self.give_orders(entity);
        }
        if matches!(
            def.kind,
            bota_proto::UnitKind::CreepMelee
                | bota_proto::UnitKind::CreepFlagbearer
                | bota_proto::UnitKind::CreepRanged
                | bota_proto::UnitKind::CreepSiege
        ) {
            self.march.insert(
                entity,
                March {
                    route_step: 0,
                    trace: None,
                    shove: 0,
                },
            );
        }
        entity
    }

    /// Fills a unit's pools to whatever its stats now allow.
    ///
    /// Called once the stats are known, which is why spawning does not do it.
    pub fn fill_pools(&mut self, entity: Entity) {
        let Some(stats) = self.stats.get(entity).copied() else {
            return;
        };
        if let Some(health) = self.health.get_mut(entity) {
            health.hp = stats.max_hp;
        }
        if let Some(mana) = self.mana.get_mut(entity) {
            mana.mana = stats.max_mana;
        }
    }
}
