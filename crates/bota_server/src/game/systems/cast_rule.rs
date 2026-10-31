//! The decider of the AI's cast rules: each part of a [`CastRule`] read
//! against the world, in one place, for every unit and spell alike.

use bota_core::{
    Among, CastRule, Mark, ModifierKind, Need, NeutralAbility, Pick, Within, effect_def, rules,
    spell_effect,
};
use bota_proto::{Aim, Target, Team, UnitKind, Vec2};

use crate::game::{ActionState, Entity, UnitOrder, World, wire_id};

/// What a rule is read against: the caster, where it stands, the spell, and
/// the unit it is set on.
#[derive(Clone, Copy)]
struct Seen {
    caster: Entity,
    at: Vec2,
    ability: NeutralAbility,
    held: Entity,
}

impl World {
    /// What a rule casts its spell at now, if anything: the unit it picks,
    /// pointed at as the spell aims. `held` is who the caster is set on.
    pub(super) fn cast_by_rule(
        &self,
        caster: Entity,
        ability: NeutralAbility,
        rule: &CastRule,
        held: Entity,
    ) -> Option<Target> {
        let at = self.pos(caster)?;
        let seen = Seen {
            caster,
            at,
            ability,
            held,
        };
        if !rule.needs.iter().all(|need| self.need_holds(&seen, *need)) {
            return None;
        }
        let fits = |unit: &Entity| {
            rule.marks
                .iter()
                .all(|mark| self.marked(&seen, *unit, *mark))
        };
        let unit = match rule.among {
            Among::Caster => Some(caster).filter(fits),
            Among::Held => Some(held).filter(fits),
            Among::Foes(within) => {
                let foes = self.foes_near(caster, at, self.reach(&seen, within));
                self.pick(&seen, rule.pick, foes.filter(fits))
            }
            Among::Allies(within) => {
                let allies = self.allies_near(caster, at, self.reach(&seen, within));
                self.pick(&seen, rule.pick, allies.filter(fits))
            }
        }?;
        self.aimed(ability.entry().def.aim, unit)
    }

    /// How a spell of an aim is pointed at a unit.
    fn aimed(&self, aim: Aim, at: Entity) -> Option<Target> {
        match aim {
            Aim::Unit => Some(Target::Unit(wire_id(at))),
            Aim::Point => self.pos(at).map(Target::Pos),
            _ => Some(Target::None),
        }
    }

    /// A reach in world units.
    fn reach(&self, seen: &Seen, within: Within) -> bota_proto::Fixed {
        rules::units(match within {
            Within::CastRange => seen.ability.entry().def.range,
            Within::Radius(radius) => radius,
        })
    }

    /// Whether a need holds of the caster.
    fn need_holds(&self, seen: &Seen, need: Need) -> bool {
        let caster = seen.caster;
        match need {
            Need::HurtBelow { pct } => self.below_pct(caster, pct),
            Need::Swinging => matches!(
                self.action.get(caster).map(|action| action.state),
                Some(ActionState::Attack { .. })
            ),
            Need::Fighting => self.in_reach(caster, seen.held),
            Need::Crowd {
                radius,
                at_least,
                heroes,
                on_it,
            } => {
                let near = self.foes_near(caster, seen.at, rules::units(radius));
                let (crowd, among) = near
                    .filter(|foe| !on_it || self.target_of(*foe) == Some(caster))
                    .fold((0, 0), |(crowd, among), foe| {
                        (crowd + 1, among + usize::from(self.is_hero(foe)))
                    });
                crowd >= usize::from(at_least) && among >= usize::from(heroes)
            }
        }
    }

    /// Whether a mark holds of a unit a cast may go at.
    fn marked(&self, seen: &Seen, unit: Entity, mark: Mark) -> bool {
        match mark {
            Mark::Hero => self.is_hero(unit),
            Mark::InCastRange => {
                let range = rules::units(seen.ability.entry().def.range);
                self.pos(unit).is_some_and(|pos| pos.within(seen.at, range))
            }
            Mark::HpBelow { hp } => self
                .health
                .get(unit)
                .is_some_and(|health| health.hp.to_int() < hp),
            Mark::HurtBelow { pct } => self.below_pct(unit, pct),
            Mark::AttacksCaster => self.target_of(unit) == Some(seen.caster),
            Mark::Buffed => self.modifiers.get(unit).is_some_and(|on_it| {
                on_it.active().any(|held| {
                    effect_def(held.kind.effect_id()).is_some_and(|def| def.dispellable)
                })
            }),
            Mark::Unmarked => !self.wears_spell_effect(unit, seen.ability),
            Mark::RivalNear { radius } => self.rival_near(seen, unit, radius),
            Mark::Chasing { radius } => self.chasing(seen, unit, radius),
        }
    }

    /// The unit a pick takes of the ones that qualify; ties go to the first.
    fn pick(
        &self,
        seen: &Seen,
        pick: Pick,
        mut units: impl Iterator<Item = Entity>,
    ) -> Option<Entity> {
        match pick {
            Pick::First => units.next(),
            Pick::Nearest => units.min_by_key(|unit| {
                self.pos(*unit)
                    .map_or(i64::MAX, |pos| pos.distance_squared(seen.at))
            }),
            Pick::LeastHp => units
                .filter_map(|unit| Some((self.health.get(unit)?.hp.to_int(), unit)))
                .min_by_key(|(hp, _)| *hp)
                .map(|(_, unit)| unit),
            Pick::LeastHealthPct => units
                .filter_map(|unit| Some((self.health_pct(unit)?, unit)))
                .min_by_key(|(share, _)| *share)
                .map(|(_, unit)| unit),
        }
    }

    /// A unit's health as a whole percent of its maximum, rounded down.
    fn health_pct(&self, unit: Entity) -> Option<i64> {
        let hp = i64::from(self.health.get(unit)?.hp.to_int());
        let most = i64::from(self.stats.get(unit)?.max_hp.to_int()).max(1);
        Some(hp * 100 / most)
    }

    /// Whether a unit is below a share of its health.
    fn below_pct(&self, unit: Entity, pct: i32) -> bool {
        self.health_pct(unit)
            .is_some_and(|share| share < i64::from(pct))
    }

    /// Whether a unit wears a modifier of a spell's own effect.
    fn wears_spell_effect(&self, unit: Entity, ability: NeutralAbility) -> bool {
        let Some(own) = spell_effect(ability) else {
            return false;
        };
        self.modifiers.get(unit).is_some_and(|on_it| {
            on_it.active().any(|held| match held.kind {
                ModifierKind::Spell { ability, .. } => spell_effect(ability) == Some(own),
                _ => false,
            })
        })
    }

    /// Whether a hero of a side other than the unit's, and not neutral,
    /// stands within `radius` of the caster.
    fn rival_near(&self, seen: &Seen, unit: Entity, radius: i32) -> bool {
        let Some(side) = self.team.get(unit).copied() else {
            return false;
        };
        self.entities.iter().any(|other| {
            other != seen.caster
                && self.is_hero(other)
                && self.alive(other)
                && self.other_side(other, side)
                && self
                    .pos(other)
                    .is_some_and(|pos| pos.within(seen.at, rules::units(radius)))
        })
    }

    /// Whether a unit is after a hero of a side other than its own standing
    /// within `radius` of the caster: the one it is ordered to attack or
    /// follow, else the one it is set on.
    fn chasing(&self, seen: &Seen, unit: Entity, radius: i32) -> bool {
        let Some(side) = self.team.get(unit).copied() else {
            return false;
        };
        let chased = match self.orders.get(unit).map(|orders| orders.current) {
            Some(UnitOrder::Attack { target, .. } | UnitOrder::Follow { target, .. }) => {
                Some(target)
            }
            _ => self.target_of(unit),
        };
        chased.is_some_and(|chased| {
            chased != seen.caster
                && self.is_hero(chased)
                && self.alive(chased)
                && self.other_side(chased, side)
                && self
                    .pos(chased)
                    .is_some_and(|pos| pos.within(seen.at, rules::units(radius)))
        })
    }

    /// Whether a unit is of a side, neither `side` nor neutral.
    fn other_side(&self, unit: Entity, side: Team) -> bool {
        self.team
            .get(unit)
            .is_some_and(|team| *team != side && *team != Team::Neutral)
    }

    /// Every foe whose centre lies within a radius of a spot, in slot order.
    pub(super) fn foes_near(
        &self,
        caster: Entity,
        at: Vec2,
        radius: bota_proto::Fixed,
    ) -> impl Iterator<Item = Entity> + '_ {
        self.entities
            .iter()
            .filter(move |other| self.is_foe(caster, *other))
            .filter(move |other| self.pos(*other).is_some_and(|pos| pos.within(at, radius)))
    }

    /// Every neutral creep of the caster's side that can be targeted, alive,
    /// within a radius of a spot, in slot order; the caster included.
    fn allies_near(
        &self,
        caster: Entity,
        at: Vec2,
        radius: bota_proto::Fixed,
    ) -> impl Iterator<Item = Entity> + '_ {
        let side = self.team.get(caster).copied();
        self.entities.iter().filter(move |ally| {
            self.alive(*ally)
                && side.is_some_and(|side| self.team.get(*ally) == Some(&side))
                && self.kind.get(*ally) == Some(&UnitKind::CreepNeutral)
                && !self
                    .stats
                    .get(*ally)
                    .is_some_and(|stats| stats.invulnerable)
                && self.pos(*ally).is_some_and(|pos| pos.within(at, radius))
        })
    }
}
