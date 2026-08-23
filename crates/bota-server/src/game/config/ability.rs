//! Every ability a hero may carry, and what each one costs to use.
//!
//! What an ability does is the cast system's business; what it is called, how
//! it is aimed and what it asks for is here.

use bota_proto::{AbilityId, Aim};

use crate::game::rules;

/// One entry of the ability list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Mana each level costs.
    pub mana: &'static [i32],
    /// Ticks between casts, by level.
    pub cooldown: &'static [u32],
    /// How far it reaches, in world units. Zero for one that reaches nowhere.
    pub range: i32,
}

/// Sylla's critical strike: nothing is cast, it simply happens.
pub const CRIT: AbilityId = AbilityId(0);
/// Sylla's frenzy: attacks come faster for a while.
pub const FRENZY: AbilityId = AbilityId(1);
/// Sylla's bouncing bolt.
pub const BOUNCE: AbilityId = AbilityId(2);
/// Sylla's volley: everything near takes a shot.
pub const VOLLEY: AbilityId = AbilityId(3);
/// Pudge's hook.
pub const MEAT_HOOK: AbilityId = AbilityId(4);
/// Pudge's rot: a toggle that burns everything near, its owner included.
pub const ROT: AbilityId = AbilityId(5);
/// Pudge's flesh heap: what he keeps of everything that dies near him.
pub const FLESH_HEAP: AbilityId = AbilityId(6);
/// Pudge's dismember: a channel that holds one unit and eats it.
pub const DISMEMBER: AbilityId = AbilityId(7);
/// A courier's burst of speed.
pub const BURST: AbilityId = AbilityId(8);
/// A courier putting back what it carries.
pub const RETURN_ITEMS: AbilityId = AbilityId(9);
/// A courier taking what waits in the stash.
pub const TAKE_STASH: AbilityId = AbilityId(10);
/// A courier handing over what it carries.
pub const DELIVER: AbilityId = AbilityId(11);
/// A courier's shield, which nothing gets through.
pub const SHIELD: AbilityId = AbilityId(12);
/// Shadow Fiend's nearest raze.
pub const RAZE_NEAR: AbilityId = AbilityId(13);
/// Shadow Fiend's middle raze.
pub const RAZE_MID: AbilityId = AbilityId(14);
/// Shadow Fiend's farthest raze.
pub const RAZE_FAR: AbilityId = AbilityId(15);
/// Shadow Fiend's requiem, which spends nothing and grows with the souls it
/// has gathered.
pub const REQUIEM: AbilityId = AbilityId(16);

/// Every ability, indexed by [`AbilityId`].
pub const ABILITIES: [AbilityDef; 17] = [
    AbilityDef {
        name: "Crit",
        aim: Aim::Own,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: true,
        ultimate: false,
        at_an_enemy: false,
        mana: &[],
        cooldown: &[],
        range: 0,
    },
    AbilityDef {
        name: "Frenzy",
        aim: Aim::Own,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        ultimate: false,
        at_an_enemy: false,
        mana: &rules::SYLLA_FRENZY_MANA,
        cooldown: &rules::SYLLA_FRENZY_COOLDOWN,
        range: 0,
    },
    AbilityDef {
        name: "Bounce",
        aim: Aim::Unit,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        ultimate: false,
        at_an_enemy: true,
        mana: &rules::SYLLA_BOUNCE_MANA,
        cooldown: &rules::SYLLA_BOUNCE_COOLDOWN,
        range: rules::SYLLA_BOUNCE_CAST_RANGE,
    },
    AbilityDef {
        name: "Volley",
        aim: Aim::Own,
        max_level: rules::ULT_MAX_LEVEL,
        passive: false,
        ultimate: true,
        at_an_enemy: false,
        mana: &rules::SYLLA_MULTI_MANA,
        cooldown: &rules::SYLLA_MULTI_COOLDOWN,
        range: rules::SYLLA_MULTI_RADIUS,
    },
    AbilityDef {
        name: "Meat Hook",
        aim: Aim::Point,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        ultimate: false,
        at_an_enemy: false,
        mana: &rules::HOOK_MANA,
        cooldown: &rules::HOOK_COOLDOWN,
        range: rules::HOOK_RANGE,
    },
    AbilityDef {
        name: "Rot",
        aim: Aim::Own,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        ultimate: false,
        at_an_enemy: false,
        mana: &[0, 0, 0, 0],
        cooldown: &[0, 0, 0, 0],
        range: rules::ROT_RADIUS,
    },
    AbilityDef {
        name: "Flesh Heap",
        aim: Aim::Own,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: true,
        ultimate: false,
        at_an_enemy: false,
        mana: &[],
        cooldown: &[],
        range: rules::FLESH_HEAP_RANGE,
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
    },
    AbilityDef {
        name: "Burst",
        aim: Aim::Own,
        max_level: 1,
        passive: false,
        ultimate: false,
        at_an_enemy: false,
        mana: &[0],
        cooldown: &[rules::COURIER_BURST_COOLDOWN],
        range: 0,
    },
    AbilityDef {
        name: "Return",
        aim: Aim::Own,
        max_level: 1,
        passive: false,
        ultimate: false,
        at_an_enemy: false,
        mana: &[0],
        cooldown: &[0],
        range: 0,
    },
    AbilityDef {
        name: "Take Stash",
        aim: Aim::Own,
        max_level: 1,
        passive: false,
        ultimate: false,
        at_an_enemy: false,
        mana: &[0],
        cooldown: &[0],
        range: 0,
    },
    AbilityDef {
        name: "Deliver",
        aim: Aim::Own,
        max_level: 1,
        passive: false,
        ultimate: false,
        at_an_enemy: false,
        mana: &[0],
        cooldown: &[0],
        range: 0,
    },
    AbilityDef {
        name: "Shield",
        aim: Aim::Own,
        max_level: 1,
        passive: false,
        ultimate: false,
        at_an_enemy: false,
        mana: &[0],
        cooldown: &[rules::COURIER_SHIELD_COOLDOWN],
        range: 0,
    },
    AbilityDef {
        name: "Shadowraze (Near)",
        aim: Aim::Point,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        ultimate: false,
        at_an_enemy: false,
        mana: &rules::RAZE_MANA,
        cooldown: &rules::RAZE_COOLDOWN,
        range: rules::RAZE_DISTANCE[0],
    },
    AbilityDef {
        name: "Shadowraze (Mid)",
        aim: Aim::Point,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        ultimate: false,
        at_an_enemy: false,
        mana: &rules::RAZE_MANA,
        cooldown: &rules::RAZE_COOLDOWN,
        range: rules::RAZE_DISTANCE[1],
    },
    AbilityDef {
        name: "Shadowraze (Far)",
        aim: Aim::Point,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        ultimate: false,
        at_an_enemy: false,
        mana: &rules::RAZE_MANA,
        cooldown: &rules::RAZE_COOLDOWN,
        range: rules::RAZE_DISTANCE[2],
    },
    AbilityDef {
        name: "Requiem of Souls",
        aim: Aim::Own,
        max_level: rules::ULT_MAX_LEVEL,
        passive: false,
        ultimate: true,
        at_an_enemy: false,
        mana: &rules::REQUIEM_MANA,
        cooldown: &rules::REQUIEM_COOLDOWN,
        range: rules::REQUIEM_RADIUS,
    },
];

/// What one ability is, or nothing if no such ability exists.
pub fn ability_def(id: AbilityId) -> Option<&'static AbilityDef> {
    ABILITIES.get(usize::from(id.0))
}

/// What one cast costs at a level.
///
/// Zero for a slot that holds nothing castable.
pub fn ability_mana_cost(id: AbilityId, level: u8) -> i32 {
    let Some(def) = ability_def(id) else {
        return 0;
    };
    pick(def.mana, level)
}

/// What one cast puts on the clock at a level.
pub fn ability_cooldown(id: AbilityId, level: u8) -> u32 {
    let Some(def) = ability_def(id) else {
        return 0;
    };
    pick(def.cooldown, level)
}

/// The entry of a per-level table for a level, the last one standing for
/// everything past it.
fn pick<T: Copy + Default>(table: &[T], level: u8) -> T {
    let level = usize::from(level.max(1) - 1);
    table
        .get(level)
        .copied()
        .unwrap_or_else(|| table.last().copied().unwrap_or_default())
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
