//! Shadow Fiend's numbers, as of Dota 7.41f.
//!
//! From the official hero data (dota2.com datafeed, hero 11: base stats,
//! special values, talents), the game's
//! `scripts/npc/heroes/npc_dota_hero_nevermore.txt` of client 6933
//! (2026-09-15: cast points, behaviours, the death requiem's share, the
//! Necromastery curve), the official patch notes 7.39 to 7.41f, and
//! Liquipedia's *Shadow Fiend* page for what those leave out (backswings, the
//! requiem's first line, the feast's gathering). Where they disagree the live
//! data wins.

use bota_proto::Fixed;

use crate::Ratio;
use crate::rules::TICKS_PER_SECOND;

/// How far in front of the caster a raze lands, in world units, from the
/// nearest reach to the farthest.
pub const RAZE_DISTANCE: [i32; 3] = [200, 450, 700];
/// How wide a raze burns, in world units.
pub const RAZE_RADIUS: i32 = 250;
/// Damage a raze deals before the souls, by level.
pub const RAZE_DAMAGE: [i32; 4] = [85, 150, 215, 280];
/// Damage a raze adds for every soul its caster holds when it goes off.
pub const RAZE_DAMAGE_PER_SOUL: i32 = 2;
/// Additional magical damage per prior same-caster stack, by cast level.
pub const RAZE_STACK_DAMAGE: [i32; 4] = [35, 50, 65, 80];
/// Ticks until all of one caster's stacks expire after its latest damaging hit.
pub const RAZE_DEBUFF_TICKS: u32 = 6 * TICKS_PER_SECOND;
/// Maximum stacks held per target and caster.
pub const RAZE_MAX_STACKS: u8 = 255;
/// Maximum independent caster records on one target; earliest expiry is evicted first.
pub const RAZE_MAX_SOURCES: usize = 16;
/// Milliseconds of a raze's cast point. A raze has no backswing.
pub const RAZE_CAST_POINT_MS: u32 = 550;

const _: () = {
    assert!(RAZE_STACK_DAMAGE.len() == RAZE_DAMAGE.len());
    assert!(RAZE_DEBUFF_TICKS == 180);
    assert!(RAZE_MAX_STACKS == u8::MAX);
    assert!(
        RAZE_DAMAGE[3]
            + RAZE_DAMAGE_PER_SOUL * SOULS_HELD_MAX as i32
            + (RAZE_STACK_DAMAGE[3] + TALENT_RAZE_STACK_DAMAGE) * (RAZE_MAX_STACKS as i32)
            < 32768
    );
};

/// Mana a raze costs, by level.
pub const RAZE_MANA: [i32; 4] = [75; 4];
/// Ticks between two razes of the same reach, by level. Each reach keeps its
/// own.
pub const RAZE_COOLDOWN: [u32; 4] = [9 * TICKS_PER_SECOND; 4];
/// Souls one unit brought down is worth to whoever gathers them.
pub const SOULS_PER_UNIT: u32 = 1;
/// Souls one hero brought down is worth to whoever gathers them.
pub const SOULS_PER_HERO: u32 = 4;
/// Souls the necromastery holds from kills.
pub const NECRO_SOUL_CAP: u32 = 20;
/// Tenths of attack damage one soul is worth at hero levels 1 to 5.
pub const SOUL_DAMAGE_TENTHS: i32 = 10;
/// Tenths of attack damage one soul gains every [`SOUL_DAMAGE_LEVELS`]
/// hero levels.
pub const SOUL_DAMAGE_TENTHS_GAIN: i32 = 8;
/// Hero levels between two gains of what a soul is worth.
pub const SOUL_DAMAGE_LEVELS: i32 = 6;
/// Percent of the souls held that a death lets go, rounded down.
pub const SOULS_LOST_ON_DEATH_PCT: u32 = 30;
/// How far the presence reaches, in world units.
pub const PRESENCE_RADIUS: i32 = 1200;
/// Armor the presence takes from an enemy standing in it, by level.
pub const PRESENCE_ARMOR: [Fixed; 4] = [
    Fixed::from_ratio(5, 2),
    Fixed::from_int(4),
    Fixed::from_ratio(11, 2),
    Fixed::from_int(7),
];
/// Ticks the presence lingers on an enemy that walks out of it.
pub const PRESENCE_LINGER_TICKS: u32 = TICKS_PER_SECOND / 2;
/// How far each line of the requiem flies, in world units.
pub const REQUIEM_LINE_DISTANCE: i32 = 1000;
/// World units a second a line of the requiem flies.
pub const REQUIEM_LINE_SPEED: i32 = 700;
/// How wide a line of the requiem catches as it sets out, in world units.
pub const REQUIEM_LINE_WIDTH_START: i32 = 125;
/// How wide a line of the requiem catches at the end of its flight.
pub const REQUIEM_LINE_WIDTH_END: i32 = 300;
/// The most lines one requiem lets go.
pub const REQUIEM_LINES_MAX: u32 = 20;
/// Damage each line of the requiem lands on each it crosses, by level.
pub const REQUIEM_LINE_DAMAGE: [i32; 3] = [80, 120, 160];
/// Percent of speed the requiem takes from what it catches, by level.
pub const REQUIEM_SLOW_PCT: [i32; 3] = [20, 25, 30];
/// Percent of the magic it keeps out that the requiem takes from what it
/// catches, by level.
pub const REQUIEM_MAGIC_BREAK_PCT: [i32; 3] = [5, 10, 15];
/// Ticks the requiem's fear, slow and torn resistance hold for each line
/// that crosses what it catches: 0.6 seconds.
pub const REQUIEM_LINE_TICKS: u32 = 18;
/// The most ticks the requiem's fear, slow and torn resistance hold: 2.15
/// seconds, the half tick rounded up.
pub const REQUIEM_HOLD_MAX_TICKS: u32 = 65;
/// Milliseconds of the requiem's cast point.
pub const REQUIEM_CAST_POINT_MS: u32 = 1670;
/// Milliseconds of the requiem's backswing.
pub const REQUIEM_BACKSWING_MS: u32 = 900;
/// Share of the souls kept through a death that the requiem cast on that
/// death lets fly as lines, rounded down.
pub const REQUIEM_DEATH_LINES: Ratio = Ratio::new(1, 2);
/// Mana the requiem costs, by level.
pub const REQUIEM_MANA: [i32; 3] = [150, 175, 200];
/// Ticks between requiems, by level.
pub const REQUIEM_COOLDOWN: [u32; 3] = [
    120 * TICKS_PER_SECOND,
    110 * TICKS_PER_SECOND,
    100 * TICKS_PER_SECOND,
];
/// Attack speed the feast of souls adds, by level.
pub const FEAST_ATTACK_SPEED: [i32; 4] = [35, 50, 65, 80];
/// Percent the feast of souls adds to movement speed, by level.
pub const FEAST_MOVE_PCT: [i32; 4] = [4, 6, 8, 10];
/// Ticks the feast of souls runs.
pub const FEAST_TICKS: u32 = 8 * TICKS_PER_SECOND;
/// Ticks between two gatherings of a feast, the first one that long after
/// the cast.
pub const FEAST_PERIOD_TICKS: u32 = TICKS_PER_SECOND / 2;
/// Enemies one gathering of a feast takes souls from at most.
pub const FEAST_PER_PERIOD: usize = 2;
/// How far from its caster a feast takes souls, in world units.
pub const FEAST_RADIUS: i32 = 600;
/// Souls a feast takes from an enemy hero.
pub const FEAST_SOULS_PER_HERO: u32 = 3;
/// Souls a feast takes from any other enemy.
pub const FEAST_SOULS_PER_UNIT: u32 = 1;
/// Enemies one feast takes souls from in all, by level.
pub const FEAST_TARGETS: [usize; 4] = [4, 6, 8, 10];
/// Ticks the souls of the enemies that died during a feast are kept past
/// the cap once the feast is over.
pub const FEAST_LINGER_TICKS: u32 = 8 * TICKS_PER_SECOND;
/// Mana the feast of souls costs, by level.
pub const FEAST_MANA: [i32; 4] = [60, 65, 70, 75];
/// Ticks between two feasts.
pub const FEAST_COOLDOWN: [u32; 4] = [21 * TICKS_PER_SECOND; 4];
/// Attack speed the level-10 talent adds to a feast.
pub const TALENT_FEAST_ATTACK_SPEED: i32 = 30;
/// Raze damage a stack the level-10 talent adds.
pub const TALENT_RAZE_STACK_DAMAGE: i32 = 30;
/// Souls a feast takes from a hero on top, with the level-15 talent.
pub const TALENT_FEAST_SOULS_PER_HERO: u32 = 2;
/// Armor the presence takes on top, with the level-15 talent: 1.5 since
/// 7.41f.
pub const TALENT_PRESENCE_ARMOR: Fixed = Fixed::from_ratio(3, 2);
/// Ticks of fear, slow and torn resistance a requiem line adds on top, with
/// the level-20 talent: 0.2 seconds.
pub const TALENT_REQUIEM_LINE_TICKS: u32 = 6;
/// Ticks the requiem's hold may reach on top, with the level-20 talent: the
/// 0.45 seconds that bring 2.15 to 2.6.
pub const TALENT_REQUIEM_HOLD_TICKS: u32 = 13;
/// Souls the necromastery holds on top, with the level-20 talent.
pub const TALENT_NECRO_SOUL_CAP: u32 = 5;
/// Percent of its length a cast point lasts while a feast runs, with the
/// level-25 talent.
pub const TALENT_FEAST_CAST_POINT_PCT: u32 = 70;
/// Souls a Shadow Fiend can hold at once: the cap and every soul the
/// largest feast can take on top of it, talents included.
pub const SOULS_HELD_MAX: u32 = NECRO_SOUL_CAP
    + TALENT_NECRO_SOUL_CAP
    + FEAST_TARGETS[3] as u32 * (FEAST_SOULS_PER_HERO + TALENT_FEAST_SOULS_PER_HERO);

const _: () = {
    assert!(FEAST_TICKS.is_multiple_of(FEAST_PERIOD_TICKS));
    assert!(FEAST_PER_PERIOD > 0);
    assert!(FEAST_TARGETS[3] >= FEAST_TARGETS[0]);
    assert!(REQUIEM_HOLD_MAX_TICKS >= REQUIEM_LINE_TICKS);
    assert!(REQUIEM_LINE_TICKS + TALENT_REQUIEM_LINE_TICKS == 24);
    assert!(REQUIEM_HOLD_MAX_TICKS + TALENT_REQUIEM_HOLD_TICKS == 78);
    assert!(TALENT_FEAST_CAST_POINT_PCT > 0);
    assert!(TALENT_FEAST_CAST_POINT_PCT <= 100);
};
