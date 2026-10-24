//! Health counted in blows, as Dota counts it on a courier or a ward ("hits
//! to kill"): each attack takes a fixed share whatever it deals, by who
//! swung it, and nothing else takes any.

use bota_proto::UnitKind;

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

/// Health counted in hits: the health one attack of each striker takes, out
/// of the unit's maximum. Armor, crits, damage and amplification count for
/// nothing, and a spell, a burn or what rides with a swing takes none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitCount {
    /// What a hero's melee swing takes.
    pub melee_hero: i32,
    /// What a hero's missile takes.
    pub ranged_hero: i32,
    /// What anything else's attack takes.
    pub other: i32,
}

impl HitCount {
    /// The health a blow takes: its striker's share for a swing, none for
    /// anything no swing dealt.
    pub const fn taken(self, by: Option<Striker>) -> i32 {
        match by {
            Some(Striker::MeleeHero) => self.melee_hero,
            Some(Striker::RangedHero) => self.ranged_hero,
            Some(Striker::Other) => self.other,
            None => 0,
        }
    }
}
