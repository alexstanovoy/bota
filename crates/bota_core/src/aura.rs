//! An aura: what a unit hands out to those standing near it, as Dota has
//! it. A unit carries it of its own (a fountain's, a tower's, a
//! flagbearer's: [`UnitDef::auras`](crate::UnitDef::auras)) or with an
//! ability whose passive it is (Shadow Fiend's presence, a neutral's aura:
//! [`AbilityDef::aura`](crate::AbilityDef::aura)).

use crate::{ModifierKind, NeutralAbility, Talent, presence_armor};

/// Whom an aura reaches: its carrier's side or every other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whom {
    /// Its carrier's own side.
    Allies,
    /// Every side but its carrier's, the neutral one among them.
    Enemies,
}

/// Which units of the side it is for an aura reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Every unit, buildings and wards and the invulnerable among them.
    All,
    /// Heroes, the invulnerable among them.
    Heroes,
    /// Every unit but heroes, buildings and wards, the invulnerable among
    /// them.
    Units,
    /// Every unit standing alive that damage reaches: no building, no ward,
    /// nothing invulnerable.
    Living,
}

/// What an aura puts on whoever it reaches, as its carrier holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuraModifier {
    /// This modifier, grown by the upgrade intervals its carrier spawned
    /// after ([`ModifierKind::grown`]).
    Kind(ModifierKind),
    /// The armor the presence tears at the level its carrier holds it, the
    /// talent tearing more: [`presence_armor`].
    ArmorBroken,
    /// A neutral spell's own modifier, at the level its carrier holds the
    /// spell.
    Spell(NeutralAbility),
}

impl AuraModifier {
    /// What it puts on from a carrier holding it at `level` (counted from
    /// one), with `talented` saying which talents the carrier has learned,
    /// spawned after `steps` upgrade intervals.
    pub fn modifier(
        self,
        level: u8,
        talented: impl Fn(Talent) -> bool,
        steps: u32,
    ) -> ModifierKind {
        match self {
            AuraModifier::Kind(kind) => kind.grown(steps),
            AuraModifier::ArmorBroken => ModifierKind::ArmorBroken {
                armor: presence_armor(level, talented(Talent::PresenceArmor)),
            },
            AuraModifier::Spell(ability) => ModifierKind::Spell {
                ability,
                level,
                amount: 0,
            },
        }
    }
}

/// One aura.
///
/// Standing in it puts its modifier on afresh every tick, so what `linger`
/// buys is how long that holds once its wearer walks out, or once the aura
/// stops reaching it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aura {
    /// Which side it reaches.
    pub whom: Whom,
    /// Which units of that side it reaches.
    pub reaches: Reach,
    /// How far it reaches, in world units. One a neutral carries reaches no
    /// further than [`rules::NEUTRAL_AURA_RADIUS`](crate::rules::NEUTRAL_AURA_RADIUS).
    pub radius: i32,
    /// Ticks what it puts on holds once put on.
    pub linger: u32,
    /// Whether a Break silences it: Dota's `IsBreakable`, set on the
    /// passives of abilities, a creep's and a neutral's among them, but for a
    /// few the game leaves it off (the Granite Aura, the tornado's Tempest).
    /// A building's aura is no ability, and nothing breaks it.
    pub breakable: bool,
    /// Whether it reaches only those whose side sees its carrier.
    pub seen_only: bool,
    /// Whether it works only while its carrier stands in the river.
    pub water: bool,
    /// What it puts on.
    pub modifier: AuraModifier,
}
