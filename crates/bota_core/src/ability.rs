//! Every ability a hero may carry: what it is called, how it is aimed, and
//! what it asks for. What it does is the server's: it keeps a hook table
//! with one row to each entry here.

use bota_proto::{AbilityId, Aim};

use crate::rules;

/// One entry of the ability list.
#[derive(Clone, Copy, Debug)]
pub struct AbilityDef {
    /// What it is called.
    pub name: &'static str,
    /// How it is aimed.
    pub aim: Aim,
    /// Levels it may reach.
    pub max_level: u8,
    /// Whether it works on its own and is never cast.
    pub passive: bool,
    /// Whether it is the ultimate, which waits on higher hero levels.
    pub ultimate: bool,
    /// Whether what it is aimed at has to be something its caster fights.
    pub at_an_enemy: bool,
    /// Whether ordering it leaves what the body is doing alone: it goes off
    /// beside a swing or another cast instead of breaking either off. Only
    /// an ability with no cast point, backswing or duration may be.
    pub immediate: bool,
    /// Whether ordering it while a channel runs leaves the channel be:
    /// Dota's `DOTA_ABILITY_BEHAVIOR_IGNORE_CHANNEL`. Only an ability with
    /// no cast point, backswing or duration may.
    pub ignores_channel: bool,
    /// Mana each level costs.
    pub mana: &'static [i32],
    /// Ticks between casts, by level.
    pub cooldown: &'static [u32],
    /// How far it reaches, in world units. Zero for one that reaches nowhere.
    pub range: i32,
    /// Milliseconds of cast point. Zero: the effect lands in the same tick.
    pub point: u32,
    /// Milliseconds of animation after. Zero: the body is not held.
    pub backswing: u32,
    /// Ticks it runs on after the cast. Zero: over at once.
    pub duration: u32,
}

/// Nothing at all, so an entry names only what sets it apart.
const PLAIN: AbilityDef = AbilityDef {
    name: "",
    aim: Aim::Own,
    max_level: 0,
    passive: true,
    ultimate: false,
    at_an_enemy: false,
    immediate: false,
    ignores_channel: false,
    mana: &[],
    cooldown: &[],
    range: 0,
    point: 0,
    backswing: 0,
    duration: 0,
};

/// How many abilities the game has: every table indexed by [`AbilityId`] is
/// this long.
pub const ABILITY_COUNT: usize = 17;

/// Every ability, indexed by [`AbilityId`].
pub const ABILITIES: [AbilityDef; ABILITY_COUNT] = [
    AbilityDef {
        name: "Meat Hook",
        aim: Aim::Point,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &rules::HOOK_MANA,
        cooldown: &rules::HOOK_COOLDOWN,
        range: rules::HOOK_RANGE,
        ..PLAIN
    },
    AbilityDef {
        name: "Rot",
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        ignores_channel: true,
        mana: &[0, 0, 0, 0],
        cooldown: &[0, 0, 0, 0],
        range: rules::ROT_RADIUS,
        ..PLAIN
    },
    AbilityDef {
        name: "Flesh Heap",
        max_level: rules::ABILITY_MAX_LEVEL,
        range: rules::FLESH_HEAP_RANGE,
        ..PLAIN
    },
    AbilityDef {
        name: "Dismember",
        aim: Aim::Unit,
        max_level: rules::ULT_MAX_LEVEL,
        passive: false,
        ultimate: true,
        at_an_enemy: true,
        mana: &rules::DISMEMBER_MANA,
        cooldown: &rules::DISMEMBER_COOLDOWN,
        range: rules::DISMEMBER_RANGE,
        duration: rules::DISMEMBER_TICKS,
        ..PLAIN
    },
    AbilityDef {
        name: "Burst",
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown: &[rules::COURIER_BURST_COOLDOWN],
        ..PLAIN
    },
    AbilityDef {
        name: "Return",
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown: &[0],
        ..PLAIN
    },
    AbilityDef {
        name: "Take Stash",
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown: &[0],
        ..PLAIN
    },
    AbilityDef {
        name: "Deliver",
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown: &[0],
        ..PLAIN
    },
    AbilityDef {
        name: "Shield",
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown: &[rules::COURIER_SHIELD_COOLDOWN],
        ..PLAIN
    },
    AbilityDef {
        name: "Shadowraze (Near)",
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &rules::RAZE_MANA,
        cooldown: &rules::RAZE_COOLDOWN,
        range: rules::RAZE_DISTANCE[0],
        point: rules::RAZE_CAST_POINT_MS,
        ..PLAIN
    },
    AbilityDef {
        name: "Shadowraze (Mid)",
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &rules::RAZE_MANA,
        cooldown: &rules::RAZE_COOLDOWN,
        range: rules::RAZE_DISTANCE[1],
        point: rules::RAZE_CAST_POINT_MS,
        ..PLAIN
    },
    AbilityDef {
        name: "Shadowraze (Far)",
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &rules::RAZE_MANA,
        cooldown: &rules::RAZE_COOLDOWN,
        range: rules::RAZE_DISTANCE[2],
        point: rules::RAZE_CAST_POINT_MS,
        ..PLAIN
    },
    AbilityDef {
        name: "Requiem of Souls",
        max_level: rules::ULT_MAX_LEVEL,
        passive: false,
        ultimate: true,
        mana: &rules::REQUIEM_MANA,
        cooldown: &rules::REQUIEM_COOLDOWN,
        range: rules::REQUIEM_LINE_DISTANCE,
        point: rules::REQUIEM_CAST_POINT_MS,
        backswing: rules::REQUIEM_BACKSWING_MS,
        ..PLAIN
    },
    AbilityDef {
        name: "Necromastery",
        max_level: 1,
        ..PLAIN
    },
    AbilityDef {
        name: "Presence of the Dark Lord",
        max_level: rules::ABILITY_MAX_LEVEL,
        range: rules::PRESENCE_RADIUS,
        ..PLAIN
    },
    AbilityDef {
        name: "Feast of Souls",
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        immediate: true,
        mana: &rules::FEAST_MANA,
        cooldown: &rules::FEAST_COOLDOWN,
        range: rules::FEAST_RADIUS,
        ..PLAIN
    },
    AbilityDef {
        name: "Throw Grenade",
        ..PLAIN
    },
];

const _: () = {
    let mut at = 0;
    while at < ABILITIES.len() {
        let def = &ABILITIES[at];
        assert!(!def.immediate || (def.point == 0 && def.backswing == 0 && def.duration == 0));
        assert!(
            !def.ignores_channel || (def.point == 0 && def.backswing == 0 && def.duration == 0)
        );
        at += 1;
    }
};

/// What one ability is, or nothing if no such ability exists.
#[inline]
pub fn ability_def(id: AbilityId) -> Option<&'static AbilityDef> {
    ABILITIES.get(usize::from(id.0))
}

/// What one cast costs at a level.
///
/// Zero for a slot that holds nothing castable.
#[inline]
pub fn ability_mana_cost(id: AbilityId, level: u8) -> i32 {
    ability_def(id).map_or(0, |def| pick(def.mana, level))
}

/// What one cast puts on the clock at a level.
#[inline]
pub fn ability_cooldown(id: AbilityId, level: u8) -> u32 {
    ability_def(id).map_or(0, |def| pick(def.cooldown, level))
}

/// The entry of a per-level table for a level, the last one standing for
/// everything past it.
#[inline]
fn pick<T: Copy + Default>(table: &[T], level: u8) -> T {
    let level = usize::from(level.max(1) - 1);
    table
        .get(level)
        .copied()
        .unwrap_or_else(|| table.last().copied().unwrap_or_default())
}

/// The ability whose level a whole group of slots stands at.
///
/// The three razes are one skill worn as three slots: a point into any of
/// them levels all three, and the group costs points as one ability. Every
/// other ability is a group of itself.
pub fn learn_group(id: AbilityId) -> AbilityId {
    match id {
        AbilityId::RAZE_MID | AbilityId::RAZE_FAR => AbilityId::RAZE_NEAR,
        _ => id,
    }
}

/// The hero level one more level of an ability waits for.
///
/// A basic one opens on every other level from the first; the ultimate waits
/// on [`rules::ULT_LEVEL_FLOORS`].
pub fn level_floor(def: &AbilityDef, level: u8) -> u8 {
    if def.ultimate {
        rules::ULT_LEVEL_FLOORS
            .get(usize::from(level))
            .copied()
            .unwrap_or(u8::MAX)
    } else {
        2 * (level + 1) - 1
    }
}
