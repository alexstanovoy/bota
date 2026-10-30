//! The world as a side is allowed to see it.

use bota_core::{ability_mana_cost, effect_def, kind_def, phase_ticks_left, rules};
use bota_proto::{
    AbilityId, AbilityView, AggroView, CastView, EffectView, Fixed, Inflictor, PlayerView,
    ProjectileView, StatusFlags, SwingView, TalentSlot, TalentView, Talents, Team, UnitKind,
    UnitView, WorldView,
};

use crate::game::{ActionPhase, ActionState, Entity, Seat, World, item_view, item_views};
use crate::profile::Phase;

/// The handle as it travels on the wire.
pub fn wire_id(entity: Entity) -> bota_proto::EntityId {
    bota_proto::EntityId {
        idx: entity.index().0,
        generation: entity.generation().0.get(),
    }
}

impl World {
    /// The world through one side's fog of war.
    /// How far cheats moved the clocks, absent while they never did.
    fn clock_shift(&self) -> Option<bota_proto::ClockShift> {
        (self.clock_shift != bota_proto::ClockShift::default()).then_some(self.clock_shift)
    }

    pub fn view(&self, team: Team) -> WorldView {
        self.project(Some(team))
    }

    /// The whole world, for spectators and the replay.
    pub fn view_full(&self) -> WorldView {
        self.project(None)
    }

    /// Everything a viewer is allowed to be told. `None` holds nothing back.
    fn project(&self, viewer: Option<Team>) -> WorldView {
        let _profile = self.scope(Phase::Projection);
        let mut units = Vec::with_capacity(self.entities.len());
        let mut projectiles = Vec::new();
        let mut loot = Vec::new();
        let mut aggro = Vec::new();
        for entity in self.entities.iter() {
            let seen = self.viewer_sees(viewer, entity);
            if !(seen || self.own_to(viewer, entity)) {
                continue;
            }
            if let Some(unit) = self.told_unit(viewer, entity) {
                aggro.extend(self.project_aggro(viewer, entity, unit.kind));
                units.push(unit);
            }
            projectiles.extend(self.project_projectile(viewer, entity));
            if seen {
                loot.extend(self.project_loot(viewer, entity));
            }
        }
        debug_assert!(aggro.iter().all(|entry| {
            let carried = |id| units.binary_search_by_key(&id, |unit| unit.id).is_ok();
            carried(entry.unit)
                && entry.target.is_none_or(carried)
                && entry.swing.is_none_or(|swing| carried(swing.target))
        }));
        debug_assert!(projectiles.iter().all(|shot| {
            let carried = |id| units.binary_search_by_key(&id, |unit| unit.id).is_ok();
            shot.source.is_none_or(carried) && shot.target.is_none_or(carried)
        }));
        debug_assert!(units.iter().flat_map(|unit| &unit.effects).all(|effect| {
            let carried = |id| units.binary_search_by_key(&id, |unit| unit.id).is_ok();
            effect.caster.is_none_or(carried)
        }));
        let talents = self.project_talents(viewer, &units);
        let disguised = self.disguised();
        let (felled_trees, planted_trees) = self.project_forest(viewer);
        let view = WorldView {
            tick: self.tick,
            viewer,
            units,
            projectiles,
            players: self
                .seats
                .iter()
                .map(|seat| self.project_player(viewer, seat, &disguised))
                .collect(),
            felled_trees,
            planted_trees,
            loot,
            aggro,
            order_delays: Vec::new(),
            skipped_ticks: Vec::new(),
            stock: self.project_stock(viewer),
            talents,
            teleports: self.teleport_views(),
            glyphs: self.glyph_views(viewer),
            clock: self.clock_shift(),
        };
        debug_assert!(within_limits(&view), "a view past the protocol's bounds");
        view
    }

    /// Every hero with illusions of it standing.
    fn disguised(&self) -> Vec<Entity> {
        let mut heroes: Vec<Entity> = self
            .entities
            .iter()
            .filter_map(|entity| self.illusion.get(entity).map(|illusion| illusion.of))
            .collect();
        heroes.sort_unstable();
        heroes.dedup();
        heroes
    }

    /// One seat as a viewer is told of it. Which unit is the hero is kept
    /// from the other side while illusions of it stand. As Dota's scoreboard
    /// has it, the other side is told the hero's level and experience as it
    /// last saw them, and neither its gold nor its last hits and denies; its
    /// kills, deaths, assists and respawn wait are public.
    fn project_player(
        &self,
        viewer: Option<Team>,
        seat: &Seat,
        disguised: &[Entity],
    ) -> PlayerView {
        let mana_rate = self.mana_cost_rate_of(seat.unit);
        let own = viewer.is_none_or(|team| team == seat.team);
        let in_view = seat
            .unit
            .zip(viewer)
            .is_some_and(|(unit, team)| self.shows(team, unit));
        let shown = if own || in_view {
            crate::game::Sighting {
                level: seat.level,
                xp: seat.xp,
                at: Some(self.tick),
            }
        } else {
            viewer
                .and_then(bota_core::side_index)
                .map_or(crate::game::Sighting::START, |side| seat.sighted[side])
        };
        PlayerView {
            slot: seat.slot,
            team: seat.team,
            hero: seat.hero,
            unit: seat
                .unit
                .filter(|unit| own || !disguised.contains(unit))
                .map(wire_id),
            level: shown.level,
            xp: shown.xp,
            // A side is told its own gold, stash and what a fallen body
            // left, nobody else's.
            gold: own.then_some(seat.gold),
            stash: own.then(|| item_views(&seat.stash, mana_rate, &seat.item_clocks)),
            kit: seat
                .kept
                .as_ref()
                .filter(|_| own)
                .map(|kept| bota_proto::Kit {
                    abilities: ability_views(&kept.book, mana_rate),
                    items: item_views(&kept.bag, mana_rate, &seat.item_clocks),
                    town_portal: kept
                        .bag
                        .town_portal
                        .flatten()
                        .map(|stack| item_view(&stack, mana_rate, &seat.item_clocks)),
                }),
            kills: seat.kills,
            deaths: seat.deaths,
            assists: seat.assists,
            last_hits: 0,
            denies: 0,
            respawn_left: seat.respawn_left,
            creep_score: own.then_some(bota_proto::CreepScore {
                last_hits: seat.last_hits,
                denies: seat.denies,
            }),
            seen_at: shown.at,
        }
    }

    /// The forest as the viewer last saw it, or as it stands: which of the
    /// map's trees are down, and where the planted ones stand.
    fn project_forest(&self, viewer: Option<Team>) -> (Vec<u32>, Vec<bota_proto::Vec2>) {
        let Some(team) = viewer else {
            return (
                self.trees.felled().collect(),
                self.trees.planted().iter().map(|tree| tree.at).collect(),
            );
        };
        self.trees
            .known(team)
            .map_or_else(Default::default, |known| {
                (known.down.clone(), known.planted.clone())
            })
    }

    /// What the shops of the viewer's side, or of every side, have left.
    fn project_stock(&self, viewer: Option<Team>) -> Vec<bota_proto::StockView> {
        self.shop_stock
            .iter()
            .filter(|held| viewer.is_none_or(|team| team == held.team))
            .map(|held| bota_proto::StockView {
                team: held.team,
                item: held.item,
                left: held.left,
                max: bota_core::item_def(held.item)
                    .and_then(|def| def.stock)
                    .map_or(0, |rule| rule.max),
                restock_left: held.restock_left,
            })
            .collect()
    }

    /// The talents of every seat whose hero has a tree that a viewer is told
    /// of: its own side's, standing or fallen, and another side's while the
    /// hero is among the units it is told of. What a point could go into is
    /// told to the seat's own side alone.
    fn project_talents(&self, viewer: Option<Team>, units: &[UnitView]) -> Vec<TalentView> {
        self.seats
            .iter()
            .filter_map(|seat| {
                let book = match seat.unit {
                    Some(unit) => self.abilities.get(unit),
                    None => seat.kept.as_ref().map(|kept| &kept.book),
                }?;
                book.tree?;
                let own = viewer.is_none_or(|team| team == seat.team);
                let carried = seat.unit.is_some_and(|unit| {
                    units.iter().any(|shown| {
                        self.of_wire(shown.id)
                            .is_some_and(|entity| self.shows_as(entity) == unit)
                    })
                });
                if !own && !carried {
                    return None;
                }
                let learnable = seat.unit.filter(|unit| own && self.alive(*unit)).map_or(
                    Talents::default(),
                    |unit| {
                        (0..TalentSlot::COUNT)
                            .map(TalentSlot)
                            .filter(|slot| self.can_learn_talent(unit, *slot))
                            .fold(Talents::default(), Talents::with)
                    },
                );
                let attribute_bonus_learnable = seat.unit.is_some_and(|unit| {
                    own && self.alive(unit) && self.can_learn_attribute_bonus(unit)
                });
                Some(TalentView {
                    slot: seat.slot,
                    learned: book.talents,
                    learnable: Talents::default(),
                    attribute_bonus: book.attribute_bonus,
                    attribute_bonus_learnable: false,
                    may_learn: own.then_some(learnable),
                    may_learn_attribute_bonus: own.then_some(attribute_bonus_learnable),
                })
            })
            .collect()
    }

    /// Whether a viewer is shown an entity. `None` sees everything.
    fn viewer_sees(&self, viewer: Option<Team>, entity: Entity) -> bool {
        viewer.is_none_or(|team| self.shows(team, entity))
    }

    /// Whether an entity is of a viewer's own side.
    fn own_to(&self, viewer: Option<Team>, entity: Entity) -> bool {
        viewer.is_some_and(|team| self.team.get(entity) == Some(&team))
    }

    /// Whether a viewer's units carry this entity.
    fn carried(&self, viewer: Option<Team>, entity: Entity) -> bool {
        self.entities.contains(entity)
            && (self.viewer_sees(viewer, entity) || self.own_to(viewer, entity))
            && self.kind.contains(entity)
            && self.transform.contains(entity)
            && self.stats.contains(entity)
    }

    /// Whom one carried unit of a kind fights, as far as the viewer is told,
    /// or nothing when there is nothing to tell.
    fn project_aggro(
        &self,
        viewer: Option<Team>,
        entity: Entity,
        kind: UnitKind,
    ) -> Option<AggroView> {
        let held = self.target_of(entity);
        let swing = self.swing_of(entity);
        let lane = kind_def(kind)
            .lane_creep
            .then(|| self.lane_ai.get(entity))
            .flatten();
        let cooldown = self.orders.get(entity).map_or(0, |orders| orders.cooldown);
        // A hero's wait is its own side's; a tower's is anyone's to see.
        let draw_wait = match kind {
            UnitKind::Hero if viewer.is_none() || self.own_to(viewer, entity) => Some(cooldown),
            UnitKind::Hero => None,
            UnitKind::Tower => Some(cooldown),
            _ => Some(0),
        };
        let returning = kind == UnitKind::CreepNeutral
            && self.neutral_ai.get(entity).is_some_and(|ai| ai.going_home);
        // A hold is on the target: of one the view does not carry, nothing
        // is told.
        let target = held.filter(|on| self.carried(viewer, *on)).map(wire_id);
        let held_for = target.map(|_| lane.map_or(0, |ai| ai.keep_until.saturating_sub(self.tick)));
        let swing = swing
            .filter(|(on, _)| self.carried(viewer, *on))
            .map(|(on, strike_in)| SwingView {
                target: wire_id(on),
                strike_in,
            });
        let quiet = target.is_none()
            && swing.is_none()
            && draw_wait.is_none_or(|wait| wait == 0)
            && !returning;
        (!quiet).then(|| AggroView {
            unit: wire_id(entity),
            target,
            swing,
            hold_left: 0,
            chase_left: 0,
            draw_cooldown: 0,
            returning,
            held_for,
            chased_for: None,
            draw_wait,
        })
    }

    /// The swing an entity has in its windup: whom at, and in how many ticks
    /// it strikes at the current attack speed.
    fn swing_of(&self, entity: Entity) -> Option<(Entity, u32)> {
        let ActionState::Attack {
            target,
            phase: ActionPhase::Before { progress },
        } = self.action.get(entity)?.state
        else {
            return None;
        };
        let stats = self.stats.get(entity)?;
        Some((
            target,
            phase_ticks_left(stats.attack_point, progress, stats.attack_speed),
        ))
    }

    /// One missile, hook, mark, line of a requiem or Blood Grenade, or
    /// nothing when the entity is none of them. Who let it go and whom it
    /// homes on are told while the viewer's units carry them; how an attack's
    /// missile looks is told with the missile.
    fn project_projectile(&self, viewer: Option<Team>, entity: Entity) -> Option<ProjectileView> {
        let ability = |id| Some(Inflictor::Ability(id));
        let (inflictor, source, target, model) = if let Some(missile) = self.projectile.get(entity)
        {
            let target = Some(missile.target);
            (missile.inflictor, missile.source, target, missile.model)
        } else if let Some(mark) = self.mark.get(entity) {
            (ability(mark.ability), Some(mark.owner), None, None)
        } else if let Some(line) = self.requiem_line.get(entity) {
            (ability(AbilityId::REQUIEM), Some(line.owner), None, None)
        } else if let Some(hook) = self.hook.get(entity) {
            (ability(AbilityId::MEAT_HOOK), Some(hook.owner), None, None)
        } else {
            let grenade = self.grenade.get(entity)?;
            let item = Some(Inflictor::Item(grenade.item));
            (item, Some(grenade.owner), None, None)
        };
        let at = self.transform.get(entity)?;
        let told =
            |unit: Option<Entity>| unit.filter(|unit| self.carried(viewer, *unit)).map(wire_id);
        Some(ProjectileView {
            id: wire_id(entity),
            pos: at.pos,
            facing: at.facing,
            team: self.team.get(entity).copied().unwrap_or(Team::Neutral),
            ability: None,
            source: told(source),
            target: told(target),
            inflictor,
            seen_by: self.seen_by_sides(viewer, entity, true),
            model,
        })
    }

    /// Which sides' views hold an entity: told in the view of everything
    /// alone, from the very rule a side's view is made by. `own_shown` for
    /// what a side's view holds of its own whether it sees it or not.
    fn seen_by_sides(
        &self,
        viewer: Option<Team>,
        entity: Entity,
        own_shown: bool,
    ) -> Option<bota_proto::SeenBy> {
        if viewer.is_some() {
            return None;
        }
        let held = |team| {
            self.viewer_sees(Some(team), entity) || (own_shown && self.own_to(Some(team), entity))
        };
        Some(bota_proto::SeenBy {
            radiant: held(Team::Radiant),
            dire: held(Team::Dire),
        })
    }

    /// One item on the ground, or nothing when the entity is not one.
    fn project_loot(&self, viewer: Option<Team>, entity: Entity) -> Option<bota_proto::LootView> {
        let crate::game::Loot(stack) = self.loot.get(entity)?;
        let at = self.transform.get(entity)?;
        let def = bota_core::item_def(stack.id);
        Some(bota_proto::LootView {
            id: wire_id(entity),
            pos: at.pos,
            item: stack.id,
            charges: def
                .filter(|def| def.charges > 0 || def.cast_charges > 0)
                .map(|_| stack.active_charges()),
            seen_by: self.seen_by_sides(viewer, entity, false),
        })
    }

    /// The hero an entity shows as: the one it is an illusion of, or
    /// itself.
    fn shows_as(&self, entity: Entity) -> Entity {
        self.illusion
            .get(entity)
            .map_or(entity, |illusion| illusion.of)
    }

    /// One unit as a viewer is told of it: its queue to its own side alone,
    /// and an illusion to the other side as its hero (see the contract).
    fn told_unit(&self, viewer: Option<Team>, entity: Entity) -> Option<UnitView> {
        let mut unit = self.project_unit(viewer, entity)?;
        if viewer.is_none_or(|team| team == unit.team) {
            return Some(unit);
        }
        unit.queue.clear();
        unit.effects
            .retain(|effect| effect_def(effect.id).is_none_or(|def| !def.own_side_only));
        if let Some(illusion) = self.illusion.get(entity)
            && let Some(hero) = self.project_unit(viewer, illusion.of)
        {
            wear_sheet(&mut unit, hero);
        }
        keep_to_its_side(&mut unit);
        Some(unit)
    }

    /// One unit, or nothing when the entity is not one.
    fn project_unit(&self, viewer: Option<Team>, entity: Entity) -> Option<UnitView> {
        let kind = *self.kind.get(entity)?;
        let transform = self.transform.get(entity)?;
        let stats = self.stats.get(entity)?;
        let health = self.health.get(entity);
        let mana = self.mana.get(entity);
        // The waits a seat owes on kinds of item show on its hero's stacks.
        let clocks: &[(bota_proto::ItemId, u32)] = self
            .seats
            .iter()
            .find(|seat| seat.unit == Some(entity))
            .map_or(&[], |seat| &seat.item_clocks);
        let mana_rate = self.mana_cost_rate_of(Some(entity));
        let bag = self.inventory.get(entity);
        let hull = self.hull.get(entity);
        Some(UnitView {
            id: wire_id(entity),
            kind,
            team: self.team.get(entity).copied().unwrap_or(Team::Neutral),
            pos: transform.pos,
            facing: transform.facing,
            hp: shown(health.map_or(Fixed::ZERO, |h| h.hp)),
            max_hp: stats.max_hp.to_int(),
            mana: shown(mana.map_or(Fixed::ZERO, |m| m.mana)),
            max_mana: stats.max_mana.to_int(),
            move_speed: stats.move_speed,
            attack_damage: stats.damage + stats.damage_spread / 2,
            attack_damage_min: stats.damage,
            attack_damage_max: stats.damage + stats.damage_spread,
            hp_regen: per_second_view(stats.hp_regen),
            mana_regen: per_second_view(stats.mana_regen),
            attack_range: stats.attack_range,
            attack_time: after_speed(stats.attack_time, stats.attack_speed),
            attack_point: after_speed(stats.attack_point, stats.attack_speed),
            attack_speed: stats.attack_speed,
            armor: stats.armor,
            magic_resist: Fixed::from_ratio(stats.magic_resist_bp, rules::NOMINAL_BP),
            status_resist: Fixed::from_ratio(stats.status_resist_bp, rules::NOMINAL_BP),
            evasion: stats.evasion.fraction(),
            seen_by: self.seen_by_sides(viewer, entity, true),
            collision: hull.map_or(Fixed::ZERO, |h| h.collision),
            bound: hull.map_or(Fixed::ZERO, |h| h.bound),
            vision_radius: stats.vision,
            true_sight_radius: stats.true_sight,
            statuses: StatusFlags {
                bits: self.statuses_of(entity, stats),
            },
            attributes: stats.attributes,
            primary: stats.primary,
            hero: self.hero.get(entity).copied(),
            owner: self.owner.get(entity).copied(),
            level: self.level.get(entity).map_or(0, |l| l.0),
            abilities: self.abilities.get(entity).map_or_else(Vec::new, |book| {
                let level = self.level.get(entity).map_or(0, |held| held.0);
                book.slots
                    .iter()
                    .enumerate()
                    .filter(|(_, ability)| slot_shown(ability))
                    .map(|(slot, ability)| {
                        ability_view(
                            ability,
                            self.ability_on(entity, ability.id),
                            book.can_raise(slot, level),
                            mana_rate,
                        )
                    })
                    .collect()
            }),
            items: bag.map_or_else(Vec::new, |bag| item_views(bag, mana_rate, clocks)),
            effects: self.effects_on(viewer, entity),
            town_portal: bag
                .and_then(|bag| bag.town_portal.flatten())
                .map(|stack| item_view(&stack, mana_rate, clocks)),
            neutral: self.neutral.get(entity).map(|kind| kind.id()),
            cast: self.cast_of(entity),
            queue: self.queue.get(entity).map_or_else(Vec::new, |queue| {
                queue.iter().map(|queued| queued.order).collect()
            }),
        })
    }
}

/// Puts a hero's sheet on the view of an illusion of it: everything that
/// says what the hero is and holds, leaving the illusion's own body (where
/// it stands and faces, its health, statuses, effects, cast and queue).
/// Whether every collection of the view is within the protocol's bounds
/// ([`bota_proto::limits`]): what a client refuses a view for.
fn within_limits(view: &WorldView) -> bool {
    use bota_proto::limits::*;
    view.units.len() <= MAX_VIEW_UNITS
        && view.projectiles.len() <= MAX_PROJECTILES
        && view.players.len() <= MAX_SEATS
        && view.loot.len() <= MAX_SNAPSHOT_LOOT
        && view.felled_trees.len() <= MAX_STATIC_TREES
        && view.planted_trees.len() <= MAX_PLANTED_TREES
        && view.stock.len() <= 2 * MAX_SHOP_ENTRIES
        && view.skipped_ticks.len() <= MAX_SKIPPED_TICKS
        && [
            view.order_delays.len(),
            view.talents.len(),
            view.teleports.len(),
            view.glyphs.len(),
        ]
        .into_iter()
        .all(|len| len <= MAX_SEATS)
        && view.units.iter().all(|unit| {
            unit.abilities.len() <= MAX_ABILITY_SLOTS
                && unit.items.len() <= HERO_ITEM_SLOTS
                && unit.effects.len() <= MAX_EFFECTS_PER_UNIT
                && unit.queue.len() <= MAX_QUEUED_ORDERS
        })
}

fn wear_sheet(illusion: &mut UnitView, hero: UnitView) {
    let body = UnitView {
        id: illusion.id,
        team: illusion.team,
        pos: illusion.pos,
        facing: illusion.facing,
        hp: illusion.hp,
        max_hp: illusion.max_hp,
        statuses: illusion.statuses,
        effects: core::mem::take(&mut illusion.effects),
        cast: illusion.cast,
        queue: core::mem::take(&mut illusion.queue),
        ..hero
    };
    *illusion = body;
}

/// Takes off a unit what only its own side is told: whether a skill point
/// could go into an ability, and which items are marked for sale. Dota
/// shows another side's abilities with their levels and cooldowns, and its
/// items, but not its unspent points or its own plans for them.
fn keep_to_its_side(unit: &mut UnitView) {
    for ability in &mut unit.abilities {
        ability.learnable = None;
    }
    for item in unit.items.iter_mut().flatten().chain(&mut unit.town_portal) {
        item.selling = None;
    }
}

/// Milliseconds of animation at an attack speed, clamped as the cycle
/// clamps it.
fn after_speed(ms: u32, attack_speed: i32) -> u32 {
    let speed = attack_speed.clamp(
        bota_core::rules::MIN_ATTACK_SPEED,
        bota_core::rules::MAX_ATTACK_SPEED,
    );
    ms * bota_core::rules::BASE_ATTACK_SPEED as u32 / speed as u32
}

/// A pool as a number to show: anything above zero shows as at least one
/// point.
fn shown(held: Fixed) -> i32 {
    if held > Fixed::ZERO {
        held.to_int().max(1)
    } else {
        held.to_int()
    }
}

/// One ability slot on the wire. Whether a toggle is running and whether a
/// point could go into it come from the caller.
fn ability_view(
    held: &crate::game::AbilityState,
    on: bool,
    can_level: bool,
    mana_rate_bp: i32,
) -> AbilityView {
    let def = bota_core::ability_def(held.id);
    AbilityView {
        id: held.id,
        level: held.level,
        max_level: def.map_or(0, |def| def.max_level),
        cooldown_left: held.cooldown,
        mana_cost: crate::game::cost_after(ability_mana_cost(held.id, held.level), mana_rate_bp),
        range: def.map_or(0, |def| def.range),
        aim: def.map_or(bota_proto::Aim::Own, |def| def.aim),
        passive: def.is_some_and(|def| def.passive),
        on,
        can_level: false,
        learnable: Some(can_level),
    }
}

/// A whole book on the wire, as it stands with no body under it: nothing
/// toggled on and no point to spend.
fn ability_views(book: &crate::game::AbilityBook, mana_rate_bp: i32) -> Vec<AbilityView> {
    book.slots
        .iter()
        .filter(|held| slot_shown(held))
        .map(|held| ability_view(held, false, false, mana_rate_bp))
        .collect()
}

/// Whether a view shows a slot: every one but a hidden ability's, which
/// sits after the shown ones, so a shown slot's index is its slot's.
fn slot_shown(held: &crate::game::AbilityState) -> bool {
    bota_core::ability_def(held.id).is_none_or(|def| !def.hidden)
}

impl World {
    /// Everything showing on one entity: its active modifiers but those Dota
    /// hides, each with who put it on, while the viewer's units carry that
    /// one, and with what. One a Break disables shows a count of zero, as
    /// Dota's HUD shows a broken aura's icon.
    fn effects_on(&self, viewer: Option<Team>, entity: Entity) -> Vec<EffectView> {
        self.modifiers
            .get(entity)
            .into_iter()
            .flat_map(|on_it| on_it.active())
            .filter(|held| effect_def(held.kind.effect_id()).is_none_or(|def| !def.hidden))
            .map(|held| EffectView {
                id: held.kind.effect_id(),
                ticks_left: held.ticks_left,
                stacks: if self.disabled(held) {
                    Some(0)
                } else {
                    held.kind.stacks()
                },
                caster: held
                    .source
                    .filter(|by| self.carried(viewer, *by))
                    .map(wire_id),
                inflictor: held.inflictor,
                ticks_total: held.ticks_total,
            })
            .collect()
    }

    /// The cast point an entity stands in: the ability it casts, none for
    /// an item, and the ticks before it goes off.
    fn cast_of(&self, entity: Entity) -> Option<CastView> {
        let (ability, point, progress) = match self.action.get(entity)?.state {
            ActionState::CastAbility {
                slot,
                phase: ActionPhase::Before { progress },
                ..
            } => {
                let ability = self.ability_in(entity, slot);
                let point = bota_core::ability_def(ability).map_or(0, |def| def.point);
                (Some(ability), point, progress)
            }
            ActionState::UseItem {
                slot,
                phase: ActionPhase::Before { progress },
                ..
            } => {
                let def = self.item_def_in(entity, usize::from(slot.0));
                (None, def.map_or(0, |def| def.point), progress)
            }
            _ => return None,
        };
        let point = self.cast_point_of(entity, point);
        Some(CastView {
            ability,
            goes_off_in: phase_ticks_left(point, progress, rules::BASE_ATTACK_SPEED),
        })
    }

    /// Whether an entity has a toggle switched on right now: it wears a
    /// modifier it put on itself with that ability.
    fn ability_on(&self, entity: Entity, id: bota_proto::AbilityId) -> bool {
        bota_core::ability_def(id).is_some_and(|def| def.toggle)
            && self.modifiers.get(entity).is_some_and(|on_it| {
                on_it.active().any(|held| {
                    held.source == Some(entity) && held.inflictor == Some(Inflictor::Ability(id))
                })
            })
    }

    /// The [`StatusFlags`] bits of a unit: what each of its modifiers does to
    /// it ([`ModifierKind::statuses`]), and whether it hides, cannot be hurt
    /// or channels.
    fn statuses_of(&self, entity: Entity, stats: &crate::game::Stats) -> u16 {
        let mut bits = 0;
        for held in self
            .modifiers
            .get(entity)
            .into_iter()
            .flat_map(|on_it| on_it.active())
        {
            bits |= held.kind.statuses();
        }
        for (set, flag) in [
            (stats.hides, StatusFlags::INVISIBLE),
            (stats.invulnerable, StatusFlags::INVULNERABLE),
            (stats.spell_immune, StatusFlags::MAGIC_IMMUNE),
            (self.is_channelling(entity), StatusFlags::CHANNELLING),
            (self.is_casting(entity), StatusFlags::CASTING),
        ] {
            if set {
                bits |= flag;
            }
        }
        bits
    }
}

/// A per-tick regeneration as points per second.
fn per_second_view(per_tick: Fixed) -> Fixed {
    Fixed {
        raw: per_tick.raw.saturating_mul(rules::TICKS_PER_SECOND as i32),
    }
}
