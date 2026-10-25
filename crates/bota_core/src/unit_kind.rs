//! What a kind of unit is, by its kind alone: one row per [`UnitKind`].
//!
//! Every rule that asks "what sort of thing is this" reads a column here
//! instead of matching on the kind, so a new kind is one proto variant and
//! one row.

use bota_proto::{AbilityId, UnitKind};

use crate::{AttackClass, PriorityOrder, TargetClass, rules};

/// The row of one kind of unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KindDef {
    /// The kind the row is for: its place in [`UNIT_KINDS`].
    pub kind: UnitKind,
    /// The rules' identifier for the kind, for logs and tests; not a
    /// player-facing label.
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
    /// What an area spell aimed at units strikes: Dota's hero and basic unit
    /// types. Not a building, and not a ward, which only attacks bring down.
    pub struck_by_area_spells: bool,
    /// Its fall is announced to both sides wherever it stands: Dota's kill
    /// feed and announcer tell every hero kill and deny, a tower's,
    /// a barracks' and an Ancient's fall and a courier's loss.
    pub death_announced: bool,
    /// Attacks at it never miss and melee attackers reach
    /// [`rules::WARD_MELEE_RANGE_BONUS`] further at it, as at a ward in Dota.
    pub surely_struck: bool,
    /// Who brought it down is announced with its fall when a hero did: the
    /// kill feed names the hero that killed a hero, a tower or a barracks
    /// (and only the side of any other killer), but not a courier's.
    pub killer_announced: bool,
    /// Its blows on the other side break what breaks on damage. Read through
    /// [`breaks_regen`](crate::breaks_regen), the one rule for it.
    pub(crate) breaks_regen: bool,
    /// Ticks its side's Glyph of Fortification holds on it, put on what
    /// stands when the Glyph is used: "all friendly buildings and lane
    /// creeps" (Liquipedia, *Glyph of Fortification*), the base's buildings
    /// with the fountain and the Effigies, its fillers (Liquipedia, *Map*),
    /// among them. Absent for a kind the Glyph leaves bare: the neutral
    /// buildings, "permanently invulnerable" and "immune to any ability"
    /// (Liquipedia, *Buildings*), an Outpost a side holds included.
    pub glyphed_for: Option<u32>,
    /// The hidden ability a hero casts at it when ordered to attack it or to
    /// move to it, as Dota turns a click on an Outpost or a Watcher into
    /// `ability_capture` or `ability_lamp_use`. Absent for a kind a click
    /// only attacks or follows.
    pub interact: Option<AbilityId>,
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
    struck_by_area_spells: true,
    surely_struck: false,
    death_announced: false,
    killer_announced: false,
    breaks_regen: false,
    glyphed_for: None,
    interact: None,
};

const fn lane_creep(kind: UnitKind, name: &'static str, attack_class: AttackClass) -> KindDef {
    KindDef {
        kind,
        name,
        lane_creep: true,
        creep: true,
        attack_class,
        deny_below_pct: Some(rules::DENY_HP_PCT),
        glyphed_for: Some(rules::GLYPH_CREEP_TICKS),
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
        struck_by_area_spells: false,
        death_announced: true,
        glyphed_for: Some(rules::GLYPH_STRUCTURE_TICKS),
        ..PLAIN
    }
}

/// A neutral building of the map's objectives: invulnerable, it never
/// falls, so nothing denies it and nothing announces its fall, and the
/// Glyph leaves it bare.
const fn objective(kind: UnitKind, name: &'static str, takes_scrolls: bool) -> KindDef {
    KindDef {
        deny_below_pct: None,
        death_announced: false,
        glyphed_for: None,
        ..building(kind, name, TargetClass::Building, takes_scrolls)
    }
}

/// Every kind of unit, indexed by its wire variant.
pub const UNIT_KINDS: [KindDef; 16] = [
    KindDef {
        kind: UnitKind::Hero,
        attack_class: AttackClass::Hero,
        death_announced: true,
        killer_announced: true,
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
    KindDef {
        killer_announced: true,
        ..building(UnitKind::Tower, "Tower", TargetClass::Building, true)
    },
    building(
        UnitKind::Ancient,
        "The Ancient",
        TargetClass::Building,
        true,
    ),
    KindDef {
        killer_announced: true,
        ..building(UnitKind::Barracks, "Barracks", TargetClass::Unit, true)
    },
    building(UnitKind::Fountain, "Fountain", TargetClass::Building, true),
    KindDef {
        kind: UnitKind::Ward,
        name: "Ward",
        fixed: true,
        leaves_a_death: false,
        target_class: TargetClass::Ward,
        struck_by_area_spells: false,
        surely_struck: true,
        deny_below_pct: Some(rules::WARD_DENY_HP_PCT),
        ..PLAIN
    },
    KindDef {
        kind: UnitKind::Courier,
        name: "Courier",
        death_announced: true,
        ..PLAIN
    },
    // Dota announces no filler building's fall.
    KindDef {
        death_announced: false,
        ..building(UnitKind::Filler, "Building", TargetClass::Unit, false)
    },
    // A side teleports to the Outposts it holds [LIQ Outpost].
    KindDef {
        interact: Some(AbilityId::CAPTURE),
        ..objective(UnitKind::Outpost, "Outpost", true)
    },
    KindDef {
        interact: Some(AbilityId::LAMP_USE),
        ..objective(UnitKind::Watcher, "Watcher", false)
    },
    objective(UnitKind::Shrine, "Shrine of Wisdom", false),
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
        | UnitKind::Filler
        | UnitKind::Outpost
        | UnitKind::Watcher
        | UnitKind::Shrine => kind as usize,
    }
}

/// What a kind of unit is.
#[inline]
pub const fn kind_def(kind: UnitKind) -> &'static KindDef {
    &UNIT_KINDS[row(kind)]
}
