//! What an ability is, beyond what the wire says about it.
//!
//! A snapshot carries an ability's level, its wait and what a cast would cost,
//! but not whether it can be cast at all or what it must be aimed at. Those
//! are the server's rules, and a bot that guesses at them spends its ticks on
//! orders that come back refused — a thousand of them in the first match this
//! was left out of, which is a thousand creeps not swung at.
//!
//! So the bot keeps its own list, by the number the wire names an ability
//! with. An ability that is not on it is treated as one that cannot be cast:
//! saying nothing is better than being wrong about it.

use bota_proto::AbilityId;

use crate::Aim;

/// How an ability must be aimed for the server to take the order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aiming {
    /// At nobody: it works on whoever cast it.
    Own,
    /// At a spot on the ground.
    Spot,
    /// At a unit, and one the caster fights.
    Foe,
}

/// What one ability is, or nothing for one the bot has never heard of.
///
/// Answers nothing for anything that works on its own and is never cast.
pub fn aiming_of(id: AbilityId) -> Option<Aiming> {
    Some(match id.0 {
        // Sylla: a critical strike that simply happens, a frenzy on itself, a
        // bolt at somebody, a volley about itself.
        0 => return None,
        1 => Aiming::Own,
        2 => Aiming::Foe,
        3 => Aiming::Own,
        // Pudge: a hook thrown at the ground, a rot toggled on itself, a heap
        // that simply grows, a dismember of somebody.
        4 => Aiming::Spot,
        5 => Aiming::Own,
        6 => return None,
        7 => Aiming::Foe,
        // A courier's errands all work on the courier.
        8..=11 => Aiming::Own,
        // Shadow Fiend: three razes laid along a line to a spot, and a requiem
        // about himself.
        13..=15 => Aiming::Spot,
        16 => Aiming::Own,
        _ => return None,
    })
}

/// Whether a way of aiming suits an ability.
///
/// The one place the two vocabularies meet: [`Aim`] is what a deed says, and
/// [`Aiming`] is what the server will take.
pub fn suits(id: AbilityId, aim: Aim) -> bool {
    matches!(
        (aiming_of(id), aim),
        (Some(Aiming::Own), Aim::Own)
            | (Some(Aiming::Spot), Aim::Ahead)
            | (Some(Aiming::Foe), Aim::Hero | Aim::Creep)
    )
}
