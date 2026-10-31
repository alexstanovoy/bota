//! When a unit no player commands casts a spell, and at what: Dota's neutral
//! AI as data.
//!
//! Dota keeps its neutral AI in the game's code, not in its scripts; what it
//! does is Liquipedia's article of each creep ("casts this ability whenever
//! it's aggroed and there are >2 enemy units nearby"). Each such sentence is
//! one [`CastRule`] on the spell's row, keyed by the spell's Dota name, built
//! from a handful of parts:
//! - [`Among`]: what the cast may go at (the caster, the unit it is set on,
//!   the enemies or the allies within a reach);
//! - [`Need`]: what must hold of the caster (hurt, swinging, beset by a
//!   crowd);
//! - [`Mark`]: what must hold of a unit for the cast to go at it (a hero, low
//!   on health, attacking the caster, wearing a buff);
//! - [`Pick`]: which of those it takes.
//!
//! One decider reads them for every unit the AI drives, each think, in the
//! order of its ability slots (the first rule that holds wins), for the
//! spells that are learned, off cooldown and paid for. A spell the AI never
//! casts has no rule. A new unit needs no code as long as its sentences are
//! made of these parts; a new part is one variant and its one reading in the
//! decider.

use crate::NeutralKind;

/// When the AI casts a spell, besides its being learned, off cooldown and
/// paid for, and its caster awake, set on somebody, and neither stunned nor
/// disarmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// Each think, at the unit the rule picks, if it picks one.
    Think(CastRule),
    /// A `chance_pct` chance as a neutral of `kind` of its camp dies, at
    /// whoever killed it.
    MateFalls { kind: NeutralKind, chance_pct: i32 },
}

/// One rule: the units a cast may go at, what must hold of the caster and of
/// the unit, and which unit it takes. With no unit left, nothing is cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CastRule {
    pub among: Among,
    /// Every one must hold of the caster.
    pub needs: &'static [Need],
    /// Every one must hold of the unit the cast goes at.
    pub marks: &'static [Mark],
    pub pick: Pick,
}

/// What a cast may go at. The spell's aim says how it is pointed at the unit
/// taken: at it, at its spot, or around the caster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Among {
    /// The caster itself.
    Caster,
    /// The unit the caster is set on.
    Held,
    /// Every enemy within a reach of the caster that the spell can strike:
    /// no building, ward or courier.
    Foes(Within),
    /// Every neutral creep of the caster's side within a reach of it, itself
    /// included, that can be targeted: none invulnerable.
    Allies(Within),
}

/// How far from the caster a unit's centre may stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Within {
    /// The spell's cast range.
    CastRange,
    /// A radius, in world units.
    Radius(i32),
}

/// What must hold of the caster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// Below `pct` percent of its health.
    HurtBelow { pct: i32 },
    /// In an attack.
    Swinging,
    /// The unit it is set on is in its attack reach.
    Fighting,
    /// At least `at_least` enemies stand within `radius` of it, `heroes` of
    /// them heroes; with `on_it`, counting only those set on it.
    Crowd {
        radius: i32,
        at_least: u8,
        heroes: u8,
        on_it: bool,
    },
}

/// What must hold of the unit a cast goes at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// A hero, an illusion included.
    Hero,
    /// Within the spell's cast range of the caster.
    InCastRange,
    /// Less than `hp` health.
    HpBelow { hp: i32 },
    /// Below `pct` percent of its health.
    HurtBelow { pct: i32 },
    /// Set on the caster.
    AttacksCaster,
    /// Wearing a modifier a purge takes off.
    Buffed,
    /// Wearing no modifier of the spell's own effect.
    Unmarked,
    /// A hero of a side other than its own (and not neutral) stands within
    /// `radius` of the caster.
    RivalNear { radius: i32 },
    /// After a hero of a side other than its own standing within `radius`
    /// of the caster.
    Chasing { radius: i32 },
}

/// Which unit a cast goes at when several qualify; ties go to the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// The first, in the world's order.
    First,
    /// The one nearest the caster.
    Nearest,
    /// The one with the least health.
    LeastHp,
    /// The one with the least share of its health.
    LeastHealthPct,
}

impl CastRule {
    /// A rule for the caster itself, with what must hold of it.
    pub const fn on_itself(needs: &'static [Need]) -> CastRule {
        CastRule {
            among: Among::Caster,
            needs,
            marks: &[],
            pick: Pick::First,
        }
    }

    /// A rule for the unit it is set on, with what must hold of it.
    pub const fn on_held(marks: &'static [Mark]) -> CastRule {
        CastRule {
            among: Among::Held,
            needs: &[],
            marks,
            pick: Pick::First,
        }
    }
}
