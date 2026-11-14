//! Caster-centred expanding waves, independent for every cast.

use bota_core::{AreaWaveSpec, Reach, UseSpec, item_def, rules, widened};
use bota_proto::{DamageKind, Fixed, Inflictor, ItemId};

use crate::game::{AreaWave, Entity, Modifier, World};

// Bound a debug-mode caster's repeated activations and each cast's persistent history.
const MAX_AREA_WAVES: usize = 128;
const MAX_AREA_WAVE_TARGETS: usize = 16_384;

impl World {
    pub(crate) fn begin_area_wave(&mut self, owner: Entity, item: ItemId) -> bool {
        let count = self
            .entities
            .iter()
            .filter(|entity| self.area_wave.contains(*entity))
            .count();
        if count >= MAX_AREA_WAVES {
            return false;
        }
        let (Some(at), Some(side)) = (self.pos(owner), self.team.get(owner).copied()) else {
            return false;
        };
        let mut wave = AreaWave {
            owner,
            item,
            side,
            at,
            elapsed: 0,
            aoe_bonus: self.aoe_bonus_of(owner),
            damage_amp_bp: self.outgoing_spell_amp_bp(Some(owner), DamageKind::Magical),
            struck: Default::default(),
        };
        let Some(UseSpec::Wave(spec)) = item_def(item).map(|def| def.effect) else {
            return false;
        };
        let entities: Vec<_> = self.entities.iter().collect();
        self.strike_wave(&mut wave, spec, Fixed::ZERO, &entities);
        let entity = self.spawn();
        self.area_wave.insert(entity, wave);
        true
    }

    pub(crate) fn tick_area_waves(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let Some(mut wave) = self.area_wave.remove(entity) else {
                continue;
            };
            let Some(UseSpec::Wave(spec)) = item_def(wave.item).map(|def| def.effect) else {
                self.discard(entity);
                continue;
            };
            wave.elapsed += 1;
            wave.at = self.pos(wave.owner).unwrap_or(wave.at);
            let maximum = rules::units(widened(spec.radius, wave.aoe_bonus));
            let travelled = i64::from(rules::units(spec.speed).raw) * i64::from(wave.elapsed)
                / i64::from(rules::TICKS_PER_SECOND);
            let radius = Fixed {
                raw: travelled.min(i64::from(maximum.raw)) as i32,
            };
            self.strike_wave(&mut wave, spec, radius, &entities);
            if radius >= maximum || wave.struck.len() >= MAX_AREA_WAVE_TARGETS {
                self.discard(entity);
            } else {
                self.area_wave.insert(entity, wave);
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    fn strike_wave(
        &mut self,
        wave: &mut AreaWave,
        spec: AreaWaveSpec,
        radius: Fixed,
        entities: &[Entity],
    ) {
        let def = item_def(wave.item).expect("a wave's item remains registered");
        let template = def.modifiers[usize::from(spec.debuff)];
        let with = Inflictor::Item(wave.item);
        for target in entities.iter().copied() {
            if wave.struck.len() >= MAX_AREA_WAVE_TARGETS {
                break;
            }
            if !self.alive(target)
                || !self.reach_takes(Reach::Combatants, target)
                || self.team.get(target).copied() == Some(wave.side)
                || self
                    .stats
                    .get(target)
                    .is_none_or(|stats| stats.invulnerable)
                || self
                    .pos(target)
                    .is_none_or(|at| !at.within(wave.at, radius))
                || wave.struck.contains(&target)
            {
                continue;
            }
            wave.struck.insert(target);
            let immune = self
                .stats
                .get(target)
                .is_some_and(|stats| stats.spell_immune);
            if immune && !spec.pierces_immunity {
                continue;
            }
            let kind = template.kind(wave.item, spec.debuff, self.tick, false);
            if super::support::item_works(kind, immune) {
                self.put_modifier(
                    target,
                    Modifier::new(kind, Some(wave.owner), Some(with), Some(template.ticks)),
                );
            }
            self.push_hit_with_amp(
                Some(wave.owner),
                target,
                spec.damage,
                DamageKind::Magical,
                wave.damage_amp_bp,
                Some(with),
            );
        }
    }
}
