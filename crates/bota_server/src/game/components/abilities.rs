//! What an entity can cast.

use bota_core::{
    Talent, TalentTree, ability_def, attribute_bonus_floor, learn_group, level_floor, rules,
};
use bota_proto::{AbilityId, Talents};

use crate::engine::{Fnv, StateHash, state_hash};

/// One ability in a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbilityState {
    pub id: AbilityId,
    /// Level it stands at. Zero until it is learned.
    pub level: u8,
    /// Ticks until it may be cast again.
    pub cooldown: u32,
}

/// The abilities an entity holds, in slot order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AbilityBook {
    /// Every slot, in the order they are shown.
    pub slots: Vec<AbilityState>,
    /// The talents it may learn. Absent for one with none.
    pub tree: Option<&'static TalentTree>,
    /// The talents it has learned, by slot of its tree.
    pub talents: Talents,
    pub attribute_bonus: u8,
}

impl AbilityBook {
    pub fn slot(&self, index: usize) -> Option<&AbilityState> {
        self.slots.get(index)
    }

    pub fn has_talent(&self, talent: Talent) -> bool {
        self.tree
            .is_some_and(|tree| bota_core::has_talent(tree, self.talents, talent))
    }

    /// Skill points spent: on abilities and the Attribute Bonus. Talents
    /// are not among them: since Dota 7.40 they are learned with talent
    /// points of their own, nor is an ability a level grants.
    ///
    /// Slots sharing a learn group were paid for together, so the group
    /// counts once.
    pub fn points_spent(&self) -> u8 {
        self.attribute_bonus
            + self
                .slots
                .iter()
                .enumerate()
                .filter(|(_, held)| ability_def(held.id).is_none_or(|def| def.granted_at.is_none()))
                .filter(|(at, held)| {
                    let group = learn_group(held.id);
                    !self.slots[..*at]
                        .iter()
                        .any(|other| learn_group(other.id) == group)
                })
                .map(|(_, held)| held.level)
                .sum::<u8>()
    }

    /// Whether a point could raise one slot at a hero level: one a point
    /// learns at all, a point unspent, room left in the ability, and the
    /// hero level its next level waits for.
    pub fn can_raise(&self, slot: usize, hero_level: u8) -> bool {
        let Some(ability) = self.slot(slot) else {
            return false;
        };
        let Some(def) = ability_def(ability.id) else {
            return false;
        };
        def.granted_at.is_none()
            && ability.level < def.max_level
            && self.points_spent() < hero_level
            && hero_level >= level_floor(def, ability.level)
    }

    /// Gives every ability its bearer's level opens ([`AbilityDef::granted_at`])
    /// its one level.
    ///
    /// [`AbilityDef::granted_at`]: bota_core::AbilityDef::granted_at
    pub fn grant(&mut self, level: u8) {
        for held in &mut self.slots {
            let opens = ability_def(held.id).and_then(|def| def.granted_at);
            if opens.is_some_and(|from| level >= from) {
                held.level = held.level.max(1);
            }
        }
    }

    /// Whether a point could go into the Attribute Bonus at a hero level:
    /// the book has a talent tree, which is where Dota keeps the bonus, a
    /// point is unspent, the bonus is below its last level, and the hero
    /// level has reached [`attribute_bonus_floor`].
    pub fn can_learn_attribute_bonus(&self, hero_level: u8) -> bool {
        self.tree.is_some()
            && self.attribute_bonus < rules::ATTRIBUTE_BONUS_MAX_LEVEL
            && self.points_spent() < hero_level
            && hero_level >= attribute_bonus_floor(self.attribute_bonus)
    }

    /// Spends a point on the Attribute Bonus when it could take one.
    pub fn learn_attribute_bonus(&mut self, hero_level: u8) -> bool {
        if !self.can_learn_attribute_bonus(hero_level) {
            return false;
        }
        self.attribute_bonus += 1;
        true
    }

    /// Spends the point of a level up on the Attribute Bonus by itself when
    /// no slot could take it, as Dota does: "levels up automatically if all
    /// ability points are spent on the hero's respective abilities"
    /// (Liquipedia, *Attributes*; level 15, 16, 17 and 19 to 22 for a hero
    /// with four abilities).
    pub fn auto_attribute_bonus(&mut self, hero_level: u8) {
        if !(0..self.slots.len()).any(|slot| self.can_raise(slot, hero_level)) {
            self.learn_attribute_bonus(hero_level);
        }
    }
}

state_hash!(AbilityState {
    id,
    level,
    cooldown
});

/// The talent tree is the hero's own and never changes, so only what has been learned goes in.
impl StateHash for AbilityBook {
    fn state_hash(&self, fnv: &mut Fnv) {
        let AbilityBook {
            slots,
            tree: _,
            talents,
            attribute_bonus,
        } = self;
        slots.state_hash(fnv);
        talents.state_hash(fnv);
        attribute_bonus.state_hash(fnv);
    }
}

#[cfg(test)]
impl crate::engine::Specimen for AbilityBook {
    fn low() -> Self {
        AbilityBook {
            slots: crate::engine::Specimen::low(),
            tree: None,
            talents: crate::engine::Specimen::low(),
            attribute_bonus: crate::engine::Specimen::low(),
        }
    }

    fn high() -> Self {
        AbilityBook {
            slots: crate::engine::Specimen::high(),
            tree: None,
            talents: crate::engine::Specimen::high(),
            attribute_bonus: crate::engine::Specimen::high(),
        }
    }
}
