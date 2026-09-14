//! Every ability a hero may carry: what it is called, how it is aimed, what
//! it asks for, and what it does at each moment of its life.

use bota_proto::{AbilityId, Aim, Target};

use crate::game::{Entity, World, rules};

/// What an ability does at a moment of its life. `false`: it did not happen.
pub type CastHook = fn(&mut World, Entity, Target) -> bool;

/// What an ability does when it ends.
pub type EndHook = fn(&mut World, Entity, Target);

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
    /// At the cast point. `false`: nothing happened and nothing is spent.
    pub on_cast: CastHook,
    /// Every tick it runs on. `false`: it breaks off.
    pub on_during: CastHook,
    /// Broken off by an order, a stun, the caster falling, or its own
    /// `false`.
    pub on_cancel: EndHook,
    /// Run to the end of `duration`.
    pub on_complete: EndHook,
}

/// Nothing at all, so an entry names only what sets it apart.
const PLAIN: AbilityDef = AbilityDef {
    name: "",
    aim: Aim::Own,
    max_level: 0,
    passive: true,
    ultimate: false,
    at_an_enemy: false,
    mana: &[],
    cooldown: &[],
    range: 0,
    point: 0,
    backswing: 0,
    duration: 0,
    on_cast: never,
    on_during: goes_on,
    on_cancel: nothing,
    on_complete: nothing,
};

fn never(_: &mut World, _: Entity, _: Target) -> bool {
    false
}

fn goes_on(_: &mut World, _: Entity, _: Target) -> bool {
    true
}

fn nothing(_: &mut World, _: Entity, _: Target) {}

/// Which level of an ability its caster has learned, counted from zero.
fn level_of(world: &World, caster: Entity, id: AbilityId) -> usize {
    usize::from(world.carried_level(caster, id).max(1) - 1)
}

fn frenzy(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, FRENZY);
    world.cast_frenzy(caster, level)
}

fn bounce(world: &mut World, caster: Entity, target: Target) -> bool {
    let level = level_of(world, caster, BOUNCE);
    world.cast_bounce(caster, level, target)
}

fn volley(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, VOLLEY).min(2);
    world.cast_multishot(caster, level)
}

fn meat_hook(world: &mut World, caster: Entity, target: Target) -> bool {
    let level = level_of(world, caster, MEAT_HOOK);
    world.cast_hook(caster, level, target)
}

fn rot(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, ROT);
    world.toggle_rot(caster, level)
}

fn dismember(world: &mut World, caster: Entity, target: Target) -> bool {
    world.dismember_takes_hold(caster, target)
}

fn dismember_holds(world: &mut World, caster: Entity, target: Target) -> bool {
    world.dismember_holds(caster, target)
}

fn dismember_lets_go(world: &mut World, caster: Entity, target: Target) {
    world.dismember_lets_go(caster, target);
}

fn burst(world: &mut World, courier: Entity, _: Target) -> bool {
    world.courier_burst(courier)
}

fn return_items(world: &mut World, courier: Entity, _: Target) -> bool {
    world.courier_return_items(courier)
}

fn take_stash(world: &mut World, courier: Entity, _: Target) -> bool {
    world.courier_take_stash(courier)
}

fn deliver(world: &mut World, courier: Entity, _: Target) -> bool {
    world.courier_deliver(courier)
}

fn shield(world: &mut World, courier: Entity, _: Target) -> bool {
    world.courier_shield(courier)
}

fn raze_near(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, RAZE_NEAR);
    world.cast_raze(caster, level, 0)
}

fn raze_mid(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, RAZE_MID);
    world.cast_raze(caster, level, 1)
}

fn raze_far(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, RAZE_FAR);
    world.cast_raze(caster, level, 2)
}

fn requiem(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, REQUIEM).min(2);
    world.cast_requiem(caster, level)
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
/// Pudge's dismember: holds one unit and eats it.
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
/// Shadow Fiend's necromastery: a soul kept of everything he brings down.
pub const NECROMASTERY: AbilityId = AbilityId(17);
/// Shadow Fiend's presence: enemies standing near him wear less armor.
pub const PRESENCE: AbilityId = AbilityId(18);

/// Every ability, indexed by [`AbilityId`].
pub const ABILITIES: [AbilityDef; 19] = [
    AbilityDef {
        name: "Crit",
        max_level: rules::ABILITY_MAX_LEVEL,
        ..PLAIN
    },
    AbilityDef {
        name: "Frenzy",
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &rules::SYLLA_FRENZY_MANA,
        cooldown: &rules::SYLLA_FRENZY_COOLDOWN,
        on_cast: frenzy,
        ..PLAIN
    },
    AbilityDef {
        name: "Bounce",
        aim: Aim::Unit,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        at_an_enemy: true,
        mana: &rules::SYLLA_BOUNCE_MANA,
        cooldown: &rules::SYLLA_BOUNCE_COOLDOWN,
        range: rules::SYLLA_BOUNCE_CAST_RANGE,
        on_cast: bounce,
        ..PLAIN
    },
    AbilityDef {
        name: "Volley",
        max_level: rules::ULT_MAX_LEVEL,
        passive: false,
        ultimate: true,
        mana: &rules::SYLLA_MULTI_MANA,
        cooldown: &rules::SYLLA_MULTI_COOLDOWN,
        range: rules::SYLLA_MULTI_RADIUS,
        on_cast: volley,
        ..PLAIN
    },
    AbilityDef {
        name: "Meat Hook",
        aim: Aim::Point,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &rules::HOOK_MANA,
        cooldown: &rules::HOOK_COOLDOWN,
        range: rules::HOOK_RANGE,
        on_cast: meat_hook,
        ..PLAIN
    },
    AbilityDef {
        name: "Rot",
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &[0, 0, 0, 0],
        cooldown: &[0, 0, 0, 0],
        range: rules::ROT_RADIUS,
        on_cast: rot,
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
        on_cast: dismember,
        on_during: dismember_holds,
        on_cancel: dismember_lets_go,
        on_complete: dismember_lets_go,
        ..PLAIN
    },
    AbilityDef {
        name: "Burst",
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown: &[rules::COURIER_BURST_COOLDOWN],
        on_cast: burst,
        ..PLAIN
    },
    AbilityDef {
        name: "Return",
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown: &[0],
        on_cast: return_items,
        ..PLAIN
    },
    AbilityDef {
        name: "Take Stash",
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown: &[0],
        on_cast: take_stash,
        ..PLAIN
    },
    AbilityDef {
        name: "Deliver",
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown: &[0],
        on_cast: deliver,
        ..PLAIN
    },
    AbilityDef {
        name: "Shield",
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown: &[rules::COURIER_SHIELD_COOLDOWN],
        on_cast: shield,
        ..PLAIN
    },
    AbilityDef {
        name: "Shadowraze (Near)",
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &rules::RAZE_MANA,
        cooldown: &rules::RAZE_COOLDOWN,
        range: rules::RAZE_DISTANCE[0],
        on_cast: raze_near,
        ..PLAIN
    },
    AbilityDef {
        name: "Shadowraze (Mid)",
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &rules::RAZE_MANA,
        cooldown: &rules::RAZE_COOLDOWN,
        range: rules::RAZE_DISTANCE[1],
        on_cast: raze_mid,
        ..PLAIN
    },
    AbilityDef {
        name: "Shadowraze (Far)",
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &rules::RAZE_MANA,
        cooldown: &rules::RAZE_COOLDOWN,
        range: rules::RAZE_DISTANCE[2],
        on_cast: raze_far,
        ..PLAIN
    },
    AbilityDef {
        name: "Requiem of Souls",
        max_level: rules::ULT_MAX_LEVEL,
        passive: false,
        ultimate: true,
        mana: &rules::REQUIEM_MANA,
        cooldown: &rules::REQUIEM_COOLDOWN,
        range: rules::REQUIEM_RADIUS,
        on_cast: requiem,
        ..PLAIN
    },
    AbilityDef {
        name: "Necromastery",
        max_level: rules::ABILITY_MAX_LEVEL,
        ..PLAIN
    },
    AbilityDef {
        name: "Presence of the Dark Lord",
        max_level: rules::ABILITY_MAX_LEVEL,
        range: rules::PRESENCE_RADIUS,
        ..PLAIN
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

/// The ability whose level a whole group of slots stands at.
///
/// The three razes are one skill worn as three slots: a point into any of
/// them levels all three, and the group costs points as one ability. Every
/// other ability is a group of itself.
pub fn learn_group(id: AbilityId) -> AbilityId {
    match id {
        RAZE_MID | RAZE_FAR => RAZE_NEAR,
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
