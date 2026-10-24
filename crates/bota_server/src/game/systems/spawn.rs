//! Putting an entity into the world with everything its kind needs.

use bota_core::{
    COURIER, COURIER_ABILITIES, HERO, ModifierKind, NeutralAbility, NeutralKind, UnitDef,
    effect_def, hero_def, neutral_ability_level, rules,
};
use bota_proto::{AbilityId, Angle, Fixed, HeroId, Inflictor, SlotId, Team, Vec2};

use crate::game::{
    AbilityBook, AbilityState, Action, ActionState, AppliedModifier, AppliedOrigin, Bounty,
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
        let innate = def.innate.iter();
        let worn = innate.map(|&kind| Modifier::new(kind, Some(entity), None, None));
        self.modifiers.insert(entity, Modifiers(worn.collect()));
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
        let mut book = hero_kit(hero);
        book.slots
            .extend(self.interactions().map(|id| AbilityState {
                id,
                level: 1,
                cooldown: 0,
            }));
        self.abilities.insert(entity, book);
        self.inventory
            .insert(entity, Inventory::of_hero(slot, self.tick));
        entity
    }

    /// Lays a seat's hero down for `wait` ticks: it leaves the map and keeps
    /// all else, as Dota's dead hero, which is never created again (see
    /// [`World::leave_the_map`]). Counting the death and paying for it are
    /// the caller's: an Aegis's rising is a lying down that counts as none.
    pub(crate) fn lay_hero_down(&mut self, index: usize, wait: u32) {
        assert!(wait > 0, "a hero lies dead at least a tick");
        let Some(hero) = self.seats[index].unit else {
            return;
        };
        self.leave_the_map(hero);
        self.seats[index].fell = self.tick;
        self.seats[index].respawn_left = wait;
        // Its numbers are those of what it still wears.
        self.refresh_stats();
    }

    /// Stands a seat's fallen hero up at `at`, full and fresh, with no order
    /// in hand: the same entity that fell. Where it stands up and what it
    /// wears for it are the caller's: the fountain's invulnerability for a
    /// respawn, nothing for an Aegis's rising.
    pub(crate) fn raise_hero(&mut self, index: usize, at: Vec2) {
        let Some(hero) = self.seats[index].unit else {
            return;
        };
        let body = hero_def(self.seats[index].hero).map_or(&HERO, |def| def.unit);
        self.stand_on_the_map(hero, body, at);
        self.seats[index].respawn_left = 0;
        self.settle();
        self.fill_pools(hero);
    }

    /// Takes a unit that respawns (a hero, a courier) off the map and keeps
    /// it: it stands nowhere, so nothing sees it, aims at it, walks round it
    /// or reaches it with an aura; it acts on nothing and takes no order; its
    /// pools are empty. Its book, its bag, its level and what it gathered stay
    /// on it; the modifiers Dota removes on death go.
    fn leave_the_map(&mut self, unit: Entity) {
        if let Some(on_it) = self.modifiers.get_mut(unit) {
            on_it.0.retain(|held| {
                held.ticks_left != Some(0)
                    && effect_def(held.kind.effect_id()).is_some_and(|def| def.kept_through_death)
            });
        }
        self.transform.remove(unit);
        self.hull.remove(unit);
        self.action.remove(unit);
        self.orders.remove(unit);
        self.queue.remove(unit);
        self.route.remove(unit);
        self.plan.remove(unit);
        self.motion.remove(unit);
        self.target.remove(unit);
        self.visibility.remove(unit);
        self.exposure.remove(unit);
        self.handling.remove(unit);
        self.applied.remove(unit);
        self.feast.remove(unit);
        if let Some(health) = self.health.get_mut(unit) {
            health.hp = Fixed::ZERO;
        }
        if let Some(mana) = self.mana.get_mut(unit) {
            mana.mana = Fixed::ZERO;
        }
    }

    /// Puts a unit taken off the map back on it at `at`, as `body` stands:
    /// seen by its own side from this moment, with a body, ready to act and
    /// told nothing, its match-setup modifiers put on afresh.
    fn stand_on_the_map(&mut self, unit: Entity, body: &UnitDef, at: Vec2) {
        let Some(team) = self.team.get(unit).copied() else {
            return;
        };
        self.transform.insert(
            unit,
            Transform {
                pos: at,
                facing: Angle::default(),
            },
        );
        self.set_team(unit, team);
        self.give_hull(unit, body);
        self.give_action(unit);
        self.give_orders(unit);
        self.apply_spawn_modifiers(unit);
    }

    /// Lays a seat's courier down for `wait` ticks, off the map as a fallen
    /// hero is: the same courier stands again, with what it carried.
    pub(crate) fn lay_courier_down(&mut self, index: usize, wait: u32) {
        assert!(wait > 0, "a courier lies dead at least a tick");
        let Some(courier) = self.seats[index].courier else {
            return;
        };
        self.leave_the_map(courier);
        self.seats[index].courier_left = wait;
    }

    /// Stands a seat's fallen courier up at `at`, full, phased and on no
    /// errand, as it first stood.
    pub(crate) fn raise_courier_up(&mut self, index: usize, at: Vec2) {
        let Some(courier) = self.seats[index].courier else {
            return;
        };
        self.stand_on_the_map(courier, &COURIER, at);
        self.put_courier_phased(courier);
        self.errand.insert(courier, Errand::None);
        self.seats[index].courier_left = 0;
        self.settle();
        self.fill_pools(courier);
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
        let level = neutral_ability_level(self.clock());
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
            on_it.set_count(
                ModifierKind::neutral_upgrade,
                count,
                (entity, NeutralAbility::NeutralUpgrade.id()),
            );
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

    /// Puts one of the map's objectives up: a building that stands where it
    /// is put, takes room and does nothing of its own.
    pub(crate) fn spawn_objective(
        &mut self,
        def: &'static UnitDef,
        team: Team,
        pos: Vec2,
    ) -> Entity {
        let entity = self.spawn_body(def, team, pos);
        self.give_hull(entity, def);
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
        self.give_hull(entity, &COURIER);
        self.give_action(entity);
        self.give_orders(entity);
        self.owner.insert(entity, slot);
        self.inventory.insert(entity, load);
        let mut book = AbilityBook {
            slots: COURIER_ABILITIES
                .into_iter()
                .map(|id| AbilityState {
                    id,
                    level: 0,
                    cooldown: 0,
                })
                .collect(),
            tree: None,
            talents: bota_proto::Talents::default(),
            attribute_bonus: 0,
        };
        book.grant(1);
        self.abilities.insert(entity, book);
        self.level.insert(entity, Level(1));
        self.put_courier_phased(entity);
        self.errand.insert(entity, Errand::None);
        entity
    }

    /// Phased from the start (Liquipedia, Courier: "The permanent phased
    /// movement is granted by a status buff", `modifier_phased`).
    fn put_courier_phased(&mut self, courier: Entity) {
        self.put_modifier(
            courier,
            Modifier::new(ModifierKind::Phased, Some(courier), None, None),
        );
    }

    /// Raises an entity to a level: what its kind gains a level comes with
    /// it ([`UnitDef::per_level`]), the bounty its fall pays every enemy
    /// seat and the abilities the level opens among them.
    pub(crate) fn set_level(&mut self, entity: Entity, level: u8) {
        self.level.insert(entity, Level(level));
        if let (Some(Def(def)), Some(bounty)) =
            (self.def.get(entity).copied(), self.bounty.get_mut(entity))
        {
            bounty.team = def.bounty_team + def.per_level.team_gold * (i32::from(level.max(1)) - 1);
        }
        if let Some(book) = self.abilities.get_mut(entity) {
            book.grant(level);
        }
    }

    /// Stands a ward at a spot for so many ticks; `by` is who stood it, and
    /// with what.
    pub fn spawn_ward(
        &mut self,
        def: &'static UnitDef,
        (team, pos): (Team, Vec2),
        ticks: u32,
        by: (Option<Entity>, Option<Inflictor>),
    ) -> Entity {
        let entity = self.spawn_body(def, team, pos);
        self.give_timed_life(entity, ticks, by);
        entity
    }

    /// Gives a unit a life of so many ticks, as Dota's `modifier_kill`: it is
    /// gone once the modifier runs out. `by` is who called it up, and with
    /// what.
    pub(crate) fn give_timed_life(
        &mut self,
        unit: Entity,
        ticks: u32,
        (source, inflictor): (Option<Entity>, Option<Inflictor>),
    ) {
        self.put_modifier(
            unit,
            Modifier::new(ModifierKind::TimedLife, source, inflictor, Some(ticks)),
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
