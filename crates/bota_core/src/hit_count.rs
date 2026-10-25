//! Health counted in blows, as Dota counts it on a courier or a ward ("hits
//! to kill"): each attack takes a fixed share whatever it deals, by who
//! swung it, and nothing else takes any.

use bota_proto::{Fixed, UnitKind};

/// Who swung a blow, as health counted in hits tells blows apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Striker {
    /// A hero swinging in melee.
    MeleeHero,
    /// A hero throwing a missile.
    RangedHero,
    /// Anything else: a creep, a building.
    Other,
}

impl Striker {
    /// The striker a unit of this kind is, swinging in melee or throwing.
    pub const fn of(kind: UnitKind, ranged: bool) -> Striker {
        match (kind, ranged) {
            (UnitKind::Hero, false) => Striker::MeleeHero,
            (UnitKind::Hero, true) => Striker::RangedHero,
            _ => Striker::Other,
        }
    }
}

/// Health counted in hits, as Dota's "hits to kill": how many attacks of each
/// striker bring the unit down from full. Each takes that share of its
/// maximum whatever it deals; armor, crits, damage and amplification count
/// for nothing, and a spell, a burn or what rides with a swing takes none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitCount {
    /// Hero melee swings that bring it down.
    pub melee_hero: u8,
    /// Hero missiles that bring it down.
    pub ranged_hero: u8,
    /// Attacks of anything else (a creep, a building, an illusion) that
    /// bring it down.
    pub other: u8,
}

impl HitCount {
    /// The health a blow takes of a unit with `max_hp`: its striker's share
    /// for a swing, rounded up so that the counted hits always bring it down
    /// and one fewer never does, and none for anything no swing dealt.
    pub const fn taken(self, by: Option<Striker>, max_hp: Fixed) -> Fixed {
        let hits = match by {
            Some(Striker::MeleeHero) => self.melee_hero,
            Some(Striker::RangedHero) => self.ranged_hero,
            Some(Striker::Other) => self.other,
            None => return Fixed::ZERO,
        };
        assert!(hits > 0 && max_hp.raw > 0);
        let hits = hits as i32;
        Fixed {
            raw: (max_hp.raw + hits - 1) / hits,
        }
    }
}
