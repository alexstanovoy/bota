//! Putting an entity into the world with everything its kind needs.

use bota_core::{
    COURIER, COURIER_ABILITIES, HERO, ModifierKind, NeutralAbility, NeutralKind, UnitDef, hero_def,
    neutral_ability_level, rules,
};
use bota_proto::{AbilityId, Angle, Fixed, HeroId, SlotId, Team, Vec2};

use crate::game::{
    AbilityBook, AbilityState, Action, ActionState, AppliedModifier, AppliedOrigin, Auras, Bounty,
    CampHome, Def, Entity, Errand, Expiry, Health, Hull, Inventory, Lane, LaneAi, Level,
    MAX_SPAWN_MODIFIERS, Mana, March, Mark, Modifier, Modifiers, Motion, NeutralAi, Orders, Plan,
    Rax, Route, Tier, Transform, UnitOrder, Upgrades, World, check_spawn_modifier, hero_kit,
};

/// Which building of a side one is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Fountain,
    Ancient,
    Tower { lane: u8, tier: u8 },
    Barracks { lane: u8, ranged: bool },
    Filler,
}

impl World {
    /// What every kind stands on: where it is, what it is, whose it is, and
    /// the pools it fills once its stats are known.
    ///
    /// Health and mana start empty; [`derive_stats`](crate::game::derive_stats)
    /// fills them the first time it sees the entity.
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
        if def.bounty_gold.max > 0 || def.bounty_xp > 0 || def.bounty_team > 0 {
            self.bounty.insert(
                entity,
                Bounty {
                    gold: def.bounty_gold,
                    bonus: 0,
                    team: def.bounty_team,
                    xp: def.bounty_xp,
                },
            );
        }
        if !def.auras.is_empty() {
            self.auras.insert(entity, Auras(def.auras));
        }
        self.apply_spawn_modifiers(entity);
        entity
    }

    /// Puts every trusted match-setup modifier that takes a unit on it, each
    /// with its own countdown, replacing what setup put there before.
    ///
    /// Runs for every body stood up, a respawned hero included. What a cheat
    /// put on the unit is left alone. Panics on a rule that was never checked.
    pub fn apply_spawn_modifiers(&mut self, entity: Entity) {
        if self.spawn_modifiers.is_empty() {
            return;
        }
        assert!(
            self.spawn_modifiers.len() <= MAX_SPAWN_MODIFIERS,
            "a match may carry at most {} spawn modifiers",
            MAX_SPAWN_MODIFIERS
        );
        let Some(Def(def)) = self.def.get(entity).copied() else {
            return;
        };
        let team = self.team.get(entity).copied();
        let mut applied = self.applied.remove(entity).unwrap_or_default();
        applied.retain(|held| held.origin != AppliedOrigin::Setup);
        for (at, rule) in self.spawn_modifiers.iter().enumerate() {
            if let Err(error) = check_spawn_modifier(rule) {
                panic!("spawn modifier {at} was applied without being checked: {error}");
            }
            if rule.select.takes(def.kind, team) {
                applied.push(AppliedModifier {
                    spec: rule.spec,
                    ticks_left: rule.duration.ticks_left(),
                    origin: AppliedOrigin::Setup,
                });
            }
        }
        if applied.is_empty() {
            return;
        }
        self.applied.insert(entity, applied);
    }

    /// Puts the trusted match-setup modifiers on every unit already standing.
    pub fn apply_spawn_modifiers_to_all(&mut self) {
        if self.spawn_modifiers.is_empty() {
            return;
        }
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            self.apply_spawn_modifiers(entity);
        }
        self.recycle_entity_snapshot(entities);
    }

    /// The room a body takes on the ground and the edge it is reached at.
    fn give_hull(&mut self, entity: Entity, def: &UnitDef) {
        self.hull.insert(
            entity,
            Hull {
                collision: Fixed::from_int(def.collision),
                bound: Fixed::from_int(def.bound),
            },
        );
    }

    fn give_action(&mut self, entity: Entity) {
        self.action.insert(
            entity,
            Action {
                state: ActionState::Ready,
                attack_cooldown: 0,
            },
        );
    }

    /// A body that takes orders, told nothing yet, with the route, the plan
    /// and the motion a walk keeps, all empty.
    fn give_orders(&mut self, entity: Entity) {
        self.orders.insert(
            entity,
            Orders {
                current: UnitOrder::Idle,
                cooldown: 0,
                release_cooldown: 0,
                pending: None,
                immediate: None,
            },
        );
        self.route.insert(entity, Route::none());
        self.plan.insert(entity, Plan::none());
        self.motion.insert(entity, Motion::default());
    }

    /// Puts a hero on the map for a seat.
    pub fn spawn_hero(&mut self, team: Team, pos: Vec2, slot: SlotId, hero: HeroId) -> Entity {
        let body = hero_def(hero).map_or(&HERO, |def| def.unit);
        let entity = self.spawn_body(body, team, pos);
        self.give_hull(entity, body);
        self.give_action(entity);
        self.give_orders(entity);
        self.owner.insert(entity, slot);
        self.hero.insert(entity, hero);
        self.level.insert(entity, Level(1));
        self.abilities.insert(entity, hero_kit(hero));
        self.inventory
            .insert(entity, Inventory::of_hero(slot, self.tick));
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
        self.set_upgrades(entity, def, upgrades);
        if def.kind == bota_proto::UnitKind::CreepFlagbearer {
            self.pledge_flag_bonus(entity);
        }
        self.march.insert(entity, March { next: 0 });
        self.lane_ai.insert(
            entity,
            LaneAi {
                last_seen: None,
                keep_until: 0,
                roused_by: None,
                roused_at_own: false,
                chase_until: 0,
            },
        );
        entity
    }

    /// Puts a neutral on the map at its camp, asleep, with its abilities at
    /// the level a neutral spawned now holds them at.
    pub fn spawn_neutral(
        &mut self,
        kind: NeutralKind,
        pos: Vec2,
        camp: u8,
        upgrades: u32,
    ) -> Entity {
        let def = kind.def();
        let entity = self.spawn_body(def, Team::Neutral, pos);
        if def.collision > 0 {
            self.give_hull(entity, def);
        }
        if def.damage > 0 {
            self.give_action(entity);
        }
        self.give_orders(entity);
        self.neutral.insert(entity, kind);
        self.set_neutral_upgrades(entity, upgrades);
        let level = neutral_ability_level(self.tick);
        self.abilities.insert(
            entity,
            AbilityBook {
                slots: kind
                    .neutral()
                    .abilities
                    .iter()
                    .map(|ability| AbilityState {
                        id: ability.id(),
                        level,
                        cooldown: 0,
                    })
                    .collect(),
                tree: None,
                talents: bota_proto::Talents::default(),
                attribute_bonus: 0,
            },
        );
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

    /// Records how many upgrade intervals a creep stands after, and sets
    /// what killing it pays to its kind's bounty raised by as many steps.
    pub(crate) fn set_upgrades(&mut self, entity: Entity, def: &UnitDef, upgrades: u32) {
        self.upgrades.insert(entity, Upgrades(upgrades));
        self.raise_bounty(entity, def, upgrades);
    }

    /// Raises a neutral that grows with the clock by `count` intervals:
    /// Dota's hidden neutral_upgrade, a modifier no client is told of. One
    /// that does not grow is left as it is.
    pub(crate) fn set_neutral_upgrades(&mut self, entity: Entity, count: u32) {
        let Some(kind) = self.neutral.get(entity).copied() else {
            return;
        };
        if kind.neutral().innate != Some(NeutralAbility::NeutralUpgrade) {
            return;
        }
        if let Some(on_it) = self.modifiers.get_mut(entity) {
            on_it.set_count(ModifierKind::neutral_upgrade, count);
        }
        self.raise_bounty(entity, kind.def(), count);
    }

    /// What killing a creep pays once it has been raised by `upgrades`
    /// intervals.
    fn raise_bounty(&mut self, entity: Entity, def: &UnitDef, upgrades: u32) {
        let steps = i32::try_from(upgrades).expect("upgrade count fits an i32");
        if let Some(bounty) = self.bounty.get_mut(entity) {
            bounty.gold = def
                .bounty_gold
                .raised(def.per_upgrade.gold_halves * steps / 2);
            bounty.xp = def.bounty_xp + def.per_upgrade.xp * steps;
        }
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
        self.give_backdoor(entity);
        match place {
            Place::Fountain | Place::Ancient | Place::Filler => {}
            Place::Tower { lane, tier } => {
                self.lane.insert(entity, Lane(lane));
                self.tier.insert(entity, Tier(tier));
                self.give_orders(entity);
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
        let entity = self.spawn_body(&COURIER, team, pos);
        self.give_action(entity);
        self.give_orders(entity);
        self.owner.insert(entity, slot);
        self.inventory.insert(entity, load);
        self.abilities.insert(
            entity,
            AbilityBook {
                slots: COURIER_ABILITIES
                    .into_iter()
                    .map(|id| AbilityState {
                        id,
                        level: 1,
                        cooldown: 0,
                    })
                    .collect(),
                tree: None,
                talents: bota_proto::Talents::default(),
                attribute_bonus: 0,
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
        self.give_timed_life(entity, ticks);
        entity
    }

    /// Gives a unit a life of so many ticks, as Dota's `modifier_kill`: it is
    /// gone once the modifier runs out.
    pub(crate) fn give_timed_life(&mut self, unit: Entity, ticks: u32) {
        self.put_modifier(
            unit,
            Modifier::new(ModifierKind::TimedLife, None, Some(ticks)),
        );
    }

    /// An entity of a side standing at a spot, with nothing else to it.
    pub(super) fn spawn_placed(&mut self, pos: Vec2, facing: Angle, side: Team) -> Entity {
        let entity = self.spawn();
        self.transform.insert(entity, Transform { pos, facing });
        self.set_team(entity, side);
        entity
    }

    /// Takes an entity [`World::spawn_placed`] stood up out of the world.
    pub(super) fn discard(&mut self, entity: Entity) {
        self.transform.remove(entity);
        self.team.remove(entity);
        self.despawn(entity);
    }

    /// Leaves an ability's mark at a spot, on the caster's side, for so many
    /// ticks. Zero ticks leaves one that stands until it is taken.
    pub fn spawn_mark(
        &mut self,
        ability: AbilityId,
        owner: Entity,
        pos: Vec2,
        ticks: u32,
    ) -> Entity {
        let side = self.team.get(owner).copied().unwrap_or(Team::Neutral);
        let entity = self.spawn_placed(pos, Angle::default(), side);
        self.mark.insert(entity, Mark { ability, owner });
        if ticks > 0 {
            self.expiry.insert(entity, Expiry { ticks_left: ticks });
        }
        entity
    }

    pub fn take_mark(&mut self, entity: Entity) {
        self.mark.remove(entity);
        self.expiry.remove(entity);
        self.discard(entity);
    }

    /// The mark one caster's ability is showing, if it is showing one.
    pub fn mark_of(&self, owner: Entity, ability: AbilityId) -> Option<Entity> {
        self.entities.iter().find(|entity| {
            self.mark
                .get(*entity)
                .is_some_and(|mark| mark.owner == owner && mark.ability == ability)
        })
    }

    /// Puts one unit of any kind on the map with what its numbers call for:
    /// a hull for a collision size, an action for damage, orders for a speed,
    /// a march for a lane creep kind.
    #[cfg(test)]
    pub fn spawn_unit(&mut self, def: &'static UnitDef, team: Team, pos: Vec2) -> Entity {
        let entity = self.spawn_body(def, team, pos);
        if def.collision > 0 {
            self.give_hull(entity, def);
        }
        if def.damage > 0 {
            self.give_action(entity);
        }
        if def.move_speed > 0 || def.kind == bota_proto::UnitKind::Tower {
            self.give_orders(entity);
        }
        if bota_core::kind_def(def.kind).lane_creep {
            self.march.insert(entity, March { next: 0 });
        }
        entity
    }

    /// Fills a unit's pools to whatever its derived stats allow. A unit with
    /// no stats yet is left alone.
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
