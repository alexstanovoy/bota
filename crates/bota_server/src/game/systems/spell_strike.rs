//! The neutral spells that strike: a missile, a chain, a smash and spikes.

use bota_core::{
    Act, Afflicts, ModifierKind, NeutralAbility, Spell, at_level, facing_towards, isqrt64,
    move_towards, per_tick, point_ahead, point_along, rules, segment_distance_squared, ticks_of_ms,
};
use bota_proto::{Aim, Angle, DamageKind, Fixed, Inflictor, Target, Team, Vec2};

use crate::game::{
    BoulderFlight, Chain, Entity, HitEffect, Modifier, Projectile, Pulser, Shockwave, Visibility,
    World,
};

use super::spell::Cast;

/// A bolt of chain lightning: it strikes for `damage` and leaps on until it has struck `targets`,
/// each leap to the nearest foe within `radius` of the last struck, `delay_ms` after the last,
/// for `keep_pct` percent of the last leap's damage.
#[derive(Clone, Copy)]
pub(super) struct Bolt {
    pub damage: i32,
    pub targets: u8,
    pub radius: i32,
    pub delay_ms: i32,
    pub keep_pct: i32,
    /// Whether it leaps on once its source is dead.
    pub life: crate::game::ChainLife,
}

impl World {
    /// Throws a missile that tracks its target and, on arriving, deals the
    /// damage and stuns or roots. It never misses.
    pub(super) fn bolt(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let Act::Bolt {
            speed,
            hero_damage,
            other_damage,
            ms,
            root,
        } = act
        else {
            return false;
        };
        let (Some(on), Some(from)) = (
            self.foe_aimed_at(cast, target),
            self.transform.get(cast.caster).copied(),
        ) else {
            return false;
        };
        let side = self.team.get(cast.caster).copied().unwrap_or(Team::Neutral);
        let ticks = ticks_of_ms(ms);
        let missile = self.spawn();
        self.transform.insert(missile, from);
        self.set_team(missile, side);
        self.projectile.insert(
            missile,
            Projectile {
                proc: None,
                speed: Fixed::from_int(speed),
                source: Some(cast.caster),
                inflictor: cast.inflictor(),
                model: None,
                target: on,
                damage: if self.is_hero(on) {
                    hero_damage
                } else {
                    other_damage
                },
                kind: DamageKind::Magical,
                damage_amp_bp: self.outgoing_spell_amp_bp(Some(cast.caster), DamageKind::Magical),
                launch_tier: self.ground.tier(from.pos),
                can_miss_uphill: false,
                pierces: true,
                pierced: Vec::new(),
                pierce_amp_bp: rules::NOMINAL_BP,
                effect: if root {
                    HitEffect::Root { ticks }
                } else {
                    HitEffect::Stun { ticks }
                },
                striker: None,
                laid: crate::game::Laid::NONE,
                riders: crate::game::Riders::NONE,
                category: crate::game::Category::Plain,
            },
        );
        true
    }

    /// Strikes the target, and sets the chain leaping on from it.
    pub(super) fn chain_lightning(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let Act::Chain {
            damage,
            targets,
            radius,
            delay_ms,
            keep_pct,
        } = act
        else {
            return false;
        };
        let (Some(on), Some(_)) = (
            self.foe_aimed_at(cast, target),
            self.team.get(cast.caster).copied(),
        ) else {
            return false;
        };
        let Some(at) = self.pos(on) else {
            return false;
        };
        let bolt = Bolt {
            damage: at_level(&damage, cast.level),
            targets,
            radius,
            delay_ms,
            keep_pct: at_level(&keep_pct, cast.level),
            life: crate::game::ChainLife::Outlives,
        };
        self.chain_from(
            cast.caster,
            Inflictor::Ability(cast.ability.id()),
            on,
            at,
            bolt,
        );
        true
    }

    /// Strikes `on`, standing at `at`, with a bolt of `source`'s laid to `inflictor`, and sets
    /// the chain leaping on from there: a neutral's Chain Lightning, an item's.
    pub(super) fn chain_from(
        &mut self,
        source: Entity,
        inflictor: Inflictor,
        on: Entity,
        at: Vec2,
        bolt: Bolt,
    ) {
        let Some(side) = self.team.get(source).copied() else {
            return;
        };
        self.push_hit(
            Some(source),
            on,
            bolt.damage,
            DamageKind::Magical,
            Some(inflictor),
        );
        if bolt.targets > 1 {
            let delay = ticks_of_ms(bolt.delay_ms).max(1);
            let marker = self.spawn_placed(at, Angle::default(), side);
            self.chain.insert(
                marker,
                Chain {
                    source,
                    inflictor,
                    at,
                    struck: vec![on],
                    leaps_left: bolt.targets - 1,
                    damage: bolt.damage * bolt.keep_pct / 100,
                    keep_pct: bolt.keep_pct,
                    radius: rules::units(bolt.radius),
                    delay,
                    wait: delay,
                    life: bolt.life,
                },
            );
        }
    }

    /// The spot `reach` from the caster towards an aim, or straight ahead of
    /// it when the aim is where it stands.
    fn spot_towards(&self, caster: Entity, aim: Vec2, reach: i32) -> Option<Vec2> {
        let from = self.pos(caster)?;
        if aim == from {
            let facing = self.transform.get(caster)?.facing;
            return Some(point_ahead(from, facing, rules::units(reach)));
        }
        Some(point_along(from, aim, rules::units(reach)))
    }

    /// Smashes the ground `reach` ahead of the caster, towards where it was
    /// aimed: damage and a share of the current health, magical, and a stun.
    pub(super) fn smash(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let Act::Smash {
            reach,
            radius,
            damage,
            current_pct,
            ms,
        } = act
        else {
            return false;
        };
        let Target::Pos(aim) = target else {
            return false;
        };
        let Some(spot) = self.spot_towards(cast.caster, aim, reach) else {
            return false;
        };
        let area = rules::units(at_level(&radius, cast.level));
        for foe in self.foes_within(cast.caster, spot, area) {
            let current = self.health.get(foe).map_or(0, |health| health.hp.to_int());
            let blow = at_level(&damage, cast.level) + current * current_pct / 100;
            self.stun_for(cast, foe, ms);
            self.push_hit(
                Some(cast.caster),
                foe,
                blow,
                DamageKind::Magical,
                cast.inflictor(),
            );
        }
        true
    }

    /// Strikes at the spot ahead of the caster as the cast goes off, and sets the pulses after it
    /// going.
    pub(super) fn pulses(&mut self, cast: Cast, act: Act) -> bool {
        let Act::Pulses {
            reach,
            count,
            interval_ms,
            ..
        } = act
        else {
            return false;
        };
        let (Some(from), Some(side)) = (
            self.transform.get(cast.caster).copied(),
            self.team.get(cast.caster).copied(),
        ) else {
            return false;
        };
        assert!(count > 0, "a pulsing spell strikes at least once");
        let spot = point_ahead(from.pos, from.facing, rules::units(reach));
        let interval = ticks_of_ms(interval_ms).max(1);
        let marker = self.spawn_placed(spot, Angle::default(), side);
        let pulser = Pulser {
            source: cast.caster,
            ability: cast.ability.id(),
            level: cast.level,
            left: count - 1,
            interval,
            due: self.tick + interval,
        };
        self.pulse(marker, pulser);
        if pulser.left > 0 {
            self.pulser.insert(marker, pulser);
        } else {
            self.discard(marker);
        }
        true
    }

    /// Strikes everything of the other side within the spell's radius of the marker.
    fn pulse(&mut self, marker: Entity, pulser: Pulser) {
        let Some(ability) = NeutralAbility::of(pulser.ability) else {
            return;
        };
        let Spell::Cast {
            act:
                Act::Pulses {
                    radius,
                    damage,
                    slow_ms,
                    ..
                },
            ..
        } = *ability.spell()
        else {
            return;
        };
        let Some(spot) = self.pos(marker) else {
            return;
        };
        let cast = Cast {
            caster: pulser.source,
            ability,
            level: pulser.level,
        };
        for foe in self.foes_within(marker, spot, rules::units(radius)) {
            if !self.spell_reaches(cast.ability.id(), foe) {
                continue;
            }
            self.wear(cast, foe, slow_ms, 0);
            self.push_hit(
                Some(cast.caster),
                foe,
                at_level(&damage, cast.level),
                DamageKind::Magical,
                cast.inflictor(),
            );
        }
    }

    /// Strikes the pulse each pulsing spell is due.
    pub(super) fn tick_pulsers(&mut self) {
        let entities = self.take_entity_snapshot();
        for marker in entities.iter().copied() {
            let Some(mut pulser) = self.pulser.get(marker).copied() else {
                continue;
            };
            if self.tick < pulser.due {
                continue;
            }
            self.pulse(marker, pulser);
            pulser.left -= 1;
            pulser.due += pulser.interval;
            if pulser.left == 0 {
                self.pulser.remove(marker);
                self.discard(marker);
            } else {
                self.pulser.insert(marker, pulser);
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Raises spikes in lines from the caster: centred on the aim, or, for
    /// a spell cast at nothing, round the caster from where it faces. Every
    /// foe within the width of a line takes the damage and the stun once.
    pub(super) fn spikes(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let Act::Spikes {
            lines,
            spread_deg,
            length,
            width,
            damage,
            afflicts,
            ms,
        } = act
        else {
            return false;
        };
        let Some(from) = self.pos(cast.caster) else {
            return false;
        };
        let around = cast.ability.entry().def.aim == Aim::Own;
        let base = match target {
            Target::Pos(aim) if aim != from => facing_towards(from, aim),
            _ => self
                .transform
                .get(cast.caster)
                .map_or(Angle::default(), |t| t.facing),
        };
        let step = spread_deg * 65536 / 360;
        let first = if around {
            0
        } else {
            -step * (i32::from(lines) - 1) / 2
        };
        let mut struck = Vec::new();
        for line in 0..i32::from(lines) {
            let facing = Angle {
                brads: (i32::from(base.brads) + first + step * line) as u16,
            };
            let end = point_ahead(from, facing, rules::units(length));
            let reach = rules::units(width).squared_raw();
            for foe in self.entities.iter().collect::<Vec<_>>() {
                let near = self
                    .pos(foe)
                    .is_some_and(|pos| segment_distance_squared(pos, from, end) <= reach);
                if near && !struck.contains(&foe) && self.is_foe(cast.caster, foe) {
                    struck.push(foe);
                }
            }
        }
        let ms = at_level(&ms, cast.level);
        for foe in struck {
            match afflicts {
                Afflicts::Stun => self.stun_for(cast, foe, ms),
                Afflicts::Break => self.break_for(cast, foe, ms),
            }
            let blow = at_level(&damage, cast.level);
            self.push_hit(
                Some(cast.caster),
                foe,
                blow,
                DamageKind::Magical,
                cast.inflictor(),
            );
        }
        true
    }

    /// A Break on a foe for `ms`: its passives are silenced.
    fn break_for(&mut self, cast: Cast, on: Entity, ms: i32) {
        let held = Modifier::new(
            ModifierKind::Broken,
            Some(cast.caster),
            cast.inflictor(),
            Some(ticks_of_ms(ms)),
        );
        self.put_modifier(on, held);
    }

    /// Launches a point-targeted boulder. Travel time scales with distance through cast range.
    pub(super) fn boulder(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let (Act::Boulder { min_ms, max_ms, .. }, Target::Pos(aim), Some(origin)) =
            (act, target, self.pos(cast.caster))
        else {
            return false;
        };
        let range = self.reach_of(cast.caster, cast.ability.entry().def.range);
        let range = i64::from(rules::units(range.max(1)).raw);
        let distance = isqrt64(origin.distance_squared(aim)).min(range);
        let milliseconds =
            i64::from(min_ms) + (i64::from(max_ms) - i64::from(min_ms)) * distance / range;
        let ticks = ticks_of_ms(i32::try_from(milliseconds).expect("bounded travel time"));
        assert!(ticks > 0);
        let shot = self.spawn_mark(cast.ability.id(), cast.caster, origin, 0);
        self.transform
            .get_mut(shot)
            .expect("a placed boulder")
            .facing = facing_towards(origin, aim);
        self.visibility.insert(shot, Visibility::NONE);
        self.boulder.insert(
            shot,
            BoulderFlight {
                origin,
                aim,
                launched: self.tick,
                ticks,
                level: cast.level,
            },
        );
        true
    }

    pub(crate) fn tick_boulders(&mut self) {
        if self.boulder.is_empty() {
            return;
        }
        let entities = self.take_entity_snapshot();
        for shot in entities.iter().copied() {
            let Some(boulder) = self.boulder.get(shot).copied() else {
                continue;
            };
            let elapsed = self.tick.saturating_sub(boulder.launched);
            if elapsed >= boulder.ticks {
                self.boulder_impact(shot, boulder);
                self.boulder.remove(shot);
                self.mark.remove(shot);
                self.discard(shot);
                continue;
            }
            let progress = Fixed::from_ratio(elapsed as i32, boulder.ticks as i32);
            let delta = boulder.aim - boulder.origin;
            self.transform.get_mut(shot).expect("a placed boulder").pos = boulder.origin
                + Vec2 {
                    x: delta.x * progress,
                    y: delta.y * progress,
                };
        }
        self.recycle_entity_snapshot(entities);
    }

    fn boulder_impact(&mut self, shot: Entity, boulder: BoulderFlight) {
        let mark = self
            .mark
            .get(shot)
            .copied()
            .expect("the boulder has a spell source");
        let ability = NeutralAbility::of(mark.ability).expect("a neutral ability");
        let Spell::Cast {
            act:
                Act::Boulder {
                    radius,
                    damage,
                    kind,
                    ms,
                    ..
                },
            ..
        } = *ability.spell()
        else {
            unreachable!("only boulder casts release boulders");
        };
        let cast = Cast {
            caster: mark.owner,
            ability,
            level: boulder.level,
        };
        let ms = at_level(&ms, cast.level);
        for foe in self.foes_within(shot, boulder.aim, rules::units(radius)) {
            self.stun_for(cast, foe, ms);
            self.push_hit(
                Some(cast.caster),
                foe,
                at_level(&damage, cast.level),
                kind,
                cast.inflictor(),
            );
        }
    }

    pub(super) fn start_shockwave(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let (
            Act::Shockwave {
                distance,
                speed,
                start_radius,
                end_radius,
                damage,
            },
            Target::Pos(aim),
            Some(caster),
        ) = (act, target, self.transform.get(cast.caster).copied())
        else {
            return false;
        };
        let facing = if caster.pos == aim {
            caster.facing
        } else {
            facing_towards(caster.pos, aim)
        };
        let wave = self.spawn_mark(cast.ability.id(), cast.caster, caster.pos, 0);
        self.transform.get_mut(wave).expect("a placed wave").facing = facing;
        self.visibility.insert(wave, Visibility::NONE);
        let distance = rules::units(distance);
        self.shockwave.insert(
            wave,
            Shockwave {
                aim: point_ahead(caster.pos, facing, distance),
                distance,
                speed: rules::units(speed),
                travelled: Fixed::ZERO,
                start_radius: self.widened_of(cast.caster, start_radius),
                end_radius: self.widened_of(cast.caster, end_radius),
                damage: at_level(&damage, cast.level),
                damage_amp_bp: self.outgoing_spell_amp_bp(Some(cast.caster), DamageKind::Magical),
                struck: Vec::new(),
            },
        );
        true
    }

    pub(crate) fn tick_shockwaves(&mut self) {
        if self.shockwave.is_empty() {
            return;
        }
        let entities = self.take_entity_snapshot();
        for marker in entities.iter().copied() {
            let Some(mut wave) = self.shockwave.remove(marker) else {
                continue;
            };
            let mark = self
                .mark
                .get(marker)
                .copied()
                .expect("the wave has a spell source");
            let at = self.pos(marker).expect("a placed wave");
            let step = per_tick(wave.speed);
            let next = move_towards(at, wave.aim, step);
            self.transform.get_mut(marker).expect("a placed wave").pos = next;
            wave.travelled = (wave.travelled + step).min(wave.distance);
            let width = wave.start_radius
                + (wave.end_radius - wave.start_radius) * (wave.travelled / wave.distance);
            for target in entities.iter().copied() {
                if wave.struck.contains(&target) || !self.burns((marker, mark.ability), target) {
                    continue;
                }
                let hull = self.hull.get(target).map_or(Fixed::ZERO, |hull| hull.bound);
                if self.pos(target).is_some_and(|pos| {
                    segment_distance_squared(pos, at, next) <= (width + hull).squared_raw()
                }) {
                    self.push_hit_with_amp(
                        Some(mark.owner),
                        target,
                        wave.damage,
                        DamageKind::Magical,
                        wave.damage_amp_bp,
                        Some(Inflictor::Ability(mark.ability)),
                    );
                    wave.struck.push(target);
                }
            }
            if wave.travelled < wave.distance {
                self.shockwave.insert(marker, wave);
            } else {
                self.mark.remove(marker);
                self.discard(marker);
            }
        }
        self.recycle_entity_snapshot(entities);
    }
}
