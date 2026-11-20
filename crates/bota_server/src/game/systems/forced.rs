//! Grips and airborne movement share the ordinary push owner and the normal damage queue.
use super::spell::Cast;
use crate::game::{
    ActionState, Entity, Hit, HitEffect, Landing, Leaping, Push, PushPhase, World, wire_id,
};
use bota_core::{Act, Grab, NeutralAbility, Spell, point_ahead, rules, ticks_of_ms};
use bota_proto::{DamageKind, Fixed, Inflictor, Target, Team, Vec2};

impl World {
    pub(super) fn start_grab(&mut self, cast: Cast, spec: &Grab, target: Target) -> bool {
        let Some(on) = self
            .foe_aimed_at(cast, target)
            .filter(|u| self.is_hero(*u) && !self.wears_illusion(*u))
        else {
            return false;
        };
        if self.push.contains(on)
            || self.held_by_hook(on)
            || self.stats.get(on).is_some_and(|s| s.forced_move_immune)
        {
            return false;
        }
        let Some(at) = self.pos(on) else {
            return false;
        };
        let combo = self.journey(cast.caster).map_or(0, |(_, _, throws)| throws);
        self.push.insert(
            on,
            Push {
                source: cast.caster,
                inflictor: Inflictor::Ability(cast.ability.id()),
                team: self.team.get(cast.caster).copied().unwrap_or(Team::Neutral),
                aim: at,
                speed: Fixed::ZERO,
                clear: at,
                effect: Some(spec.grabbed_effect),
                phase: PushPhase::Grip {
                    started: self.tick,
                    level: cast.level,
                    contacted: false,
                    combo,
                },
            },
        );
        self.forget_walk(on);
        true
    }
    pub(super) fn spell_channel_holds(
        &self,
        caster: Entity,
        ability: NeutralAbility,
        target: Target,
    ) -> bool {
        let Some(on) = self.unit_aimed_at(target) else {
            return false;
        };
        self.push.get(on).is_some_and(|push| {
            push.source == caster
                && push.inflictor == Inflictor::Ability(ability.id())
                && push.carries()
        })
    }
    pub(super) fn cancel_spell_channel(
        &mut self,
        caster: Entity,
        ability: NeutralAbility,
        target: Target,
    ) {
        let Some(on) = self.unit_aimed_at(target) else {
            return;
        };
        let ours = self.push.get(on).is_some_and(|p| {
            p.source == caster
                && p.inflictor == Inflictor::Ability(ability.id())
                && matches!(p.phase, PushPhase::Grip { .. })
        });
        if ours {
            self.push.remove(on);
            self.forget_walk(on);
        }
    }
    pub(super) fn tick_grip(&mut self, on: Entity, mut push: Push) {
        let PushPhase::Grip {
            started,
            level,
            mut contacted,
            combo,
        } = push.phase
        else {
            return;
        };
        let Inflictor::Ability(id) = push.inflictor else {
            self.push.remove(on);
            return;
        };
        let Some(ability) = NeutralAbility::of(id) else {
            self.push.remove(on);
            return;
        };
        let Spell::Cast {
            act: Act::Grab(spec),
            ..
        } = *ability.spell()
        else {
            self.push.remove(on);
            return;
        };
        let owns=self.action.get(push.source).is_some_and(|a|matches!(a.state,ActionState::CastAbility{slot,target:Target::Unit(victim),phase:crate::game::ActionPhase::During{..}} if victim==wire_id(on)&&self.ability_in(push.source,slot)==id));
        let def = &ability.entry().def;
        let interrupted = (self.held(push.source) || self.feared(push.source))
            && !def.cast_while_stunned
            || self.silenced(push.source) && !def.cast_while_silenced;
        if !self.alive(push.source) || !owns || interrupted {
            self.push.remove(on);
            self.forget_walk(on);
            return;
        }
        let age = self.tick.saturating_sub(started);
        if !contacted && age >= spec.contact_after {
            self.journey_contact(push.source, on, spec.contact_ability, false);
            contacted = true;
        }
        if age >= spec.duration {
            self.launch_grab(on, push, ability, level, combo, spec);
        } else {
            push.phase = PushPhase::Grip {
                started,
                level,
                contacted,
                combo,
            };
            self.push.insert(on, push);
        }
    }
    fn launch_grab(
        &mut self,
        on: Entity,
        mut push: Push,
        _ability: NeutralAbility,
        level: u8,
        combo: u8,
        spec: &Grab,
    ) {
        let (Some(at), Some(source)) = (self.pos(on), self.transform.get(push.source)) else {
            self.push.remove(on);
            return;
        };
        push.aim =
            bota_core::clamp_to_map(point_ahead(at, source.facing, rules::units(spec.distance)));
        push.effect = Some(spec.thrown_effect);
        let damage = (i64::from(spec.damage) + i64::from(spec.damage_per_throw) * i64::from(combo))
            .clamp(0, i32::MAX.into()) as i32;
        let landing = Landing {
            damage,
            health_growth: Some(spec.health_growth),
            level,
            kind: DamageKind::Pure,
            stun: spec.stun,
            damage_amp_bp: self.outgoing_spell_amp_bp(Some(push.source), DamageKind::Pure),
            leap: None,
        };
        push.phase = PushPhase::Arc {
            from: at,
            started: self.tick,
            duration: spec.flight,
            height: spec.height,
            tree_radius: 0,
            landing: Some(landing),
        };
        self.note_journey_throw(push.source);
        self.push.insert(on, push);
    }
    pub(super) fn tick_arc(&mut self, on: Entity, mut push: Push) {
        let PushPhase::Arc {
            from,
            started,
            duration,
            tree_radius,
            landing,
            ..
        } = push.phase
        else {
            return;
        };
        assert!(duration > 0);
        let age = self.tick.saturating_sub(started).min(duration);
        let coordinate = |a: Fixed, b: Fixed| Fixed {
            raw: (i64::from(a.raw)
                + (i64::from(b.raw) - i64::from(a.raw)) * i64::from(age) / i64::from(duration))
                as i32,
        };
        let next = Vec2 {
            x: coordinate(from.x, push.aim.x),
            y: coordinate(from.y, push.aim.y),
        };
        let before = self.pos(on).unwrap_or(from);
        if tree_radius > 0 {
            self.fell_trees_along(before, next, tree_radius, push.source);
        }
        if self.landing_clear(on, next) {
            push.clear = next;
        }
        if age == duration {
            self.push.remove(on);
            let to = self.resolve_landing_for(on, push.aim).at;
            if let Some(at) = self.transform.get_mut(on) {
                at.pos = to;
            }
            self.forget_walk(on);
            if let Some(Landing {
                leap: Some(leap), ..
            }) = landing
            {
                self.land_leap(on, leap);
            } else if let Some(landing) = landing {
                if let Err(at) = self.landing_hold.binary_search(&on) {
                    self.landing_hold.insert(at, on);
                }
                debug_assert!(self.landing_hold.is_sorted());
                self.land_forced(on, push, landing);
            }
        } else {
            if let Some(at) = self.transform.get_mut(on) {
                at.pos = next;
            }
            self.push.insert(on, push);
            self.forget_walk(on);
        }
    }
    /// Sends a caster leaping forward along its facing, as far as its spell reaches in the time the
    /// speed and acceleration give; it lands with the spell's slam and then leaps `leaps_left`
    /// times more.
    pub(super) fn start_leap(
        &mut self,
        caster: Entity,
        ability: NeutralAbility,
        level: u8,
        leaps_left: u8,
    ) -> bool {
        let Spell::Cast {
            act:
                Act::Leap {
                    distance,
                    speed,
                    acceleration,
                    ..
                },
            ..
        } = *ability.spell()
        else {
            return false;
        };
        let (Some(from), Some(team)) = (
            self.transform.get(caster).copied(),
            self.team.get(caster).copied(),
        ) else {
            return false;
        };
        if self.push.contains(caster) {
            return false;
        }
        let aim =
            bota_core::clamp_to_map(point_ahead(from.pos, from.facing, rules::units(distance)));
        let landing = Landing {
            damage: 0,
            health_growth: None,
            level,
            kind: DamageKind::Magical,
            stun: 0,
            damage_amp_bp: rules::NOMINAL_BP,
            leap: Some(Leaping {
                ability: ability.id(),
                level,
                leaps_left,
            }),
        };
        self.push.insert(
            caster,
            Push {
                source: caster,
                inflictor: bota_proto::Inflictor::Ability(ability.id()),
                team,
                aim,
                speed: Fixed::ZERO,
                clear: from.pos,
                effect: None,
                phase: PushPhase::Arc {
                    from: from.pos,
                    started: self.tick,
                    duration: ticks_of_ms(leap_ms(distance, speed, acceleration)).max(1),
                    height: 0,
                    tree_radius: 0,
                    landing: Some(landing),
                },
            },
        );
        self.forget_walk(caster);
        true
    }

    /// Lands a leaping caster: every foe within the spell's radius takes its damage and slow, and
    /// the caster leaps on while it has leaps left.
    fn land_leap(&mut self, caster: Entity, leap: Leaping) {
        let Some(ability) = NeutralAbility::of(leap.ability) else {
            return;
        };
        let Spell::Cast {
            act:
                Act::Leap {
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
        let Some(at) = self.pos(caster) else {
            return;
        };
        let cast = super::spell::Cast {
            caster,
            ability,
            level: leap.level,
        };
        for foe in self.foes_within(caster, at, rules::units(radius)) {
            if !self.spell_reaches(ability.id(), foe) {
                continue;
            }
            self.wear(cast, foe, slow_ms, 0);
            self.push_hit(
                Some(caster),
                foe,
                damage,
                DamageKind::Magical,
                cast.inflictor(),
            );
        }
        if leap.leaps_left > 0 {
            self.start_leap(caster, ability, leap.level, leap.leaps_left - 1);
        }
    }

    fn land_forced(&mut self, on: Entity, push: Push, landing: Landing) {
        if let Some(at) = self.pos(on) {
            self.events.push(crate::game::Event::Fact {
                kind: bota_proto::EventKind::Landed {
                    missile: None,
                    inflictor: push.inflictor,
                    caster: Some(wire_id(push.source)),
                    at,
                },
                visible_to: self.who_sees_spot(at, push.team),
                gold_hidden_from: None,
                source_told_to: self.who_may_name(Some(push.source)),
            });
        }
        let hp = self.health.get(on).map_or(0, |h| h.hp.to_int());
        let pct = landing.health_growth.map_or(0, |g| {
            g.amount(0, landing.level, self.clock(), self.schedule())
        });
        let damage = (i64::from(landing.damage)
            + i64::from(hp) * i64::from(pct) / i64::from(rules::NOMINAL_BP))
        .clamp(0, i32::MAX.into()) as i32;
        self.hits.push_back(Hit {
            source: Some(push.source),
            target: on,
            inflictor: Some(push.inflictor),
            amount: damage,
            kind: landing.kind,
            damage_amp_bp: landing.damage_amp_bp,
            attack: false,
            spell: true,
            striker: None,
            pierces: false,
            uphill: false,
            effect: HitEffect::Stun {
                ticks: landing.stun,
            },
            blockable_source_team: None,
            proc: None,
            laid: crate::game::Laid::NONE,
            removal: false,
            spares: false,
            riders: crate::game::Riders::NONE,
            category: crate::game::Category::Plain,
        });
    }
}

/// How long a leap of `distance` takes, in milliseconds, starting at `speed` and speeding up by
/// `acceleration` each second: the time `t` with `distance = speed * t + acceleration * t^2 / 2`.
fn leap_ms(distance: i32, speed: i32, acceleration: i32) -> i32 {
    assert!(distance > 0 && speed > 0 && acceleration > 0);
    let (distance, speed, acceleration) = (
        i64::from(distance),
        i64::from(speed),
        i64::from(acceleration),
    );
    // sqrt(speed^2 + 2 * acceleration * distance), in thousandths.
    let root = (speed * speed + 2 * acceleration * distance)
        .saturating_mul(1_000_000)
        .isqrt();
    ((root - speed * 1000) / acceleration) as i32
}
