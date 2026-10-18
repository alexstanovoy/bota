//! Pudge's numbers, as of Dota 7.41f.
//!
//! From the game's `scripts/npc/heroes/npc_dota_hero_pudge.txt` (client 6957,
//! 2026-10-09): body, ability values, cast points. Liquipedia's *Pudge* page
//! (read 2026-10-18) for what the file leaves out: the dismember bites six times
//! in its channel, the first as it takes hold, for the whole channel's damage
//! and as much healing.

use bota_proto::Fixed;

/// How fast Pudge turns, in brads per tick: a `MovementTurnRate` of 0.7, read
/// as [`crate::rules::TURN_RATE_BRADS`] reads 0.5.
pub const PUDGE_TURN_RATE_BRADS: u16 = 8113;
/// Mana the hook costs, by level.
pub const HOOK_MANA: [i32; 4] = [120; 4];
/// Ticks between hooks, by level: 18, 16, 14 and 12 seconds.
pub const HOOK_COOLDOWN: [u32; 4] = [540, 480, 420, 360];
/// Milliseconds of the hook's cast point.
pub const HOOK_CAST_POINT_MS: u32 = 300;
/// How far the hook flies, in world units.
pub const HOOK_RANGE: i32 = 1300;
/// How fast the hook flies, in world units a second.
pub const HOOK_SPEED: i32 = 1600;
/// How wide the hook catches, in world units.
pub const HOOK_RADIUS: i32 = 100;
/// Damage the hook deals, by level.
pub const HOOK_DAMAGE: [i32; 4] = [150, 220, 290, 360];
/// How far the rot reaches, in world units.
pub const ROT_RADIUS: i32 = 250;
/// Damage the rot deals a second, by level.
pub const ROT_DAMAGE_PER_SECOND: [i32; 4] = [30, 60, 90, 120];
/// Percent of speed the rot takes, by level.
pub const ROT_SLOW_PCT: [i32; 4] = [14, 20, 26, 32];
/// How near an enemy hero has to fall to feed the flesh heap, in world
/// units.
pub const FLESH_HEAP_RANGE: i32 = 450;
/// Strength one stack of the flesh heap is worth.
pub const FLESH_HEAP_STRENGTH_PER_STACK: Fixed = Fixed::from_int(2);
/// Mana the dismember costs, by level.
pub const DISMEMBER_MANA: [i32; 3] = [100, 130, 170];
/// Ticks between dismembers, by level.
pub const DISMEMBER_COOLDOWN: [u32; 3] = [900, 750, 600];
/// How far the dismember reaches, in world units.
pub const DISMEMBER_RANGE: i32 = 200;
/// Ticks a dismember holds what it caught: its channel of 2.75 seconds,
/// the half tick rounded up.
pub const DISMEMBER_TICKS: u32 = 83;
/// Milliseconds of the dismember's cast point.
pub const DISMEMBER_CAST_POINT_MS: u32 = 300;
/// How many times a dismember bites in its channel, the first one as it
/// takes hold.
pub const DISMEMBER_BITES: u32 = 6;
/// Damage a dismember deals a second before strength, by level.
pub const DISMEMBER_DAMAGE_PER_SECOND: [i32; 3] = [80, 100, 120];
/// Damage a second a point of the caster's strength adds to a dismember, by
/// level.
pub const DISMEMBER_STRENGTH_DAMAGE: [Fixed; 3] = [
    Fixed::from_ratio(3, 10),
    Fixed::from_ratio(6, 10),
    Fixed::from_ratio(9, 10),
];
/// World units a second a dismember pulls what it holds towards its caster.
pub const DISMEMBER_PULL_SPEED: i32 = 75;
/// How near a dismember pulls what it holds, in world units between their
/// centers.
pub const DISMEMBER_PULL_LIMIT: i32 = 125;
/// Links a hook's chain is laid out with between the thrower and the hook.
pub const HOOK_LINKS: usize = 10;
/// Mana the meat shield costs, by level.
pub const MEAT_SHIELD_MANA: [i32; 4] = [65, 70, 75, 80];
/// Ticks between meat shields, by level: 20, 19, 18 and 17 seconds.
pub const MEAT_SHIELD_COOLDOWN: [u32; 4] = [600, 570, 540, 510];
/// Damage the meat shield takes off every blow that reaches its holder, by
/// level.
pub const MEAT_SHIELD_BLOCK: [i32; 4] = [8, 14, 20, 26];
/// Ticks the meat shield holds, by level: 4, 5, 6 and 7 seconds.
pub const MEAT_SHIELD_TICKS: [u32; 4] = [120, 150, 180, 210];
