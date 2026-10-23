//! What a kind of unit is, by its kind alone: one row per [`UnitKind`].
//!
//! Every rule that asks "what sort of thing is this" reads a column here
//! instead of matching on the kind, so a new kind is one proto variant and
//! one row.

use bota_proto::{Team, UnitKind, Vec2};

use crate::{AttackClass, MapDef, PriorityOrder, TargetClass, barracks_ranged_at, rules};

/// The row of one kind of unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KindDef {
    /// The kind the row is for: its place in [`UNIT_KINDS`].
    pub kind: UnitKind,
    /// What the kind is called in a panel or a feed, where the game names
    /// every unit of it alike; see [`unit_name`] for the kinds it names by
    /// more than the kind.
    pub name: &'static str,
    /// A building: it stands still and blocks the ground.
    pub structure: bool,
    /// Both sides always know where it is, whatever their vision.
    pub always_seen: bool,
    /// One of what a wave is made of.
    pub lane_creep: bool,
    /// What a wave is made of, or what the jungle grows.
    pub creep: bool,
    /// Stands where it was put: nothing pulls or drags it, and an aura for
    /// ordinary units passes it by.
    pub fixed: bool,
    /// Leaves anything to what is gathered from deaths: the flesh heap and
    /// the souls.
    pub leaves_a_death: bool,
    /// How its attacks and its body answer the attack class table.
    pub attack_class: AttackClass,
    /// The class it falls in when a unit picks what to attack.
    pub target_class: TargetClass,
    /// The class order it picks its own targets by.
    pub priority: PriorityOrder,
    /// Percent of its health below which its own side may put it out.
    /// Absent for a kind that is never denied.
    pub deny_below_pct: Option<i32>,
    /// A building a town portal scroll may carry to: Dota's allied
    /// `DOTA_UNIT_TARGET_BUILDING`.
    pub takes_scrolls: bool,
    /// Its blows on the other side break what breaks on damage. Read through
    /// [`breaks_regen`](crate::breaks_regen), the one rule for it.
    pub(crate) breaks_regen: bool,
}

/// A kind that is none of the special things, so a row names only what sets
/// it apart.
const PLAIN: KindDef = KindDef {
    kind: UnitKind::Hero,
    name: "Hero",
    structure: false,
    always_seen: false,
    lane_creep: false,
    creep: false,
    fixed: false,
    leaves_a_death: true,
    attack_class: AttackClass::Plain,
    target_class: TargetClass::Unit,
    priority: PriorityOrder::Normal,
    deny_below_pct: None,
    takes_scrolls: false,
    breaks_regen: false,
};

const fn lane_creep(kind: UnitKind, name: &'static str, attack_class: AttackClass) -> KindDef {
    KindDef {
        kind,
        name,
        lane_creep: true,
        creep: true,
        attack_class,
        deny_below_pct: Some(rules::DENY_HP_PCT),
        ..PLAIN
    }
}

/// A building, and whether a scroll carries to it.
const fn building(
    kind: UnitKind,
    name: &'static str,
    target_class: TargetClass,
    takes_scrolls: bool,
) -> KindDef {
    KindDef {
        kind,
        name,
        structure: true,
        always_seen: true,
        fixed: true,
        leaves_a_death: false,
        attack_class: AttackClass::Reinforced,
        target_class,
        deny_below_pct: Some(rules::DENY_BUILDING_HP_PCT),
        takes_scrolls,
        ..PLAIN
    }
}

/// Every kind of unit, indexed by its wire variant.
pub const UNIT_KINDS: [KindDef; 13] = [
    KindDef {
        kind: UnitKind::Hero,
        attack_class: AttackClass::Hero,
        breaks_regen: true,
        ..PLAIN
    },
    lane_creep(UnitKind::CreepMelee, "Melee Creep", AttackClass::Runty),
    lane_creep(
        UnitKind::CreepFlagbearer,
        "Flagbearer Creep",
        AttackClass::Runty,
    ),
    lane_creep(UnitKind::CreepRanged, "Ranged Creep", AttackClass::Piercing),
    KindDef {
        target_class: TargetClass::Siege,
        priority: PriorityOrder::SiegeFirst,
        ..lane_creep(UnitKind::CreepSiege, "Siege Creep", AttackClass::Reinforced)
    },
    KindDef {
        kind: UnitKind::CreepNeutral,
        name: "Neutral Creep",
        creep: true,
        ..PLAIN
    },
    building(UnitKind::Tower, "Tower", TargetClass::Building, true),
    building(
        UnitKind::Ancient,
        "The Ancient",
        TargetClass::Building,
        true,
    ),
    building(UnitKind::Barracks, "Barracks", TargetClass::Unit, true),
    building(UnitKind::Fountain, "Fountain", TargetClass::Building, true),
    KindDef {
        kind: UnitKind::Ward,
        name: "Ward",
        fixed: true,
        leaves_a_death: false,
        target_class: TargetClass::Ward,
        ..PLAIN
    },
    KindDef {
        kind: UnitKind::Courier,
        name: "Courier",
        ..PLAIN
    },
    building(UnitKind::Filler, "Building", TargetClass::Unit, false),
];

const _: () = {
    let mut at = 0;
    while at < UNIT_KINDS.len() {
        assert!(row(UNIT_KINDS[at].kind) == at);
        at += 1;
    }
};

/// The row of a kind in [`UNIT_KINDS`]. Spelled out so that a new kind fails
/// to compile here until it has its row.
const fn row(kind: UnitKind) -> usize {
    match kind {
        UnitKind::Hero
        | UnitKind::CreepMelee
        | UnitKind::CreepFlagbearer
        | UnitKind::CreepRanged
        | UnitKind::CreepSiege
        | UnitKind::CreepNeutral
        | UnitKind::Tower
        | UnitKind::Ancient
        | UnitKind::Barracks
        | UnitKind::Fountain
        | UnitKind::Ward
        | UnitKind::Courier
        | UnitKind::Filler => kind as usize,
    }
}

/// What a kind of unit is.
#[inline]
pub const fn kind_def(kind: UnitKind) -> &'static KindDef {
    &UNIT_KINDS[row(kind)]
}

/// What the game calls one unit on a map, from its kind, its side and where
/// it stands: the barracks that feeds the ranged creeps is the one the map
/// puts there, and each Ancient is its side's own.
pub fn unit_name(map: &MapDef, kind: UnitKind, team: Team, pos: Vec2) -> &'static str {
    match (kind, team) {
        (UnitKind::Ancient, Team::Radiant) => "Radiant's Ancient",
        (UnitKind::Ancient, Team::Dire) => "Dire's Ancient",
        (UnitKind::Barracks, _) if barracks_ranged_at(map, team, pos) == Some(true) => {
            "Ranged Barracks"
        }
        (UnitKind::Barracks, _) => "Melee Barracks",
        _ => kind_def(kind).name,
    }
}
