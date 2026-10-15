//! The world as a side is allowed to see it.

use bota_proto::{
    AbilityView, AggroView, EffectId, EffectView, Fixed, PlayerView, ProjectileView, StatusFlags,
    SwingView, TalentSlot, TalentView, Talents, Team, UnitKind, UnitView, WorldView,
};

use crate::game::{
    ActionPhase, ActionState, Entity, FeastPhase, ModifierKind, StackKind, World,
    ability_mana_cost, attack_gain, beats, is_lane_creep, item_view, item_views, rules,
};
use crate::profile::Phase;

/// The wire's effect id of Shadowraze: one row per source, carrying its ticks
/// and stacks.
pub const EFFECT_SHADOWRAZE: u16 = 15;

/// The handle as it travels on the wire.
pub fn wire_id(entity: Entity) -> bota_proto::EntityId {
    bota_proto::EntityId {
        idx: entity.index().0,
        generation: entity.generation().0.get(),
    }
}

impl World {
    /// The world through one side's fog of war.
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
            if let Some(unit) = self.project_unit(entity) {
                aggro.extend(self.project_aggro(viewer, entity, unit.kind));
                units.push(unit);
            }
            projectiles.extend(self.project_projectile(entity));
            if seen {
                loot.extend(self.project_loot(entity));
            }
        }
        debug_assert!(aggro.iter().all(|entry| {
            let carried = |id| units.binary_search_by_key(&id, |unit| unit.id).is_ok();
            carried(entry.unit)
                && entry.target.is_none_or(carried)
                && entry.swing.is_none_or(|swing| carried(swing.target))
        }));
        let talents = self.project_talents(viewer, &units);
        WorldView {
            tick: self.tick,
            viewer,
            units,
            projectiles,
            players: self
                .seats
                .iter()
                .map(|seat| PlayerView {
                    slot: seat.slot,
                    team: seat.team,
                    hero: seat.hero,
                    unit: seat.unit.map(wire_id),
                    level: seat.level,
                    xp: seat.xp,
                    // A side is told its own gold and its own stash, nobody
                    // else's.
                    gold: match viewer {
                        None => Some(seat.gold),
                        Some(team) if team == seat.team => Some(seat.gold),
                        Some(_) => None,
                    },
                    stash: match viewer {
                        Some(team) if team != seat.team => None,
                        _ => Some(item_views(
                            &seat.stash,
                            self.mana_cost_rate_of(seat.unit),
                            &seat.item_clocks,
                        )),
                    },
                    // What a fallen body left is told to its own side alone.
                    kit: match viewer {
                        Some(team) if team != seat.team => None,
                        _ => seat.kept.as_ref().map(|kept| bota_proto::Kit {
                            abilities: ability_views(&kept.book, self.mana_cost_rate_of(seat.unit)),
                            items: item_views(
                                &kept.bag,
                                self.mana_cost_rate_of(seat.unit),
                                &seat.item_clocks,
                            ),
                            town_portal: kept.bag.town_portal.flatten().map(|stack| {
                                item_view(
                                    &stack,
                                    self.mana_cost_rate_of(seat.unit),
                                    &seat.item_clocks,
                                )
                            }),
                        }),
                    },
                    kills: seat.kills,
                    deaths: seat.deaths,
                    assists: seat.assists,
                    last_hits: seat.last_hits,
                    denies: seat.denies,
                    respawn_left: seat.respawn_left,
                })
                .collect(),
            felled_trees: self.trees.felled().collect(),
            planted_trees: self.trees.planted().iter().map(|tree| tree.at).collect(),
            loot,
            aggro,
            order_delays: Vec::new(),
            skipped_ticks: Vec::new(),
            stock: self.project_stock(viewer),
            talents,
            teleports: self.teleport_views(),
        }
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
                max: crate::game::item_def(held.item)
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
                    units
                        .binary_search_by_key(&wire_id(unit), |shown| shown.id)
                        .is_ok()
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
                Some(TalentView {
                    slot: seat.slot,
                    learned: book.talents,
                    learnable,
                })
            })
            .collect()
    }

    /// Whether a viewer sees an entity. `None` sees everything.
    fn viewer_sees(&self, viewer: Option<Team>, entity: Entity) -> bool {
        viewer.is_none_or(|team| {
            self.visibility
                .get(entity)
                .is_some_and(|seen| seen.by(team))
        })
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
        let lane = is_lane_creep(kind)
            .then(|| self.lane_ai.get(entity))
            .flatten();
        let own_hero = kind == UnitKind::Hero && (viewer.is_none() || self.own_to(viewer, entity));
        let draw_cooldown = if own_hero || kind == UnitKind::Tower {
            self.orders.get(entity).map_or(0, |orders| orders.cooldown)
        } else {
            0
        };
        let returning = kind == UnitKind::CreepNeutral
            && self.neutral_ai.get(entity).is_some_and(|ai| ai.going_home);
        let hold_left = lane.map_or(0, |ai| ai.keep_until.saturating_sub(self.tick));
        let chase_left = lane
            .filter(|_| held.is_some())
            .map_or(0, |ai| ai.chase_until.saturating_sub(self.tick));
        let target = held.filter(|on| self.carried(viewer, *on)).map(wire_id);
        let swing = swing
            .filter(|(on, _)| self.carried(viewer, *on))
            .map(|(on, strike_in)| SwingView {
                target: wire_id(on),
                strike_in,
            });
        let quiet = target.is_none()
            && swing.is_none()
            && hold_left == 0
            && chase_left == 0
            && draw_cooldown == 0
            && !returning;
        (!quiet).then(|| AggroView {
            unit: wire_id(entity),
            target,
            swing,
            hold_left,
            chase_left,
            draw_cooldown,
            returning,
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
        let left = beats(stats.attack_point).saturating_sub(progress);
        let gain = attack_gain(stats.attack_speed);
        assert!(gain > 0);
        Some((target, left.div_ceil(gain).max(1)))
    }

    /// One missile, hook, mark, line of a requiem or Blood Grenade, or
    /// nothing when the entity is none of them.
    fn project_projectile(&self, entity: Entity) -> Option<ProjectileView> {
        let ability = if self.projectile.get(entity).is_some() {
            None
        } else if let Some(mark) = self.mark.get(entity) {
            Some(mark.ability)
        } else if self.requiem_line.get(entity).is_some() {
            Some(crate::game::ability::REQUIEM)
        } else if self.hook.get(entity).is_some() {
            Some(crate::game::ability::MEAT_HOOK)
        } else if self.grenade.get(entity).is_some() {
            Some(crate::game::ability::THROW_GRENADE)
        } else {
            return None;
        };
        let at = self.transform.get(entity)?;
        Some(ProjectileView {
            id: wire_id(entity),
            pos: at.pos,
            facing: at.facing,
            team: self.team.get(entity).copied().unwrap_or(Team::Neutral),
            ability,
        })
    }

    /// One item on the ground, or nothing when the entity is not one.
    fn project_loot(&self, entity: Entity) -> Option<bota_proto::LootView> {
        let crate::game::Loot(stack) = self.loot.get(entity)?;
        let at = self.transform.get(entity)?;
        let def = crate::game::item_def(stack.id);
        Some(bota_proto::LootView {
            id: wire_id(entity),
            pos: at.pos,
            item: stack.id,
            charges: def
                .filter(|def| def.charges > 0 || def.cast_charges > 0)
                .map(|_| stack.charges),
        })
    }

    /// One unit, or nothing when the entity is not one.
    fn project_unit(&self, entity: Entity) -> Option<UnitView> {
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
            collision: self.hull.get(entity).map_or(Fixed::ZERO, |h| h.collision),
            bound: self.hull.get(entity).map_or(Fixed::ZERO, |h| h.bound),
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
                book.slots
                    .iter()
                    .map(|ability| {
                        ability_view(
                            ability,
                            self.ability_on(entity, ability.id),
                            self.can_level(entity, ability.id, ability.level),
                            self.mana_cost_rate_of(Some(entity)),
                        )
                    })
                    .collect()
            }),
            items: self.inventory.get(entity).map_or_else(Vec::new, |bag| {
                item_views(bag, self.mana_cost_rate_of(Some(entity)), clocks)
            }),
            effects: self.effects_on(entity),
            town_portal: self
                .inventory
                .get(entity)
                .and_then(|bag| bag.town_portal.flatten())
                .map(|stack| item_view(&stack, self.mana_cost_rate_of(Some(entity)), clocks)),
        })
    }
}

/// Milliseconds of animation at an attack speed, clamped as the cycle
/// clamps it.
fn after_speed(ms: u32, attack_speed: i32) -> u32 {
    let speed = attack_speed.clamp(
        crate::game::rules::MIN_ATTACK_SPEED,
        crate::game::rules::MAX_ATTACK_SPEED,
    );
    ms * crate::game::rules::BASE_ATTACK_SPEED as u32 / speed as u32
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
    let def = crate::game::ability_def(held.id);
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
        can_level,
    }
}

/// A whole book on the wire, as it stands with no body under it: nothing
/// toggled on and no point to spend.
fn ability_views(book: &crate::game::AbilityBook, mana_rate_bp: i32) -> Vec<AbilityView> {
    book.slots
        .iter()
        .map(|held| ability_view(held, false, false, mana_rate_bp))
        .collect()
}

/// Which effect one kind is on the wire.
pub(crate) fn stack_effect_id(kind: StackKind) -> u16 {
    match kind {
        StackKind::FleshHeap => 10,
        StackKind::Souls => 11,
    }
}

/// The number the wire names a modifier with.
pub(crate) fn effect_id(kind: ModifierKind) -> u16 {
    match kind {
        #[cfg(test)]
        ModifierKind::Haste { .. } => 0,
        ModifierKind::Mending { .. } => 1,
        ModifierKind::Clarity { .. } => 2,
        ModifierKind::Fountain { .. } => 3,
        ModifierKind::Stunned => 4,
        ModifierKind::Shielded => 8,
        ModifierKind::Slowed { .. } => 5,
        ModifierKind::Hastened { .. } => 7,
        ModifierKind::Burning { .. } => 6,
        ModifierKind::Phased => 9,
        ModifierKind::ArmorBroken { .. } => 12,
        ModifierKind::MagicBroken { .. } => EffectId::MAGIC_BROKEN.0,
        ModifierKind::Guarded { .. } => 13,
        ModifierKind::Inspired { .. } => 14,
        ModifierKind::Shadowraze { .. } => EFFECT_SHADOWRAZE,
        ModifierKind::Rot { .. } => 16,
        ModifierKind::Feared => 17,
        ModifierKind::BloodGrenade { .. } => EffectId::BLOOD_GRENADE.0,
    }
}

impl World {
    /// Everything showing on one entity: its active modifiers but the rot,
    /// then what it has gathered.
    fn effects_on(&self, entity: Entity) -> Vec<EffectView> {
        let mut on_it: Vec<EffectView> = self
            .modifiers
            .get(entity)
            .into_iter()
            .flat_map(|on_it| on_it.active())
            .filter(|held| !matches!(held.kind, ModifierKind::Rot { .. }))
            .map(|held| EffectView {
                id: EffectId(effect_id(held.kind)),
                ticks_left: held.ticks_left,
                stacks: match held.kind {
                    ModifierKind::Shadowraze { stacks } => Some(u32::from(stacks)),
                    _ => None,
                },
            })
            .collect();
        if let Some(gathered) = self.stacks.get(entity) {
            on_it.extend(gathered.held().map(|(kind, many)| EffectView {
                id: EffectId(stack_effect_id(kind)),
                ticks_left: None,
                stacks: Some(many),
            }));
        }
        if let Some(feast) = self.feast.get(entity) {
            on_it.push(EffectView {
                id: match feast.phase {
                    FeastPhase::Feasting => EffectId::FEASTING,
                    FeastPhase::Lingering => EffectId::FEAST_LINGER,
                },
                ticks_left: Some(feast.ticks_left),
                stacks: Some(feast.souls()),
            });
        }
        on_it
    }

    /// Whether an entity has a toggle switched on right now.
    fn ability_on(&self, entity: Entity, id: bota_proto::AbilityId) -> bool {
        match id {
            crate::game::ability::ROT => self.modifiers.get(entity).is_some_and(|on_it| {
                on_it
                    .active()
                    .any(|held| matches!(held.kind, ModifierKind::Rot { .. }))
            }),
            _ => false,
        }
    }

    /// Whether a skill point could go into one ability right now.
    ///
    /// The same three conditions [`World::learn`] asks: a point unspent, room
    /// left in the ability, and a hero level high enough for the next one.
    fn can_level(&self, entity: Entity, id: bota_proto::AbilityId, level: u8) -> bool {
        let Some(def) = crate::game::ability_def(id) else {
            return false;
        };
        let hero_level = self.level.get(entity).map_or(0, |held| held.0);
        level < def.max_level
            && self.points_spent(entity) < hero_level
            && hero_level >= crate::game::level_floor(def, level)
    }

    /// The [`StatusFlags`] bits of a unit: what its modifiers do to it, and
    /// whether it hides, cannot be hurt or channels.
    fn statuses_of(&self, entity: Entity, stats: &crate::game::Stats) -> u16 {
        let mut bits = 0;
        for held in self
            .modifiers
            .get(entity)
            .into_iter()
            .flat_map(|on_it| on_it.active())
        {
            bits |= match held.kind {
                ModifierKind::Stunned => StatusFlags::STUNNED,
                ModifierKind::Shielded => StatusFlags::MAGIC_IMMUNE,
                ModifierKind::Slowed { .. } => StatusFlags::SLOWED,
                ModifierKind::Burning { .. } => StatusFlags::DOT,
                ModifierKind::Feared => StatusFlags::FEARED,
                ModifierKind::BloodGrenade { .. } => StatusFlags::SLOWED | StatusFlags::DOT,
                _ => 0,
            };
        }
        for (set, flag) in [
            (stats.hides, StatusFlags::INVISIBLE),
            (stats.invulnerable, StatusFlags::INVULNERABLE),
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
