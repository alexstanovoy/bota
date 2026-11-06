//! Pudge's numbers, as of Dota 7.41f: read from the install's
//! `scripts/npc/heroes/npc_dota_hero_pudge.txt` through the generated `heroes_data.rs`, by
//! Dota's keys. A number written here is bota's own and says why.

use bota_proto::Fixed;

use crate::heroes_data::{
    NPC_DOTA_HERO_PUDGE, PUDGE_DISMEMBER, PUDGE_FLESH_HEAP, PUDGE_INNATE_GRAFT_FLESH,
    PUDGE_MEAT_HOOK, PUDGE_ROT,
};
use crate::{fixed, ints_of, ticks_of, turn_rate_brads};

/// How fast Pudge turns, in brads per tick: his `MovementTurnRate`.
pub const PUDGE_TURN_RATE_BRADS: u16 = turn_rate_brads(NPC_DOTA_HERO_PUDGE.turn_rate);
/// Mana the hook costs, by level.
pub const HOOK_MANA: [i32; 4] = ints_of(PUDGE_MEAT_HOOK.mana);
/// Ticks between hooks, by level.
pub const HOOK_COOLDOWN: [u32; 4] = ticks_of(PUDGE_MEAT_HOOK.cooldown);
/// Milliseconds of the hook's cast point.
pub const HOOK_CAST_POINT_MS: u32 = PUDGE_MEAT_HOOK.cast_point[0].0;
/// How far the hook flies, in world units: its cast range.
pub const HOOK_RANGE: i32 = PUDGE_MEAT_HOOK.cast_range[0];
/// How fast the hook flies, in world units a second.
pub const HOOK_SPEED: i32 = PUDGE_MEAT_HOOK.value("hook_speed").first().int();
/// How wide the hook catches, in world units.
pub const HOOK_RADIUS: i32 = PUDGE_MEAT_HOOK.value("hook_width").first().int();
/// Damage the hook deals, by level.
pub const HOOK_DAMAGE: [i32; 4] = PUDGE_MEAT_HOOK.value("damage").ints();
/// How far the rot reaches, in world units.
pub const ROT_RADIUS: i32 = PUDGE_ROT.value("rot_radius").first().int();
/// Damage the rot deals a second, by level.
pub const ROT_DAMAGE_PER_SECOND: [i32; 4] = PUDGE_ROT.value("rot_damage").ints();
/// Percent of speed the rot takes, by level: the script's `rot_slow`, which it writes as a
/// change of speed.
pub const ROT_SLOW_PCT: [i32; 4] = {
    let change: [i32; 4] = PUDGE_ROT.value("rot_slow").ints();
    [-change[0], -change[1], -change[2], -change[3]]
};
/// How near an enemy hero has to fall to feed the flesh heap, in world
/// units.
pub const FLESH_HEAP_RANGE: i32 = PUDGE_INNATE_GRAFT_FLESH
    .value("flesh_heap_range")
    .first()
    .int();
/// Strength one stack of the flesh heap is worth.
pub const FLESH_HEAP_STRENGTH_PER_STACK: Fixed = fixed(
    PUDGE_INNATE_GRAFT_FLESH
        .value("flesh_heap_strength_buff_amount")
        .first(),
);
/// Mana the dismember costs, by level.
pub const DISMEMBER_MANA: [i32; 3] = ints_of(PUDGE_DISMEMBER.mana);
/// Ticks between dismembers, by level.
pub const DISMEMBER_COOLDOWN: [u32; 3] = ticks_of(PUDGE_DISMEMBER.cooldown);
/// How far the dismember reaches, in world units.
pub const DISMEMBER_RANGE: i32 = PUDGE_DISMEMBER.cast_range[0];
/// Ticks a dismember holds what it caught: its channel, the half tick
/// rounded up.
pub const DISMEMBER_TICKS: u32 = PUDGE_DISMEMBER.channel[0].ticks();
/// Milliseconds of the dismember's cast point.
pub const DISMEMBER_CAST_POINT_MS: u32 = PUDGE_DISMEMBER.cast_point[0].0;
/// How many times a dismember bites in its channel, the first one as it
/// takes hold.
pub const DISMEMBER_BITES: u32 = PUDGE_DISMEMBER.value("ticks").first().int() as u32;
/// Damage a dismember deals a second before strength, by level.
pub const DISMEMBER_DAMAGE_PER_SECOND: [i32; 3] = PUDGE_DISMEMBER.value("dismember_damage").ints();
/// Damage a second a point of the caster's strength adds to a dismember, by
/// level.
pub const DISMEMBER_STRENGTH_DAMAGE: [Fixed; 3] = PUDGE_DISMEMBER.value("strength_damage").fixeds();
/// World units a second a dismember pulls what it holds towards its caster.
pub const DISMEMBER_PULL_SPEED: i32 = PUDGE_DISMEMBER.value("pull_units_per_second").first().int();
/// How near a dismember pulls what it holds, in world units between their
/// centers.
pub const DISMEMBER_PULL_LIMIT: i32 = PUDGE_DISMEMBER.value("pull_distance_limit").first().int();
/// Links a hook's chain is laid out with between the thrower and the hook:
/// bota's own, for the chain shown.
pub const HOOK_LINKS: usize = 10;
/// Mana the meat shield costs, by level.
pub const MEAT_SHIELD_MANA: [i32; 4] = ints_of(PUDGE_FLESH_HEAP.mana);
/// Ticks between meat shields, by level.
pub const MEAT_SHIELD_COOLDOWN: [u32; 4] = ticks_of(PUDGE_FLESH_HEAP.cooldown);
/// Damage the meat shield takes off every blow that reaches its holder, by
/// level.
pub const MEAT_SHIELD_BLOCK: [i32; 4] = PUDGE_FLESH_HEAP.value("damage_block").ints();
/// Ticks the meat shield holds, by level.
pub const MEAT_SHIELD_TICKS: [u32; 4] = PUDGE_FLESH_HEAP.value("duration").ticks();
/// Ticks a hook's followthrough holds its thrower: the hook's reach over its speed, times its
/// `self_stun_duration_percent` (Liquipedia, *Pudge*: "MaxTravelDistance / HookSpeed × 0.65").
pub const HOOK_FOLLOWTHROUGH_TICKS: u32 = {
    let percent = PUDGE_MEAT_HOOK
        .value("self_stun_duration_percent")
        .first()
        .int();
    crate::Ms((HOOK_RANGE * percent * 10 / HOOK_SPEED) as u32).ticks()
};
