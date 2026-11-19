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
    /// Anything else: a creep, a summon, a neutral.
    Other,
    /// An illusion attack; still counts as an ordinary creep against hit-count health.
    Illusion,
    /// An attack that destroys a susceptible hit-count body in one hit.
    Lethal,
    /// Roshan (not yet told apart: counts as anything else).
    Roshan,
    /// A tower or a fountain (not yet told apart: counts as anything else).
    Structure,
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

/// Health counted in hits: the health each striker's attack takes, whatever it deals; armor,
/// crits, damage and amplification count for nothing, and a spell, a burn or what rides with a
/// swing takes none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitCount {
    /// What a hero's melee swing takes.
    pub melee_hero: Fixed,
    /// What a hero's missile takes.
    pub ranged_hero: Fixed,
    /// What Roshan's attack takes.
    pub roshan: Fixed,
    /// What a tower's or a fountain's attack takes.
    pub structure: Fixed,
    /// What anything else's attack takes, an illusion's too.
    pub other: Fixed,
}

impl HitCount {
    /// Health of `max_hp` that so many attacks of each striker bring down from full, each
    /// taking its share rounded up, so that the counted hits always bring it down and one
    /// fewer never does; Roshan and the structures count as anything else.
    pub const fn hits(melee_hero: u8, ranged_hero: u8, other: u8, max_hp: i32) -> HitCount {
        let other = share(max_hp, other);
        HitCount {
            melee_hero: share(max_hp, melee_hero),
            ranged_hero: share(max_hp, ranged_hero),
            roshan: other,
            structure: other,
            other,
        }
    }

    /// The health a blow takes of a unit with `max_hp`: its striker's share for a swing, the
    /// whole for a lethal one, and none for anything no swing dealt.
    pub const fn taken(self, by: Option<Striker>, max_hp: Fixed) -> Fixed {
        match by {
            Some(Striker::Lethal) => max_hp,
            Some(Striker::MeleeHero) => self.melee_hero,
            Some(Striker::RangedHero) => self.ranged_hero,
            Some(Striker::Roshan) => self.roshan,
            Some(Striker::Structure) => self.structure,
            Some(Striker::Other | Striker::Illusion) => self.other,
            None => Fixed::ZERO,
        }
    }
}

/// `max_hp` over `hits`, rounded up to the next raw step.
const fn share(max_hp: i32, hits: u8) -> Fixed {
    assert!(hits > 0 && max_hp > 0);
    let hits = hits as i32;
    let max = Fixed::from_int(max_hp).raw;
    Fixed {
        raw: (max + hits - 1) / hits,
    }
}
