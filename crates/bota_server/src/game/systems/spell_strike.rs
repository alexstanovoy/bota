//! The neutral spells that strike: a missile, a chain, a smash and spikes.

use bota_core::{
    Act, at_level, facing_towards, point_ahead, point_along, rules, segment_distance_squared,
    ticks_of_ms,
};
use bota_proto::{Aim, Angle, DamageKind, Fixed, Target, Team, Vec2};

use crate::game::{Chain, Entity, HitEffect, Projectile, World};

use super::spell::Cast;

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
                speed: Fixed::from_int(speed),
                source: Some(cast.caster),
                target: on,
                damage: if self.is_hero(on) {
                    hero_damage
                } else {
                    other_damage
                },
                kind: DamageKind::Magical,
                damage_amp_bp: self.outgoing_damage_amp_bp(Some(cast.caster), DamageKind::Magical),
                launch_tier: self.ground.tier(from.pos),
                can_miss_uphill: false,
                pierces: true,
                pierce_damage: 0,
                pierce_amp_bp: rules::NOMINAL_BP,
                effect: if root {
                    HitEffect::Root { ticks }
                } else {
                    HitEffect::Stun { ticks }
                },
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
        let (Some(on), Some(side)) = (
            self.foe_aimed_at(cast, target),
            self.team.get(cast.caster).copied(),
        ) else {
            return false;
        };
        let Some(at) = self.pos(on) else {
            return false;
        };
        let damage = at_level(&damage, cast.level);
        self.push_hit(Some(cast.caster), on, damage, DamageKind::Magical);
        if targets > 1 {
            let delay = ticks_of_ms(delay_ms).max(1);
            let keep_pct = at_level(&keep_pct, cast.level);
            let marker = self.spawn_placed(at, Angle::default(), side);
            self.chain.insert(
                marker,
                Chain {
                    source: cast.caster,
                    at,
                    struck: vec![on],
                    leaps_left: targets - 1,
                    damage: damage * keep_pct / 100,
                    keep_pct,
                    radius: rules::units(radius),
                    delay,
                    wait: delay,
                },
            );
        }
        true
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
            self.push_hit(Some(cast.caster), foe, blow, DamageKind::Magical);
        }
        true
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
        for foe in struck {
            self.stun_for(cast, foe, ms);
            let blow = at_level(&damage, cast.level);
            self.push_hit(Some(cast.caster), foe, blow, DamageKind::Magical);
        }
        true
    }
}
