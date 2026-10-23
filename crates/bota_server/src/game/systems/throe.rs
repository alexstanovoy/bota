//! What goes off as a neutral dies: the rage it leaves its camp, the shards
//! it falls apart into.

use bota_core::{
    ModifierKind, NeutralAbility, NeutralKind, Spell, Throe, at_level, rules, ticks_of_ms,
};
use bota_proto::Vec2;

use crate::game::{Entity, Modifier, StatBonus, World};

impl World {
    /// Lets every death throe of a neutral that is dying go off, from where
    /// it stands. A broken one has none, and a neutral that merely goes
    /// from the world (a shard whose time ran out) never gets here.
    pub(super) fn death_throes(&mut self, dead: Entity) {
        if self.neutral.get(dead).is_none() || self.broken(dead) {
            return;
        }
        let held = self.abilities.get(dead).map_or(0, |book| book.slots.len());
        for at in 0..held {
            let Some(state) = self.abilities.get(dead).and_then(|book| book.slots.get(at)) else {
                continue;
            };
            let level = state.level;
            let Some(ability) = NeutralAbility::of(state.id) else {
                continue;
            };
            match *ability.spell() {
                Spell::Throe(Throe::Rouse { radius, ms, .. }) => {
                    self.rouse_allies(dead, (ability, level), radius, ms);
                }
                Spell::Throe(Throe::Split { .. }) => self.fall_apart(dead, (ability, level)),
                _ => {}
            }
        }
    }

    /// Every ally within `radius` of a dying unit wears the throe's
    /// modifier for `ms`.
    fn rouse_allies(
        &mut self,
        dead: Entity,
        (ability, level): (NeutralAbility, u8),
        radius: i32,
        ms: i32,
    ) {
        let (Some(side), Some(at)) = (self.team.get(dead).copied(), self.pos(dead)) else {
            return;
        };
        let reach = rules::units(radius);
        let kind = ModifierKind::Spell {
            ability,
            level,
            amount: 0,
        };
        for ally in self.entities.iter().collect::<Vec<_>>() {
            if ally != dead
                && self.team.get(ally).copied() == Some(side)
                && self.living_unit(ally)
                && self.pos(ally).is_some_and(|pos| pos.within(at, reach))
            {
                let held = Modifier::new(kind, Some(dead), Some(ticks_of_ms(ms)));
                self.put_modifier(ally, held);
            }
        }
    }

    /// A golem dying falls apart: its shards stand at the corpse, belong to
    /// its camp and walk back to it, and every enemy about is slowed.
    fn fall_apart(&mut self, dead: Entity, (ability, level): (NeutralAbility, u8)) {
        let Spell::Throe(Throe::Split {
            kind,
            count,
            hp,
            damage,
            life_ms,
            radius,
            slow_pct: _,
            slow_ms,
            ..
        }) = *ability.spell()
        else {
            return;
        };
        let (Some(side), Some(at), Some(home)) = (
            self.team.get(dead).copied(),
            self.pos(dead),
            self.camp_home.get(dead).copied(),
        ) else {
            return;
        };
        let steps = self.upgrades.get(dead).map_or(0, |upgrades| upgrades.0);
        for _ in 0..at_level(&count, level) {
            let shard = self.spawn_neutral(kind, at, home.camp, steps);
            self.shape_shard(
                shard,
                kind,
                home.home,
                level,
                (at_level(&hp, level), at_level(&damage, level)),
            );
            self.give_timed_life(shard, ticks_of_ms(life_ms));
        }
        let reach = rules::units(radius);
        let slow = ModifierKind::Spell {
            ability,
            level,
            amount: 0,
        };
        for foe in self.entities.iter().collect::<Vec<_>>() {
            if self.team.get(foe).is_some_and(|theirs| *theirs != side)
                && self.living_unit(foe)
                && self.pos(foe).is_some_and(|pos| pos.within(at, reach))
            {
                let held = Modifier::new(slow, Some(dead), Some(ticks_of_ms(slow_ms)));
                self.put_modifier(foe, held);
            }
        }
    }

    /// Makes a shard what its parent's spell level calls for: that level's
    /// health and damage over its kind's own, its parent's camp as its home,
    /// and a walk back to it.
    fn shape_shard(
        &mut self,
        shard: Entity,
        kind: NeutralKind,
        home: Vec2,
        level: u8,
        (hp, damage): (i32, i32),
    ) {
        let base = kind.def();
        self.stat_bonus.insert(
            shard,
            StatBonus {
                max_hp: hp - base.max_hp,
                damage: damage - (base.damage + base.damage_spread / 2),
            },
        );
        if let Some(book) = self.abilities.get_mut(shard) {
            for state in book.slots.iter_mut() {
                state.level = level;
            }
        }
        if let Some(camp) = self.camp_home.get_mut(shard) {
            camp.home = home;
        }
        if let Some(ai) = self.neutral_ai.get_mut(shard) {
            ai.going_home = true;
        }
    }
}
