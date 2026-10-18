//! The neutral spells that call something in: units raised, a tornado stood
//! up, a target carried off.

use bota_core::{
    Act, ModifierKind, at_level, clamp_to_map, jungle_upgrades, point_ahead, point_along, rules,
    ticks_of_ms,
};
use bota_proto::{Angle, Target, Vec2};

use crate::game::{CampHome, Expiry, Modifier, Push, StatBonus, World};

use super::spell::Cast;

impl World {
    /// Raises units of a kind round the caster, each with the health and the
    /// damage of the spell's level, who live a while. They belong to the
    /// caster's camp and come awake onto whoever the caster is set on.
    pub(super) fn summon(&mut self, cast: Cast, act: Act) -> bool {
        let Act::Summon {
            kind,
            count,
            hp,
            damage,
            life_ms,
        } = act
        else {
            return false;
        };
        let (Some(from), Some(home)) = (
            self.pos(cast.caster),
            self.camp_home.get(cast.caster).copied(),
        ) else {
            return false;
        };
        let foe = self.target_of(cast.caster);
        let first = self
            .transform
            .get(cast.caster)
            .map_or(0, |transform| i32::from(transform.facing.brads));
        for raised in 0..i32::from(count) {
            let facing = Angle {
                brads: (first + raised * 65536 / i32::from(count)) as u16,
            };
            let at = point_ahead(from, facing, rules::units(SUMMON_RING));
            let unit = self.spawn_neutral(kind, at, home.camp, jungle_upgrades(self.tick));
            self.camp_home.insert(
                unit,
                CampHome {
                    camp: home.camp,
                    home: home.home,
                },
            );
            if let Some(ai) = self.neutral_ai.get_mut(unit) {
                ai.awake = true;
                ai.roused_by = foe;
            }
            self.stat_bonus.insert(
                unit,
                StatBonus {
                    max_hp: at_level(&hp, cast.level) - kind.def().max_hp,
                    damage: at_level(&damage, cast.level) - kind.def().damage,
                },
            );
            self.expiry.insert(
                unit,
                Expiry {
                    ticks_left: ticks_of_ms(life_ms),
                },
            );
        }
        self.settle();
        true
    }

    /// Stands a tornado at a spot for a while. It cannot be hurt, and stays
    /// where it was put.
    pub(super) fn tornado(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let Act::Tornado { kind, life_ms } = act else {
            return false;
        };
        let (Target::Pos(at), Some(home)) = (target, self.camp_home.get(cast.caster).copied())
        else {
            return false;
        };
        let ticks = ticks_of_ms(life_ms);
        let unit = self.spawn_neutral(kind, clamp_to_map(at), home.camp, 0);
        self.expiry.insert(unit, Expiry { ticks_left: ticks });
        self.put_modifier(
            unit,
            Modifier::new(ModifierKind::Shielded, None, Some(ticks)),
        );
        self.settle();
        true
    }

    /// Starts carrying the target away from the caster, along the line from
    /// the caster through it.
    ///
    /// Dota's push is forced movement that crosses cliffs and trees; the
    /// target here is held still while it is carried, as a hook's catch is,
    /// and ends on the last open ground on the way.
    pub(super) fn push_away(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let Act::Push { distance, speed } = act else {
            return false;
        };
        let Some(on) = self.unit_aimed_at(target) else {
            return false;
        };
        let (Some(from), Some(at)) = (self.pos(cast.caster), self.pos(on)) else {
            return false;
        };
        let away: Vec2 = if from == at {
            at + Vec2::from_ints(1, 0)
        } else {
            at + (at - from)
        };
        let aim = clamp_to_map(point_along(at, away, rules::units(distance)));
        self.push.insert(
            on,
            Push {
                source: cast.caster,
                aim,
                speed: bota_proto::Fixed::from_int(speed),
                clear: at,
            },
        );
        true
    }
}

/// How far from its summoner a summoned unit stands.
const SUMMON_RING: i32 = 100;
