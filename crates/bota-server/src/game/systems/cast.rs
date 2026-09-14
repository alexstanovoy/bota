//! Casting abilities: what starts one, what it costs, and what each one does.

use bota_proto::{AbilitySlot, DamageKind, EventKind, Fixed, Target, Team};

use crate::game::{
    Entity, Modifier, ModifierKind, Projectile, World, ability_cooldown, ability_mana_cost, wire_id,
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

    /// Casts one of an entity's abilities to the moment it has gone off:
    /// the checks, the ability's own work, and the cost.
    ///
    /// False when it did not go off, and then nothing was spent: the level
    /// must be learned, the cooldown run out, the mana be there, and the
    /// ability itself must have found something to do.
    pub fn begin_ability(&mut self, entity: Entity, slot: AbilitySlot, target: Target) -> bool {
        let at = usize::from(slot.0);
        let Some(ability) = self
            .abilities
            .get(entity)
            .and_then(|book| book.slots.get(at))
            .copied()
        else {
            return false;
        };
        let Some(def) = crate::game::ability_def(ability.id) else {
            return false;
        };
        if def.passive || ability.level == 0 || ability.cooldown > 0 {
            return false;
        }
        let cost = ability_mana_cost(ability.id, ability.level);
        if self.mana.get(entity).map_or(0, |m| m.mana.to_int()) < cost {
            return false;
        }
        let cooldown = ability_cooldown(ability.id, ability.level);
        if !(def.on_cast)(self, entity, target) {
            return false;
        }
        self.charge_nearby_items(entity);
        if let Some(mana) = self.mana.get_mut(entity) {
            mana.mana -= Fixed::from_int(cost);
        }
        if let Some(book) = self.abilities.get_mut(entity)
            && let Some(ability) = book.slots.get_mut(at)
        {
            ability.cooldown = cooldown;
        }
        let at = self
            .transform
            .get(entity)
            .map_or(bota_proto::Vec2::ZERO, |t| t.pos);
        let side = self.team.get(entity).copied().unwrap_or(Team::Neutral);
        let visible_to = self.who_may_know(at, side);
        self.events.push(Event {
            kind: EventKind::AbilityCast {
                caster: wire_id(entity),
                ability: ability.id,
            },
            visible_to,
        });
        true
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
    pub fn cast_frenzy(&mut self, caster: Entity, level: usize) -> bool {
        self.put_modifier(
            caster,
            Modifier {
                kind: ModifierKind::Haste {
                    speed: rules::SYLLA_FRENZY_ATTACK_SPEED[level],
                },
                source: Some(caster),
                ticks_left: Some(rules::SYLLA_FRENZY_TICKS),
            },
        );
        true
    }

    /// Throws a missile that goes on to the next enemy after each hit.
    pub fn cast_bounce(&mut self, caster: Entity, level: usize, target: Target) -> bool {
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
                can_miss_uphill: false,
                crit: false,
                bounces_left: rules::SYLLA_BOUNCE_COUNT[level],
                bounce_range: rules::SYLLA_BOUNCE_RANGE,
                bounced: vec![mark],
            },
        );
        true
    }

    /// Strikes every enemy standing near the caster at once.
    pub fn cast_multishot(&mut self, caster: Entity, level: usize) -> bool {
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
