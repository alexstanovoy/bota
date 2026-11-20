//! The neutral spells that strike: a missile, a chain, a smash and spikes.

use bota_core::{
    Act, Afflicts, ModifierKind, NeutralAbility, Spell, at_level, facing_towards, point_ahead,
    point_along, rules, segment_distance_squared, ticks_of_ms,
};
use bota_proto::{Aim, Angle, DamageKind, Fixed, Inflictor, Target, Team, Vec2};

use crate::game::{Chain, Entity, HitEffect, Modifier, Projectile, Pulser, World};

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

    /// Lets a boulder land at the spot aimed at, within the reach of the cast: every foe within
    /// its radius takes the damage and is stunned.
    pub(super) fn boulder(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let Act::Boulder {
            radius,
            damage,
            kind,
            ms,
        } = act
        else {
            return false;
        };
        let Target::Pos(spot) = target else {
            return false;
        };
        let area = rules::units(radius);
        let ms = at_level(&ms, cast.level);
        for foe in self.foes_within(cast.caster, spot, area) {
            self.stun_for(cast, foe, ms);
            self.push_hit(
                Some(cast.caster),
                foe,
                at_level(&damage, cast.level),
                kind,
                cast.inflictor(),
            );
        }
        true
    }
}
