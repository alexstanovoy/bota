//! Health counted in blows, as Dota counts it on a courier or a ward ("hits
//! to kill"): each attack takes a fixed share whatever it deals, by who
//! swung it, and nothing else takes any.

use std::num::NonZeroU8;

use bota_proto::{Fixed, UnitKind};

/// Who swung a blow, as health counted in hits tells blows apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Striker {
    /// A hero swinging in melee.
    MeleeHero,
    /// A hero throwing a missile.
    RangedHero,
    /// Anything else: a creep, a summon, a neutral.
    Other,
    /// An illusion attack; still counts as an ordinary creep against hit-count health.
    Illusion,
    /// Roshan. The former lethal-strike hash tag is retired.
    Roshan = 5,
    /// A tower or a fountain.
    Structure,
}

impl Striker {
    /// The striker a unit of this kind is, swinging in melee or throwing.
    pub const fn of(kind: UnitKind, ranged: bool) -> Striker {
        match (kind, ranged) {
            (UnitKind::Hero, false) => Striker::MeleeHero,
            (UnitKind::Hero, true) => Striker::RangedHero,
            (UnitKind::Roshan, _) => Striker::Roshan,
            (UnitKind::Tower | UnitKind::Fountain, _) => Striker::Structure,
            _ => Striker::Other,
        }
    }
}

/// Health that attacks take in fixed pips or in a fixed number of hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitCount {
    /// Dota 7.42's health bar pips, counted in quarter pips.
    Pips { fragile: bool },
    /// Attacks needed from full health, using the unit's current maximum.
    Hits {
        melee_hero: NonZeroU8,
        ranged_hero: NonZeroU8,
        other: NonZeroU8,
    },
}

impl HitCount {
    pub const fn pips(fragile: bool) -> HitCount {
        HitCount::Pips { fragile }
    }

    pub const fn hits(melee_hero: u8, ranged_hero: u8, other: u8) -> HitCount {
        HitCount::Hits {
            melee_hero: NonZeroU8::new(melee_hero).expect("a positive melee hit count"),
            ranged_hero: NonZeroU8::new(ranged_hero).expect("a positive ranged hit count"),
            other: NonZeroU8::new(other).expect("a positive hit count"),
        }
    }

    /// Health one landed attack removes before capping it at the target's pool.
    pub const fn taken(self, by: Striker, max_hp: Fixed) -> Fixed {
        match self {
            HitCount::Pips { fragile } => Fixed::from_int(match by {
                Striker::MeleeHero | Striker::Roshan if fragile => 2 * QUARTERS_A_PIP,
                Striker::MeleeHero | Striker::RangedHero | Striker::Roshan | Striker::Structure => {
                    QUARTERS_A_PIP
                }
                Striker::Other | Striker::Illusion => 1,
            }),
            HitCount::Hits {
                melee_hero,
                ranged_hero,
                other,
            } => {
                let hits = match by {
                    Striker::MeleeHero => melee_hero,
                    Striker::RangedHero => ranged_hero,
                    _ => other,
                };
                if max_hp.raw <= 0 {
                    return Fixed::ZERO;
                }
                // Round up to the next raw step so the last hit always suffices.
                Fixed {
                    raw: (max_hp.raw - 1) / hits.get() as i32 + 1,
                }
            }
        }
    }
}

/// The quarters a pip of health holds.
pub const QUARTERS_A_PIP: i32 = 4;
