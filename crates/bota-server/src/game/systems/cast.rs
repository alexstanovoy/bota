//! Casting: what each ability does, and what stops a cast happening.

use bota_proto::{DamageKind, EventKind, Fixed, Target, Team, Vec2};

use crate::game::{
    Entity, PendingCast, Projectile, Status, StatusKind, World, ability, ability_cooldown,
    ability_mana_cost, wire_id,
};
use crate::game::{Event, rules};

impl World {
    /// Whether an entity stands in its own shop.
    pub fn at_shop(&self, entity: Entity) -> bool {
        let (Some(at), Some(side)) = (
            self.transform.get(entity).map(|t| t.pos),
            self.team.get(entity).copied(),
        ) else {
            return false;
        };
        at.within(
            crate::game::fountain_pos(self.map, side),
            rules::units(rules::SHOP_RANGE),
        )
    }

    /// Starts whatever cast each entity is waiting to make.
    ///
    /// A cast waits on its own cost: the level must be learned, the cooldown
    /// run out, and the mana be there. What it cannot pay for is dropped
    /// rather than held. What is aimed further off than it reaches is held
    /// instead: movement walks the caster in, and the cast goes off when it
    /// arrives.
    pub fn run_casts(&mut self, events: &mut Vec<Event>) {
        for entity in self.entities.iter().collect::<Vec<_>>() {
            let Some(cast) = self.casting.get(entity).copied() else {
                continue;
            };
            if self.held(entity) {
                continue;
            }
            if self.walking_into_cast(entity, cast) {
                continue;
            }
            self.casting.remove(entity);
            let slot = usize::from(cast.slot.0);
            let Some(ability) = self
                .abilities
                .get(entity)
                .and_then(|book| book.slots.get(slot))
                .copied()
            else {
                continue;
            };
            let Some(def) = crate::game::ability_def(ability.id) else {
                continue;
            };
            if def.passive || ability.level == 0 || ability.cooldown > 0 {
                continue;
            }
            let level = usize::from(ability.level - 1);
            let cost = ability_mana_cost(ability.id, ability.level);
            if self.mana.get(entity).map_or(0, |m| m.mana.to_int()) < cost {
                continue;
            }
            let cooldown = ability_cooldown(ability.id, ability.level);
            let went = match ability.id {
                ability::FRENZY => self.cast_frenzy(entity, level),
                ability::BOUNCE => self.cast_bounce(entity, level, cast.target),
                ability::VOLLEY => self.cast_multishot(entity, level.min(2)),
                ability::MEAT_HOOK => self.cast_hook(entity, level, cast.target),
                ability::ROT => self.toggle_rot(entity, level),
                ability::DISMEMBER => self.cast_dismember(entity, level, cast.target),
                ability::BURST => self.courier_burst(entity),
                ability::RETURN_ITEMS => self.courier_return_items(entity),
                ability::SHIELD => self.courier_shield(entity),
                ability::TAKE_STASH => self.courier_take_stash(entity),
                ability::DELIVER => self.courier_deliver(entity),
                ability::RAZE_NEAR => self.cast_raze(entity, level, 0),
                ability::RAZE_MID => self.cast_raze(entity, level, 1),
                ability::RAZE_FAR => self.cast_raze(entity, level, 2),
                ability::REQUIEM => self.cast_requiem(entity, level.min(2)),
                _ => false,
            };
            if !went {
                continue;
            }
            self.charge_nearby_items(entity);
            if let Some(mana) = self.mana.get_mut(entity) {
                mana.mana -= Fixed::from_int(cost);
            }
            if let Some(book) = self.abilities.get_mut(entity)
                && let Some(ability) = book.slots.get_mut(slot)
            {
                ability.cooldown = cooldown;
            }
            let at = self
                .transform
                .get(entity)
                .map_or(bota_proto::Vec2::ZERO, |t| t.pos);
            let side = self.team.get(entity).copied().unwrap_or(Team::Neutral);
            events.push(Event {
                kind: EventKind::AbilityCast {
                    caster: wire_id(entity),
                    ability: ability.id,
                },
                visible_to: self.who_may_know(at, side),
            });
        }
    }

    /// Gives every enemy item near a cast one charge of what it may hold.
    ///
    /// Only a cast by a hero is worth a charge, and only a hero standing
    /// within [`rules::MAGIC_CHARGE_RANGE`] of it takes one.
    fn charge_nearby_items(&mut self, caster: Entity) {
        if self.kind.get(caster).copied() != Some(bota_proto::UnitKind::Hero) {
            return;
        }
        let (Some(at), Some(side)) = (
            self.transform.get(caster).map(|t| t.pos),
            self.team.get(caster).copied(),
        ) else {
            return;
        };
        let reach = rules::units(rules::MAGIC_CHARGE_RANGE);
        for other in self.entities.iter().collect::<Vec<_>>() {
            if self.team.get(other).copied() == Some(side)
                || self.kind.get(other).copied() != Some(bota_proto::UnitKind::Hero)
                || !self
                    .transform
                    .get(other)
                    .is_some_and(|t| t.pos.within(at, reach))
            {
                continue;
            }
            let Some(bag) = self.inventory.get_mut(other) else {
                continue;
            };
            for stack in bag.slots.iter_mut().flatten() {
                let Some(def) = crate::game::item_def(stack.id) else {
                    continue;
                };
                if def.cast_charges > 0 {
                    stack.charges = stack.charges.saturating_add(1).min(def.cast_charges);
                }
            }
        }
    }

    /// Puts haste on the caster for a while.
    fn cast_frenzy(&mut self, caster: Entity, level: usize) -> bool {
        let mut on_it = self.statuses.remove(caster).unwrap_or_default();
        on_it.put(Status {
            kind: StatusKind::Haste {
                speed: rules::SYLLA_FRENZY_ATTACK_SPEED[level],
            },
            ticks_left: rules::SYLLA_FRENZY_TICKS,
        });
        self.statuses.insert(caster, on_it);
        true
    }

    /// Throws a missile that goes on to the next enemy after each hit.
    fn cast_bounce(&mut self, caster: Entity, level: usize, target: Target) -> bool {
        let Target::Unit(target) = target else {
            return false;
        };
        let Some(mark) = self.of_wire(target) else {
            return false;
        };
        if !self.hostile(caster, mark) {
            return false;
        }
        let range = rules::units(rules::SYLLA_BOUNCE_CAST_RANGE);
        if !self.reachable(caster, range, mark) {
            return false;
        }
        let (Some(at), Some(side)) = (
            self.transform.get(caster).copied(),
            self.team.get(caster).copied(),
        ) else {
            return false;
        };
        let missile = self.spawn();
        self.transform.insert(missile, at);
        self.set_team(missile, side);
        self.projectile.insert(
            missile,
            Projectile {
                speed: Fixed::from_int(rules::SYLLA_BOUNCE_SPEED),
                source: Some(caster),
                target: mark,
                damage: rules::SYLLA_BOUNCE_DAMAGE[level],
                kind: DamageKind::Magical,
                ability: Some(bota_proto::AbilityId(2)),
                launch_tier: 0,
                crit: false,
                bounces_left: rules::SYLLA_BOUNCE_COUNT[level],
                bounce_range: rules::SYLLA_BOUNCE_RANGE,
                bounced: vec![mark],
            },
        );
        true
    }

    /// Strikes every enemy standing near the caster at once.
    fn cast_multishot(&mut self, caster: Entity, level: usize) -> bool {
        let Some(at) = self.transform.get(caster).map(|t| t.pos) else {
            return false;
        };
        let damage = self.stats.get(caster).map_or(0, |s| s.damage)
            * rules::SYLLA_MULTI_DMG_PCT[level]
            / 100;
        let radius = rules::units(rules::SYLLA_MULTI_RADIUS);
        let struck: Vec<Entity> = self
            .entities
            .iter()
            .filter(|other| {
                self.hostile(caster, *other)
                    && self
                        .transform
                        .get(*other)
                        .is_some_and(|t| t.pos.within(at, radius))
            })
            .collect();
        for mark in struck {
            self.push_hit(Some(caster), mark, damage, DamageKind::Physical);
        }
        true
    }

    /// Points an entity at the cast it was ordered to make.
    pub fn order_cast(&mut self, entity: Entity, cast: PendingCast) {
        self.casting.insert(entity, cast);
    }
}

impl World {
    /// Whether a cast is still being walked into rather than made.
    ///
    /// A cast aimed at something out of reach is kept and the caster sent at
    /// it; one aimed at nothing in particular is never walked into.
    pub fn walking_into_cast(&mut self, entity: Entity, cast: PendingCast) -> bool {
        let Some(at) = self.cast_spot(entity, cast) else {
            return false;
        };
        let Some(from) = self.transform.get(entity).map(|t| t.pos) else {
            return false;
        };
        let reach =
            crate::game::ability_def(self.ability_in(entity, cast.slot)).map_or(0, |def| def.range);
        if reach == 0 || from.within(at, rules::units(reach)) {
            return false;
        }
        true
    }

    /// Where a cast is aimed, if it is aimed anywhere at all.
    pub fn cast_spot(&self, entity: Entity, cast: PendingCast) -> Option<Vec2> {
        match cast.target {
            Target::Pos(pos) => Some(pos),
            Target::Unit(target) => {
                let on = self.of_wire(target)?;
                self.alive(on)
                    .then(|| self.transform.get(on).map(|t| t.pos))
                    .flatten()
            }
            Target::None => {
                let _ = entity;
                None
            }
        }
    }

    /// Which ability sits in one of an entity's slots.
    pub fn ability_in(
        &self,
        entity: Entity,
        slot: bota_proto::AbilitySlot,
    ) -> bota_proto::AbilityId {
        self.abilities
            .get(entity)
            .and_then(|book| book.slots.get(usize::from(slot.0)))
            .map_or(bota_proto::AbilityId(u16::MAX), |held| held.id)
    }
}
