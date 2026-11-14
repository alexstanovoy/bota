//! Area recovery and modifier grants, using the ordinary item and aura paths.

use bota_core::{AreaSupport, ItemDef, ModifierKind, cooldown_after, effect_def, rules};
use bota_proto::{Inflictor, ItemId};

use crate::game::{Dispel, DispelPower, Entity, Modifier, World};

impl World {
    pub(crate) fn support_allies(
        &mut self,
        user: Entity,
        item: ItemId,
        def: &ItemDef,
        support: AreaSupport,
    ) -> bool {
        let (Some(at), Some(side)) = (self.pos(user), self.team.get(user).copied()) else {
            return false;
        };
        if support.dispel_self {
            self.dispel(user, Dispel::Debuffs, DispelPower::Basic);
            self.refresh_stats_of(user);
        }
        let reach = self.widened_of(user, support.radius);
        let rate = self
            .stats
            .get(user)
            .map_or(rules::NOMINAL_BP, |stats| stats.cooldown_rate_bp);
        let interval = cooldown_after(def.cooldown, rate).saturating_sub(support.lockout_margin);
        // Uses run inside the action pass, which already owns the shared snapshot.
        let entities: Vec<_> = self.entities.iter().collect();
        for target in entities {
            if !self.support_reaches(user, target, support.reaches, (at, reach), side) {
                continue;
            }
            let locked = self.support_locked(target, def, support);
            if locked && support.lockout_scope == bota_core::SupportLockout::All {
                continue;
            }
            self.support_one(user, target, (item, def), support, locked, interval);
        }
        true
    }

    fn support_reaches(
        &self,
        user: Entity,
        target: Entity,
        reaches: bota_core::Reach,
        (at, radius): (bota_proto::Vec2, bota_proto::Fixed),
        side: bota_proto::Team,
    ) -> bool {
        !(reaches == bota_core::Reach::Bearer && target != user)
            && self.team.get(target).copied() == Some(side)
            && self.alive(target)
            && self.reach_takes(reaches, target)
            && self
                .stats
                .get(target)
                .is_some_and(|stats| !stats.invulnerable)
            && self.pos(target).is_some_and(|pos| pos.within(at, radius))
    }

    fn support_one(
        &mut self,
        user: Entity,
        target: Entity,
        (item, def): (ItemId, &ItemDef),
        support: AreaSupport,
        locked: bool,
        interval: u32,
    ) {
        let with = Inflictor::Item(item);
        let (health, mana) = self.refill_pools(
            target,
            if locked { 0 } else { support.health },
            support.mana,
        );
        if health > 0 || mana > 0 {
            self.tell_healed((user, with), target, health, mana);
        }
        for &index in support.grants {
            let template = def.modifiers[usize::from(index)];
            let per_source = effect_def(template.effect).is_some_and(|effect| effect.per_source);
            let fresh = !self.modifiers.get(target).is_some_and(|on_it| {
                on_it.active().any(|held| {
                    held.kind.effect_id() == template.effect
                        && (!per_source || held.source == Some(user))
                })
            });
            self.put_modifier(
                target,
                Modifier::new(
                    template.kind(item, index, self.tick, false),
                    Some(user),
                    Some(with),
                    Some(template.ticks),
                ),
            );
            if fresh && let Some(steal) = template.movement_steal {
                let radius = self
                    .modifiers
                    .get(target)
                    .and_then(|on_it| {
                        on_it.active().find(|held| {
                            held.kind.effect_id() == template.effect
                                && (!per_source || held.source == Some(user))
                        })
                    })
                    .and_then(|held| held.cached_radius)
                    .expect("a created movement effect captures its radius");
                self.steal_movement(target, item, steal, radius);
            }
        }
        if !locked && let Some(index) = support.lockout {
            let template = def.modifiers[usize::from(index)];
            self.put_modifier(
                target,
                Modifier::new(
                    template.kind(item, index, self.tick, false),
                    Some(user),
                    Some(with),
                    Some(interval),
                ),
            );
        }
        self.refresh_stats_of(target);
    }

    pub(crate) fn steal_movement(
        &mut self,
        user: Entity,
        item: ItemId,
        steal: bota_core::MovementSteal,
        radius: i32,
    ) {
        let (Some(at), Some(side), Some(def)) = (
            self.pos(user),
            self.team.get(user).copied(),
            bota_core::item_def(item),
        ) else {
            return;
        };
        let radius = rules::units(radius);
        let enemies: Vec<_> = self
            .entities
            .iter()
            .filter(|&target| {
                self.alive(target)
                    && self.kind.get(target) == Some(&bota_proto::UnitKind::Hero)
                    && self.team.get(target).copied() != Some(side)
                    && self
                        .stats
                        .get(target)
                        .is_some_and(|stats| !stats.invulnerable && !stats.spell_immune)
                    && self.pos(target).is_some_and(|pos| pos.within(at, radius))
            })
            .collect();
        for target in enemies.iter().copied() {
            self.stack_item_buff(user, target, item, steal.debuff, 1, def);
        }
        if !enemies.is_empty() {
            let template = def.modifiers[usize::from(steal.buff)];
            let kind = ModifierKind::ItemBuff {
                item,
                index: steal.buff,
                amount: enemies.len() as i32,
                since: self.tick,
            };
            self.put_modifier(
                user,
                Modifier::new(
                    kind,
                    Some(user),
                    Some(Inflictor::Item(item)),
                    Some(template.ticks),
                ),
            );
        }
    }

    fn stack_item_buff(
        &mut self,
        user: Entity,
        target: Entity,
        item: ItemId,
        index: u8,
        more: i32,
        def: &ItemDef,
    ) {
        let template = def.modifiers[usize::from(index)];
        let per_source = effect_def(template.effect).is_some_and(|effect| effect.per_source);
        let previous = self
            .modifiers
            .get(target)
            .and_then(|on_it| {
                on_it.active().find(|held| {
                    held.kind.effect_id() == template.effect
                        && (!per_source || held.source == Some(user))
                })
            })
            .copied();
        let mut fresh = previous.unwrap_or_else(|| {
            Modifier::new(
                template.kind(item, index, self.tick, false),
                Some(user),
                Some(Inflictor::Item(item)),
                Some(template.ticks),
            )
        });
        let count = match previous.map(|held| held.kind) {
            Some(ModifierKind::ItemBuff { amount, .. }) => amount,
            _ => 0,
        }
        .saturating_add(more)
        .min(i32::from(u16::MAX));
        if let ModifierKind::ItemBuff { amount, since, .. } = &mut fresh.kind {
            *amount = count;
            *since = self.tick;
        }
        fresh.ticks_left = Some(template.ticks);
        self.put_modifier(target, fresh);
    }

    fn support_locked(&self, target: Entity, def: &ItemDef, support: AreaSupport) -> bool {
        let Some(index) = support.lockout else {
            return false;
        };
        let family = def.modifiers[usize::from(index)]
            .lockout_family
            .expect("a lockout has a family");
        self.modifiers.get(target).is_some_and(|on_it| {
            on_it.active().any(|held| {
                bota_core::item_modifier(held.kind).and_then(|template| template.lockout_family)
                    == Some(family)
            })
        })
    }
}

/// Item immunity is a property of the template, while effect polarity stays in the registry.
pub(crate) fn item_works(kind: ModifierKind, spell_immune: bool) -> bool {
    bota_core::item_modifier(kind).is_none_or(|template| {
        !spell_immune
            || template.pierces_immunity
            || !effect_def(template.effect).is_some_and(|effect| effect.harmful)
    })
}
