//! Snowball Shot: a hit that bounces from unit to unit, of either side.

use bota_core::{Act, NeutralAbility, Spell, at_level, rules, ticks_of_ms};
use bota_proto::{Angle, DamageKind, Fixed, Target};

use crate::game::{Bouncer, Entity, Purpose, World, wire_id};

use super::spell::Cast;

impl World {
    /// Hits the unit aimed at, and sets the shot bouncing on from it. The missile's flight is not
    /// modelled: the hit lands as the cast goes off.
    pub(super) fn ricochet(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let Act::Ricochet {
            bounces, delay_ms, ..
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
        self.seed_hit(cast, on);
        let left = at_level(&bounces, cast.level);
        if left > 0 {
            let marker = self.spawn_placed(at, Angle::default(), side);
            self.bouncer.insert(
                marker,
                Bouncer {
                    source: cast.caster,
                    ability: cast.ability.id(),
                    level: cast.level,
                    last: on,
                    left: left.clamp(0, i32::from(u8::MAX)) as u8,
                    due: self.tick + ticks_of_ms(delay_ms).max(1),
                },
            );
        }
        true
    }

    /// What the shot does to a unit it reaches: damage and a slow to a foe of the caster, a heal
    /// to any other.
    fn seed_hit(&mut self, cast: Cast, on: Entity) {
        let Spell::Cast {
            act: Act::Ricochet {
                damage, slow_ms, ..
            },
            ..
        } = *cast.ability.spell()
        else {
            return;
        };
        let amount = at_level(&damage, cast.level);
        if self.is_foe(cast.caster, on) {
            self.wear(cast, on, slow_ms, 0);
            self.push_hit(
                Some(cast.caster),
                on,
                amount,
                DamageKind::Magical,
                cast.inflictor(),
            );
        } else {
            self.mend_part(on, Fixed::from_int(amount));
        }
    }

    /// The units a shot may bounce to from `last`, the unit it hit: heroes and creeps of either side that stand
    /// within `range` of it, in slot order, but for `last` itself.
    fn bounce_candidates(&self, last: Entity, range: i32) -> Vec<Entity> {
        let Some(at) = self.pos(last) else {
            return Vec::new();
        };
        let reach = rules::units(range);
        self.entities
            .iter()
            .filter(|other| {
                *other != last
                    && self.alive(*other)
                    && self.targetable_unit(*other)
                    && self.pos(*other).is_some_and(|pos| pos.within(at, reach))
            })
            .collect()
    }

    /// Whether a unit is one a bouncing shot lands on: a hero or a creep, not a building, a ward
    /// or anything damage passes by.
    fn targetable_unit(&self, unit: Entity) -> bool {
        self.kind.get(unit).is_some_and(|kind| {
            let def = bota_core::kind_def(*kind);
            !def.structure && !def.fixed
        }) && !self.stats.get(unit).is_some_and(|stats| stats.invulnerable)
    }

    /// Bounces each shot that is due: to a unit picked uniformly at random from the candidates,
    /// and no further when there is none.
    pub(super) fn tick_bouncers(&mut self) {
        let entities = self.take_entity_snapshot();
        for marker in entities.iter().copied() {
            let Some(mut shot) = self.bouncer.get(marker).copied() else {
                continue;
            };
            if self.tick < shot.due {
                continue;
            }
            let next = NeutralAbility::of(shot.ability).and_then(|ability| {
                let Spell::Cast {
                    act:
                        Act::Ricochet {
                            range, delay_ms, ..
                        },
                    ..
                } = *ability.spell()
                else {
                    return None;
                };
                let found = self.bounce_candidates(shot.last, range);
                let count = u32::try_from(found.len()).ok().filter(|count| *count > 0)?;
                let mut draw = self
                    .rng
                    .for_unit(Purpose::SeedShot, wire_id(marker), shot.left);
                let on = found[draw.below(count) as usize];
                Some((ability, on, ticks_of_ms(delay_ms).max(1)))
            });
            let Some((ability, on, delay)) = next else {
                self.bouncer.remove(marker);
                self.discard(marker);
                continue;
            };
            let cast = Cast {
                caster: shot.source,
                ability,
                level: shot.level,
            };
            self.seed_hit(cast, on);
            shot.last = on;
            shot.left -= 1;
            shot.due += delay;
            if shot.left == 0 {
                self.bouncer.remove(marker);
                self.discard(marker);
            } else {
                self.bouncer.insert(marker, shot);
            }
        }
        self.recycle_entity_snapshot(entities);
    }
}
