//! Pudge's numbers.

use bota_proto::Fixed;

/// Mana the hook costs, by level.
pub const HOOK_MANA: [i32; 4] = [110, 120, 130, 140];
/// Ticks between hooks, by level.
pub const HOOK_COOLDOWN: [u32; 4] = [810, 690, 570, 450];
/// How far the hook flies, in world units.
pub const HOOK_RANGE: i32 = 1300;
/// How fast the hook flies, in world units a second.
pub const HOOK_SPEED: i32 = 1600;
/// How wide the hook catches, in world units.
pub const HOOK_RADIUS: i32 = 100;
/// Damage the hook deals, by level.
pub const HOOK_DAMAGE: [i32; 4] = [90, 180, 270, 360];
/// How far the rot reaches, in world units.
pub const ROT_RADIUS: i32 = 250;
/// Damage the rot deals a second, by level.
pub const ROT_DAMAGE_PER_SECOND: [i32; 4] = [30, 60, 90, 120];
/// Percent of speed the rot takes, by level.
pub const ROT_SLOW_PCT: [i32; 4] = [10, 15, 20, 25];
/// How near an enemy hero has to fall to feed the flesh heap, in world
/// units.
pub const FLESH_HEAP_RANGE: i32 = 450;
/// Strength one stack of the flesh heap is worth, by the heap's level.
pub const FLESH_HEAP_STRENGTH_PER_STACK: [Fixed; 4] = [
    Fixed::from_ratio(3, 2),
    Fixed::from_int(2),
    Fixed::from_ratio(5, 2),
    Fixed::from_int(3),
];
/// Magic resistance the flesh heap grants, percent by its level, stacking
/// multiplicatively with what its holder has.
pub const FLESH_HEAP_MAGIC_RESIST_PCT: [i32; 4] = [12, 14, 16, 18];
/// Mana the dismember costs, by level.
pub const DISMEMBER_MANA: [i32; 3] = [100, 130, 170];
/// Ticks between dismembers, by level.
pub const DISMEMBER_COOLDOWN: [u32; 3] = [900, 750, 600];
/// How far the dismember reaches, in world units.
pub const DISMEMBER_RANGE: i32 = 200;
/// Ticks a dismember holds what it caught.
pub const DISMEMBER_TICKS: u32 = 90;
/// Damage the dismember deals a second, by level.
pub const DISMEMBER_DAMAGE_PER_SECOND: [i32; 3] = [75, 125, 175];
/// Links a hook's chain is laid out with between the thrower and the hook.
pub const HOOK_LINKS: usize = 10;
