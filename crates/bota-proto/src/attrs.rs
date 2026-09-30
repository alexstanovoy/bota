//! The three attributes a hero is made of.

use core::ops::{Add, AddAssign};

use serde::{Deserialize, Serialize};

use crate::Fixed;

/// One of the three attributes.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Attribute {
    /// Strength: health and health regeneration.
    Strength,
    /// Agility: armor and attack speed.
    Agility,
    /// Intelligence: mana and mana regeneration.
    Intelligence,
}

/// Every one of them at once, in whole points and fractions of one.
#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct Attributes {
    pub strength: Fixed,
    pub agility: Fixed,
    pub intelligence: Fixed,
}

impl Attributes {
    /// None of any of them.
    pub const ZERO: Attributes = Attributes {
        strength: Fixed::ZERO,
        agility: Fixed::ZERO,
        intelligence: Fixed::ZERO,
    };

    /// The same whole number of every one of them.
    pub const fn all(points: i32) -> Attributes {
        Attributes {
            strength: Fixed::from_int(points),
            agility: Fixed::from_int(points),
            intelligence: Fixed::from_int(points),
        }
    }

    /// One of them, chosen at runtime.
    pub const fn of(self, which: Attribute) -> Fixed {
        match which {
            Attribute::Strength => self.strength,
            Attribute::Agility => self.agility,
            Attribute::Intelligence => self.intelligence,
        }
    }

    /// Every one of them taken `times` times over.
    pub fn scaled(self, times: Fixed) -> Attributes {
        Attributes {
            strength: self.strength * times,
            agility: self.agility * times,
            intelligence: self.intelligence * times,
        }
    }
}

impl Add for Attributes {
    type Output = Attributes;
    fn add(self, rhs: Attributes) -> Attributes {
        Attributes {
            strength: self.strength + rhs.strength,
            agility: self.agility + rhs.agility,
            intelligence: self.intelligence + rhs.intelligence,
        }
    }
}

impl AddAssign for Attributes {
    fn add_assign(&mut self, rhs: Attributes) {
        *self = *self + rhs;
    }
}
