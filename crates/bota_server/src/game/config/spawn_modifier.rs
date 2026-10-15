//! Modifiers trusted match setup puts on units as they are stood up.

use bota_proto::{ModifierSpec, Team, UnitKind, modifier_ticks_bounded};

use crate::game::{is_structure, rules};

/// The most rules one match setup may carry.
pub const MAX_SPAWN_MODIFIERS: usize = 64;
/// The most exact kinds and categories one selector may walk.
pub const MAX_SPAWN_TARGETS: usize = 16;
/// The most entries one unit may carry: every setup rule and one cheat.
pub const MAX_APPLIED_MODIFIERS_PER_UNIT: usize = MAX_SPAWN_MODIFIERS + 1;
/// The highest additive scale all entries on one unit can reach.
pub const MAX_COMBINED_MODIFIER_SCALE: i32 = rules::NOMINAL_BP
    + MAX_APPLIED_MODIFIERS_PER_UNIT as i32 * (ModifierSpec::MAX_SCALE - rules::NOMINAL_BP);

/// One rule: what to put on which spawns, and for how long.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnModifier {
    /// Which spawns it lands on.
    pub select: SpawnSelector,
    /// What it changes.
    pub spec: ModifierSpec,
    /// How long each application lasts.
    pub duration: ModifierDuration,
}

/// Which units one spawn modifier lands on.
///
/// An absent field takes everything that field could name; the fields narrow
/// one another.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpawnSelector {
    /// Which side it takes. `None` takes either side.
    pub team: Option<Team>,
    /// Which kinds it takes, `MAX_SPAWN_TARGETS` at most. Empty takes every
    /// kind; duplicates do not apply the rule twice.
    pub targets: Vec<SpawnTarget>,
}

impl SpawnSelector {
    /// Whether a standing unit is one this selector takes.
    pub fn takes(&self, kind: UnitKind, team: Option<Team>) -> bool {
        assert!(
            self.targets.len() <= MAX_SPAWN_TARGETS,
            "a selector may carry at most {MAX_SPAWN_TARGETS} targets"
        );
        if let Some(wanted) = self.team
            && team != Some(wanted)
        {
            return false;
        }
        self.targets.is_empty() || self.targets.iter().any(|target| target.takes(kind))
    }
}

/// One kind or category of unit a spawn modifier can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnTarget {
    /// Every unit of one exact kind.
    Kind(UnitKind),
    /// Every unit in a category.
    Category(SpawnCategory),
}

impl SpawnTarget {
    /// Whether a kind is the one this target names.
    pub fn takes(&self, kind: UnitKind) -> bool {
        match self {
            SpawnTarget::Kind(exact) => *exact == kind,
            SpawnTarget::Category(category) => category.takes(kind),
        }
    }
}

/// A group of unit kinds a spawn modifier can name at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnCategory {
    /// Every hero.
    Hero,
    /// Lane creeps: melee, flagbearer, ranged and siege.
    LaneCreep,
    /// Neutral camp creeps.
    NeutralCreep,
    /// Buildings: towers, ancients, barracks and fountains.
    Structure,
}

impl SpawnCategory {
    /// Whether a kind belongs to this category.
    pub fn takes(&self, kind: UnitKind) -> bool {
        match self {
            SpawnCategory::Hero => kind == UnitKind::Hero,
            SpawnCategory::LaneCreep => matches!(
                kind,
                UnitKind::CreepMelee
                    | UnitKind::CreepFlagbearer
                    | UnitKind::CreepRanged
                    | UnitKind::CreepSiege
            ),
            SpawnCategory::NeutralCreep => kind == UnitKind::CreepNeutral,
            SpawnCategory::Structure => is_structure(kind),
        }
    }
}

/// How long one application lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierDuration {
    /// Until the body falls. Every spawn gets it afresh.
    MatchLong,
    /// A number of ticks, counted from each application.
    Ticks(u32),
}

impl ModifierDuration {
    /// What each application starts its countdown with. Absent runs until
    /// the body falls.
    pub fn ticks_left(self) -> Option<u32> {
        match self {
            ModifierDuration::MatchLong => None,
            ModifierDuration::Ticks(ticks) => Some(ticks),
        }
    }

    /// Whether the policy is one a match setup may carry.
    pub fn is_valid(self) -> bool {
        match self {
            ModifierDuration::MatchLong => true,
            ModifierDuration::Ticks(ticks) => modifier_ticks_bounded(ticks),
        }
    }
}

/// Why one spawn modifier was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnModifierError {
    /// The selector asks to walk too many exact kinds and categories.
    TooManyTargets {
        /// How many were offered.
        count: usize,
    },
    /// The spec does not lie within its bounds.
    UnboundedSpec,
    /// The duration is not one a match may carry.
    BadDuration,
}

impl core::fmt::Display for SpawnModifierError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SpawnModifierError::TooManyTargets { count } => write!(
                f,
                "{count} selector targets exceed the {MAX_SPAWN_TARGETS} a rule may carry"
            ),
            SpawnModifierError::UnboundedSpec => write!(f, "the modifier spec is out of bounds"),
            SpawnModifierError::BadDuration => write!(f, "the duration is out of range"),
        }
    }
}

impl std::error::Error for SpawnModifierError {}

/// Why a rule is not one a match setup may carry, or nothing when it is.
pub fn check_spawn_modifier(rule: &SpawnModifier) -> Result<(), SpawnModifierError> {
    if rule.select.targets.len() > MAX_SPAWN_TARGETS {
        return Err(SpawnModifierError::TooManyTargets {
            count: rule.select.targets.len(),
        });
    }
    if !rule.spec.is_bounded() {
        return Err(SpawnModifierError::UnboundedSpec);
    }
    if !rule.duration.is_valid() {
        return Err(SpawnModifierError::BadDuration);
    }
    Ok(())
}
