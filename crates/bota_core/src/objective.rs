//! The map's objectives a match stands up when its rules ask for them
//! ([`ObjectiveSet`]): the Outposts, the Watchers, the Shrines of Wisdom and
//! the Twin Gates of Dota 7.41f, as numbers, places and units.
//!
//! Sources, tagged where each number is used: `[U:n]` and `[A:n]` the lines
//! of the game's `scripts/npc/npc_units.txt` and `npc_abilities.txt` (client
//! 6966, 7.41f; unchanged in 6972); `[ENT:n]` the entity blocks of the map's
//! `maps/dota/entities/default_ents.vents` (7.41f), shifted by half the map
//! like every landmark; `[BIN]` a server convar default or code constant of
//! the 6966 build (6972 for the Twin Gates); `[PN v]` Valve's patch notes of
//! version `v`; `[LIQ page]` Liquipedia's article (read 2026-10-24, the Twin
//! Gate's 2026-11-06); `[FAN page]` the Dota 2 Wiki's.

use bota_proto::{AbilityId, Aim, Angle, Fixed, ObjectiveKind, ObjectiveSet, Team, UnitKind, Vec2};

use crate::ability::BARE_ABILITY;
use crate::unit::BARE_UNIT;
use crate::{AbilityDef, Pierces, StructureId, TargetFilter, UnitDef, Until, rules};

/// The objectives the simulation stands up and plays.
pub const SIMULATED_OBJECTIVES: ObjectiveSet = ObjectiveSet::of(ObjectiveKind::Watchers)
    .with(ObjectiveKind::Outposts)
    .with(ObjectiveKind::Shrines)
    .with(ObjectiveKind::LotusPools)
    .with(ObjectiveKind::Roshan)
    .with(ObjectiveKind::TwinGates);

/// The kind of unit an objective stands as, for those that stand as one.
pub const fn objective_unit(objective: ObjectiveKind) -> Option<UnitKind> {
    match objective {
        ObjectiveKind::Watchers => Some(UnitKind::Watcher),
        ObjectiveKind::Outposts => Some(UnitKind::Outpost),
        ObjectiveKind::Shrines => Some(UnitKind::Shrine),
        ObjectiveKind::LotusPools => Some(UnitKind::LotusPool),
        ObjectiveKind::Roshan => Some(UnitKind::Roshan),
        ObjectiveKind::Tormentor => None,
        ObjectiveKind::TwinGates => Some(UnitKind::TwinGate),
    }
}

/// Ticks past the horn the Outposts and the Watchers may first be worked:
/// none, the horn itself: the Outposts' entities are `vulnerableoncreepspawn
/// 1` [ENT:193780, ENT:193871], and a Watcher "cannot be activated before the
/// horn" [PN 7.39].
pub const OBJECTIVES_OPEN_AFTER_HORN: u32 = 0;

// Outposts (`npc_dota_watch_tower`) and `ability_capture` [A:142-163].

/// Outpost health [U:3866]; it is invulnerable all the same.
pub const OUTPOST_HP: i32 = 450;
/// How near a hero must stand to capture an Outpost: `AbilityCastRange`.
pub const CAPTURE_RANGE: i32 = 300;
/// Ticks one hero channels to capture an Outpost alone: `base_capture_time`
/// 6.0. Several heroes of one side channelling at it share it: 6 s over
/// their number (`ally_exponent` 1.0) [LIQ Outpost].
pub const CAPTURE_TICKS: u32 = 6 * rules::TICKS_PER_SECOND;

/// One Outpost of a map: where it stands, the side it starts with and that
/// guards it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutpostDef {
    /// Where it stands.
    pub at: Vec2,
    /// The side on whose half it stands: it holds the Outpost at the start,
    /// and the other side may capture it only once one of this side's
    /// tier-two towers has fallen [LIQ Buildings, Neutral Buildings; LOC
    /// `DOTA_Tooltip_modifier_watch_tower_invulnerable_Description`].
    pub home: Team,
}

/// The Dota map's two Outposts [PN 7.38: the corner ones are gone].
pub const DOTA_OUTPOSTS: [OutpostDef; 2] = [
    // "npc_dota_watch_tower_bottom" (-4096, -448) [ENT:193780].
    OutpostDef {
        at: Vec2::from_ints(5120, 8768),
        home: Team::Radiant,
    },
    // "npc_dota_watch_tower_top" (3392, -448) [ENT:193871].
    OutpostDef {
        at: Vec2::from_ints(12608, 8768),
        home: Team::Dire,
    },
];

/// When the other side may capture an Outpost of its home side: once any
/// of that side's tier-two towers is down.
pub const OUTPOST_OPENS: Until = Until::AnyOf(&[
    Until::Down(StructureId::Tower {
        lane: rules::LANE_MID,
        tier: 2,
    }),
    Until::Down(StructureId::Tower {
        lane: rules::LANE_TOP,
        tier: 2,
    }),
    Until::Down(StructureId::Tower {
        lane: rules::LANE_BOT,
        tier: 2,
    }),
]);

/// An Outpost: an invulnerable building with no attack and no sight of its
/// own (7.33: "Outposts no longer provide vision"), on the filler hull
/// [U:3853-3879].
pub const OUTPOST: UnitDef = UnitDef {
    kind: UnitKind::Outpost,
    max_hp: OUTPOST_HP,
    collision: rules::FILLER_COLLISION,
    bound: rules::FILLER_BOUND,
    invulnerable: true,
    ..BARE_UNIT
};

/// Every hero's hidden `ability_capture`: a channel at an Outpost of the
/// other side, from [`CAPTURE_RANGE`]; no `IGNORE_SILENCE`, so a silence
/// keeps it from starting.
pub const CAPTURE: AbilityDef = AbilityDef {
    id: AbilityId::CAPTURE,
    key: "ability_capture",
    name: "Capture",
    aim: Aim::Unit,
    max_level: 1,
    passive: false,
    mana: &[0],
    cooldown: &[0],
    range: CAPTURE_RANGE,
    duration: CAPTURE_TICKS,
    pierces: Pierces::Yes,
    granted_at: Some(1),
    hidden: true,
    targets: TargetFilter::of(&[UnitKind::Outpost], false),
    ..BARE_ABILITY
};

// Watchers (`npc_dota_lantern`) and `ability_lamp_use` [A:168-194].

/// Watcher health [U:21469]; it is invulnerable all the same.
pub const WATCHER_HP: i32 = 100;
/// How far a lit Watcher sees for its side by day [U:21481]; ground sight
/// [LIQ Watcher].
pub const WATCHER_DAY_VISION: i32 = 800;
/// How far a lit Watcher sees for its side by night [U:21482].
pub const WATCHER_NIGHT_VISION: i32 = 450;
/// How near a hero must stand to light a Watcher: `AbilityCastRange` 200
/// [PN 7.39c].
pub const LAMP_RANGE: i32 = 200;
/// Ticks a hero channels to light or put out a Watcher: `AbilityChannelTime`
/// 1.0 [PN 7.40], whoever else channels.
pub const LAMP_CHANNEL_TICKS: u32 = rules::TICKS_PER_SECOND;
/// Ticks a lit Watcher sees for its side: `active_duration` 420.
pub const LAMP_LIT_TICKS: u32 = 420 * rules::TICKS_PER_SECOND;
/// Ticks a Watcher put out by the other side stays out: `inactive_duration`
/// 120.
pub const LAMP_OFF_TICKS: u32 = 120 * rules::TICKS_PER_SECOND;

/// The Dota map's ten Watchers, all of the neutral team; none stands within
/// about 2300 of the mid lane since 7.40.
pub const DOTA_WATCHERS: [Vec2; 10] = [
    // (7917.52, -1523.73) [ENT:185729].
    Vec2::from_ints(17134, 7692),
    // (-6464, 6656) [ENT:185831].
    Vec2::from_ints(2752, 15872),
    // (2717.69, -657.45) [ENT:185974].
    Vec2::from_ints(11934, 8559),
    // (-3483.5, 34.38) [ENT:186076].
    Vec2::from_ints(5733, 9250),
    // (1032.26, -4260.07) [ENT:186178].
    Vec2::from_ints(10248, 4956),
    // (6528, -6592) [ENT:186403].
    Vec2::from_ints(15744, 2624),
    // (768, -7648) [ENT:186733].
    Vec2::from_ints(9984, 1568),
    // (-670.88, 7129.75) [ENT:186835].
    Vec2::from_ints(8545, 16346),
    // (-1527.40, 3925.89) [ENT:187288].
    Vec2::from_ints(7689, 13142),
    // (-7808, 1056) [ENT:188210].
    Vec2::from_ints(1408, 10272),
];

/// A Watcher: an invulnerable lantern of the neutral team, on the small hull
/// [U:21460-21482]. Its sight is the neutral team's; a side sees through it
/// only while it is lit for that side.
pub const WATCHER: UnitDef = UnitDef {
    kind: UnitKind::Watcher,
    max_hp: WATCHER_HP,
    day_vision: WATCHER_DAY_VISION,
    night_vision: WATCHER_NIGHT_VISION,
    collision: rules::RANGED_CREEP_COLLISION,
    bound: rules::RANGED_CREEP_BOUND,
    invulnerable: true,
    ..BARE_UNIT
};

/// Every hero's hidden `ability_lamp_use`: a channel at a Watcher of either
/// team, from [`LAMP_RANGE`]; `IGNORE_SILENCE`, so a silence does not stop
/// it [LIQ Watcher].
pub const LAMP_USE: AbilityDef = AbilityDef {
    id: AbilityId::LAMP_USE,
    key: "ability_lamp_use",
    name: "Lamp Use",
    aim: Aim::Unit,
    max_level: 1,
    passive: false,
    mana: &[0],
    cooldown: &[0],
    range: LAMP_RANGE,
    duration: LAMP_CHANNEL_TICKS,
    pierces: Pierces::Yes,
    granted_at: Some(1),
    hidden: true,
    targets: TargetFilter::of(&[UnitKind::Watcher], true),
    ..BARE_ABILITY
};

// Shrines of Wisdom (`npc_dota_xp_fountain`) and `ability_xp_fountain`
// [A:375-395].

/// Shrine health [U:21855-21914]; it is invulnerable all the same.
pub const SHRINE_HP: i32 = 1750;
/// How near a hero must stand to gather a Shrine: `radius`.
pub const SHRINE_RADIUS: i32 = 300;
/// Ticks a side's heroes must stand in a Shrine's radius to gather it:
/// `countdown_time` 3 [PN 7.38].
pub const SHRINE_COUNTDOWN_TICKS: u32 = 3 * rules::TICKS_PER_SECOND;
/// Ticks past the horn a Shrine first holds experience: 7:00 [LIQ Shrine of
/// Wisdom].
pub const SHRINE_FIRST_AFTER_HORN: u32 = 7 * 60 * rules::TICKS_PER_SECOND;
/// Ticks between two activations: `dota_xp_fountain_activation_interval`
/// 420 [BIN], bound to the game clock, not to the last gathering [PN 7.38].
pub const SHRINE_PERIOD_TICKS: u32 = 420 * rules::TICKS_PER_SECOND;
/// Experience of the first activation [PN 7.41: "200 base and 300 per
/// subsequent shrine (200/500/800 etc)"].
pub const SHRINE_XP_FIRST: i32 = 200;
/// Experience each later activation adds.
pub const SHRINE_XP_STEP: i32 = 300;

/// The Dota map's two Shrines of Wisdom.
pub const DOTA_SHRINES: [Vec2; 2] = [
    // (-8088, 768) [ENT:187174].
    Vec2::from_ints(1128, 9984),
    // (8167.43, -1142.11) [ENT:188562].
    Vec2::from_ints(17383, 8074),
];

/// A Shrine of Wisdom: an invulnerable building of the neutral team with no
/// sight, a plaza with no body that heroes walk onto and stand in, as Dota
/// 7.41f plays it (the filler hull of its `BoundsHullName` [U:21855-21914]
/// blocks nothing there; its kind is `walkable`), with the filler's bound
/// [LIQ Shrine of Wisdom].
pub const SHRINE: UnitDef = UnitDef {
    kind: UnitKind::Shrine,
    max_hp: SHRINE_HP,
    collision: 0,
    bound: rules::FILLER_BOUND,
    invulnerable: true,
    ..BARE_UNIT
};

/// The activations of the Shrines the game clock has passed by `clock`,
/// counted from one; zero before the first.
pub const fn shrine_activations(clock: u32, schedule: rules::Schedule) -> u32 {
    let first = schedule.horn + SHRINE_FIRST_AFTER_HORN;
    if clock < first {
        0
    } else {
        (clock - first) / SHRINE_PERIOD_TICKS + 1
    }
}

/// Experience a Shrine gathered on its `activation`, counted from one,
/// gives: 200, 500, 800 and on, the activation read off the game clock.
pub const fn shrine_xp(activation: u32) -> i32 {
    let later = activation.saturating_sub(1);
    SHRINE_XP_FIRST + SHRINE_XP_STEP * later as i32
}

// Twin Gates (`npc_dota_unit_twin_gate`) and `twin_gate_portal_warp`
// [U:20700-20788, A:255-282].

/// Twin Gate health [U:20774]; it is invulnerable all the same [LIQ
/// Buildings/Neutral Buildings: "permanently invulnerable"].
pub const TWIN_GATE_HP: i32 = 450;
/// How far a Twin Gate sees round it, by day and by night [U:20786-20787],
/// for the neutral team it stands for: no side sees through it ("enemies
/// cannot see when a Twin Gate gets used without having vision over them"
/// [LIQ Buildings/Neutral Buildings]).
pub const TWIN_GATE_VISION: i32 = 500;
/// How near a unit must stand to channel a warp: `AbilityCastRange` 200
/// [A:273].
pub const WARP_RANGE: i32 = 200;
/// Ticks a warp channels: `AbilityChannelTime` 4.0 [A:267]; 3 s at 7.33,
/// "Increased channel time for Twin Gates by 1 second" [PN 7.33b].
pub const WARP_CHANNEL_TICKS: u32 = 4 * rules::TICKS_PER_SECOND;
/// Mana a warp costs: `AbilityManaCost` 30 [A:268]; "Twin Gate mana cost
/// decreased from 75 to 30" [PN 7.40]. Given back when the channel is broken
/// off [PN 7.40: "Twin Gates now refund the mana cost if the teleporting
/// channel was interrupted"]; free to a unit with no mana at all [PN 7.35:
/// "Units without mana can use Twin Gates for free"].
pub const WARP_MANA: i32 = 30;
/// Where a warp lands: the other gate's centre moved 140 along the map's
/// y axis, south, the way both gates face (yaw 270 and 267.6 [ENT:193962,
/// ENT:194065]). The server's `twin_gate_portal_warp` adds -140.0 to the
/// other gate's origin's y (+140.0 to its x for the Templar gates of
/// another mode only) and finds clear space for the unit there [BIN]; the
/// unit is then turned to face away from the gate it came out of [BIN].
pub const WARP_EXIT_SOUTH: i32 = 140;

/// One Twin Gate of a map: where it stands, which way it faces and where a
/// unit that warps to it comes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TwinGateDef {
    /// Where it stands.
    pub at: Vec2,
    /// The way it faces.
    pub facing: Angle,
    /// Where a unit carried to it lands, before open ground is looked for
    /// round it: [`WARP_EXIT_SOUTH`] south of `at`.
    pub exit: Vec2,
    /// The side whose look it takes on: Dota draws `npc_dota_portal1`, the
    /// north-western gate, in the Dire's colours and the other in the
    /// Radiant's [BIN: `modifier_twin_gate_fx` picks
    /// `team_portal_dire_ambient.vpcf` for `npc_dota_portal1`]; it belongs
    /// to neither [LIQ Buildings/Neutral Buildings].
    pub look: Team,
}

/// A Twin Gate at a world spot given in hundredths of a unit, already
/// shifted by half the map, facing `yaw` in hundredths of a degree.
const fn twin_gate(
    x_hundredths: i32,
    y_hundredths: i32,
    yaw_hundredths: i32,
    look: Team,
) -> TwinGateDef {
    let at = Vec2 {
        x: Fixed::from_ratio(x_hundredths, 100),
        y: Fixed::from_ratio(y_hundredths, 100),
    };
    TwinGateDef {
        at,
        facing: Angle {
            brads: ((yaw_hundredths as i64 * 65536 + 18_000) / 36_000) as u16,
        },
        exit: Vec2 {
            x: at.x,
            y: Fixed::from_ratio(y_hundredths - WARP_EXIT_SOUTH * 100, 100),
        },
        look,
    }
}

/// The Dota map's two Twin Gates, at the map's north-western and
/// south-eastern corners [PN 7.38, PN 7.41]; each carries to the other.
pub const DOTA_TWIN_GATES: [TwinGateDef; 2] = [
    // "npc_dota_portal1" (-6457.690918, 7599.036621), yaw 270.000061
    // [ENT:193962].
    twin_gate(275_831, 1_681_504, 27_000, Team::Dire),
    // "npc_dota_portal2" (6425.52832, -7313.782715), yaw 267.599823
    // [ENT:194065].
    twin_gate(1_564_153, 190_222, 26_760, Team::Radiant),
];

/// A Twin Gate: an invulnerable building of the neutral team on the filler
/// hull, with no attack and no mana [U:20700-20788; LIQ Twin Gate].
pub const TWIN_GATE: UnitDef = UnitDef {
    kind: UnitKind::TwinGate,
    max_hp: TWIN_GATE_HP,
    day_vision: TWIN_GATE_VISION,
    night_vision: TWIN_GATE_VISION,
    collision: rules::FILLER_COLLISION,
    bound: rules::FILLER_BOUND,
    invulnerable: true,
    ..BARE_UNIT
};

/// Every unit's hidden `twin_gate_portal_warp` [A:255-282]: a channel at a
/// Twin Gate from [`WARP_RANGE`] that carries its caster to the other gate.
/// No cooldown, at any time of the match: the ability has no
/// `AbilityCooldown` and nothing in the patch notes or the server's convars
/// opens the gates later. `IGNORE_SILENCE`; `ROOT_DISABLES`, and since 7.41e
/// a root breaks it off too [PN 7.41e: "Channeling the Twin Gate can now be
/// interrupted by root"]; damage does not, as it breaks off no channel. Any
/// unit holds it, a courier too ("This is an innate ability for all units";
/// "can be initiated by any unit, except for illusions that cannot cast
/// abilities" [LIQ Twin Gate; FAN Twin Gate]); an illusion casts nothing, so
/// none warps. `IGNORE_SILENCE` asks nothing of the simulation yet: it has no
/// silence.
pub const TWIN_GATE_WARP: AbilityDef = AbilityDef {
    id: AbilityId::TWIN_GATE_WARP,
    key: "twin_gate_portal_warp",
    name: "Warp",
    aim: Aim::Unit,
    max_level: 1,
    passive: false,
    mana: &[WARP_MANA],
    cooldown: &[0],
    range: WARP_RANGE,
    duration: WARP_CHANNEL_TICKS,
    pierces: Pierces::Yes,
    granted_at: Some(1),
    hidden: true,
    root_disables: true,
    free_without_mana: true,
    every_unit: true,
    targets: TargetFilter::of(&[UnitKind::TwinGate], false),
    ..BARE_ABILITY
};
