//! What an entity can cast.

use bota_core::{
    Talent, TalentTree, ability_def, attribute_bonus_floor, learn_group, level_floor, rules,
};
use bota_proto::{AbilityId, Talents};

/// One ability in a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbilityState {
    /// Which ability sits here.
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
    /// Levels of the Attribute Bonus learned.
    pub attribute_bonus: u8,
}

impl AbilityBook {
    /// What sits in a slot, if anything does.
    pub fn slot(&self, index: usize) -> Option<&AbilityState> {
        self.slots.get(index)
    }

    /// Whether it has learned a talent.
    pub fn has_talent(&self, talent: Talent) -> bool {
        self.tree
            .is_some_and(|tree| bota_core::has_talent(tree, self.talents, talent))
    }

    /// Every ability that has been learned.
    pub fn learned(&self) -> impl Iterator<Item = &AbilityState> {
        self.slots.iter().filter(|a| a.level > 0)
    }

    /// Skill points spent: on abilities and the Attribute Bonus. Talents
    /// are not among them: since Dota 7.40 they are learned with talent
    /// points of their own.
    ///
    /// Slots sharing a learn group were paid for together, so the group
    /// counts once.
    pub fn points_spent(&self) -> u8 {
        self.attribute_bonus
            + self
                .slots
                .iter()
                .enumerate()
                .filter(|(at, held)| {
                    let group = learn_group(held.id);
                    !self.slots[..*at]
                        .iter()
                        .any(|other| learn_group(other.id) == group)
                })
                .map(|(_, held)| held.level)
                .sum::<u8>()
    }

    /// Whether a point could raise one slot at a hero level: a point
    /// unspent, room left in the ability, and the hero level its next level
    /// waits for.
    pub fn can_raise(&self, slot: usize, hero_level: u8) -> bool {
        let Some(ability) = self.slot(slot) else {
            return false;
        };
        let Some(def) = ability_def(ability.id) else {
            return false;
        };
        ability.level < def.max_level
            && self.points_spent() < hero_level
            && hero_level >= level_floor(def, ability.level)
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
