//! Shadow Fiend's numbers: read from the install's `scripts/npc/heroes/npc_dota_hero_nevermore.txt`
//! through the generated `heroes_data.rs`, by Dota's keys. A number written here is bota's own
//! and says why: what the script does not hold (backswings, Liquipedia's *Shadow Fiend*) and
//! the bounds of bota's own bookkeeping.

use bota_proto::Fixed;

use crate::heroes_data::{
    NEVERMORE_DARK_LORD, NEVERMORE_FRENZY, NEVERMORE_NECROMASTERY, NEVERMORE_REQUIEM,
    NEVERMORE_SHADOWRAZE1, NEVERMORE_SHADOWRAZE2, NEVERMORE_SHADOWRAZE3,
};
use crate::rules::TICKS_PER_SECOND;
use crate::{Bonus, Dec, HERO_LEVELUP, fixed, ints_of, tenths, ticks_of};

/// How far in front of the caster a raze lands, in world units, from the
/// nearest reach to the farthest: each raze's `shadowraze_range`.
pub const RAZE_DISTANCE: [i32; 3] = [
    NEVERMORE_SHADOWRAZE1
        .value("shadowraze_range")
        .first()
        .int(),
    NEVERMORE_SHADOWRAZE2
        .value("shadowraze_range")
        .first()
        .int(),
    NEVERMORE_SHADOWRAZE3
        .value("shadowraze_range")
        .first()
        .int(),
];
/// How wide a raze burns, in world units.
pub const RAZE_RADIUS: i32 = NEVERMORE_SHADOWRAZE1
    .value("shadowraze_radius")
    .first()
    .int();
/// Damage a raze deals before the souls, by level.
pub const RAZE_DAMAGE: [i32; 4] = NEVERMORE_SHADOWRAZE1.value("shadowraze_damage").ints::<4>();
/// Damage a raze adds for every soul its caster holds when it goes off.
pub const RAZE_DAMAGE_PER_SOUL: i32 = NEVERMORE_SHADOWRAZE1.value("damage_per_soul").first().int();
/// Additional magical damage per prior same-caster stack, by cast level.
pub const RAZE_STACK_DAMAGE: [i32; 4] = NEVERMORE_SHADOWRAZE1
    .value("stack_bonus_damage")
    .ints::<4>();
/// The base raze slows nobody: `movement_speed_debuff` is `0 0 0 0` and only
/// Aghanim's Shard (not modelled; the Shadowmire facet is deprecated) makes it
/// 12% a stack.
/// Ticks until all of one caster's stacks expire after its latest damaging hit: `duration`.
pub const RAZE_DEBUFF_TICKS: u32 = NEVERMORE_SHADOWRAZE1.value("duration").ticks::<1>()[0];
/// Maximum stacks held per target and caster: bota's bound of the stack count, which the
/// script leaves unbounded.
pub const RAZE_MAX_STACKS: u8 = 255;
/// Maximum independent caster records on one target; earliest expiry is evicted first: bota's
/// bound.
pub const RAZE_MAX_SOURCES: usize = 16;
/// Milliseconds of a raze's cast point. A raze has no backswing.
pub const RAZE_CAST_POINT_MS: u32 = NEVERMORE_SHADOWRAZE1.cast_point[0].0;

/// The three razes are one skill: each script links the next, round to the first.
const _: () = {
    assert!(crate::item_data::str_eq(
        option_key(NEVERMORE_SHADOWRAZE1.linked),
        NEVERMORE_SHADOWRAZE2.key
    ));
    assert!(crate::item_data::str_eq(
        option_key(NEVERMORE_SHADOWRAZE2.linked),
        NEVERMORE_SHADOWRAZE3.key
    ));
    assert!(crate::item_data::str_eq(
        option_key(NEVERMORE_SHADOWRAZE3.linked),
        NEVERMORE_SHADOWRAZE1.key
    ));
};

/// The key a `LinkedAbility` names; none fails the build.
const fn option_key(linked: Option<&'static str>) -> &'static str {
    match linked {
        Some(key) => key,
        None => panic!("a raze links no other"),
    }
}

const _: () = {
    assert!(RAZE_STACK_DAMAGE.len() == RAZE_DAMAGE.len());
    assert!(RAZE_MAX_STACKS == u8::MAX);
    assert!(
        RAZE_DAMAGE[3]
            + RAZE_DAMAGE_PER_SOUL * SOULS_HELD_MAX as i32
            + (RAZE_STACK_DAMAGE[3] + TALENT_RAZE_STACK_DAMAGE) * (RAZE_MAX_STACKS as i32)
            < 32768
    );
};

/// Mana a raze costs, by level.
pub const RAZE_MANA: [i32; 4] = ints_of(NEVERMORE_SHADOWRAZE1.mana);
/// Ticks between two razes of the same reach, by level. Each reach keeps its
/// own.
pub const RAZE_COOLDOWN: [u32; 4] = ticks_of(NEVERMORE_SHADOWRAZE1.cooldown);
/// Souls one unit brought down is worth to whoever gathers them: `souls_per_kill`.
pub const SOULS_PER_UNIT: u32 = NEVERMORE_NECROMASTERY.value("souls_per_kill").first().int() as u32;
/// Souls one hero brought down is worth to whoever gathers them: `souls_per_hero_kill`.
pub const SOULS_PER_HERO: u32 = NEVERMORE_NECROMASTERY
    .value("souls_per_hero_kill")
    .first()
    .int() as u32;
/// Souls the necromastery holds from kills: `necromastery_max_souls`.
pub const NECRO_SOUL_CAP: u32 = NEVERMORE_NECROMASTERY
    .value("necromastery_max_souls")
    .first()
    .int() as u32;
/// Tenths of attack damage one soul is worth at hero levels 1 to 5:
/// `necromastery_damage_per_soul`.
pub const SOUL_DAMAGE_TENTHS: i32 = tenths(
    NEVERMORE_NECROMASTERY
        .value("necromastery_damage_per_soul")
        .first(),
);
/// Tenths of attack damage one soul gains every [`SOUL_DAMAGE_LEVELS`]
/// hero levels, and that interval: the value's `hero_levelup` and `levelup_interval`.
const SOUL_DAMAGE_GROWTH: (Dec, u8) = match NEVERMORE_NECROMASTERY
    .value("necromastery_damage_per_soul")
    .bonus(HERO_LEVELUP)
{
    Bonus::HeroLevels { add, every } => (add, every),
    _ => panic!("the soul's worth grows with the hero's level"),
};
pub const SOUL_DAMAGE_TENTHS_GAIN: i32 = tenths(SOUL_DAMAGE_GROWTH.0);
/// Hero levels between two gains of what a soul is worth.
pub const SOUL_DAMAGE_LEVELS: i32 = SOUL_DAMAGE_GROWTH.1 as i32;
/// Percent of the souls held that a death lets go, rounded down: what
/// `necromastery_soul_release` (the share kept) leaves.
pub const SOULS_LOST_ON_DEATH_PCT: u32 = (100
    - NEVERMORE_NECROMASTERY
        .value("necromastery_soul_release")
        .first()
        .hundredths()) as u32;
/// How far the presence reaches, in world units: `presence_radius`.
pub const PRESENCE_RADIUS: i32 = NEVERMORE_DARK_LORD.value("presence_radius").first().int();
/// Magic resistance, in percent, the presence takes from an enemy standing in it, by level:
/// `presence_magic_resistance_reduction` (7.42).
pub const PRESENCE_MAGIC_PCT: [i32; 4] = negated(
    NEVERMORE_DARK_LORD
        .value("presence_magic_resistance_reduction")
        .ints::<4>(),
);
/// Armor the presence takes from an enemy standing in it, by level: `presence_armor_reduction`.
pub const PRESENCE_ARMOR: [Fixed; 4] = {
    let reduction = NEVERMORE_DARK_LORD.value("presence_armor_reduction");
    let mut out = [Fixed::ZERO; 4];
    let mut level = 0;
    while level < 4 {
        out[level] = fixed(Dec(-reduction.at(level).0));
        level += 1;
    }
    out
};
/// Ticks the presence lingers on an enemy that walks out of it: Dota's half second for an aura,
/// which the script does not hold.
pub const PRESENCE_LINGER_TICKS: u32 = TICKS_PER_SECOND / 2;
/// How far each line of the requiem flies, in world units: `requiem_radius`.
pub const REQUIEM_LINE_DISTANCE: i32 = NEVERMORE_REQUIEM.value("requiem_radius").first().int();
/// World units a second a line of the requiem flies.
pub const REQUIEM_LINE_SPEED: i32 = NEVERMORE_REQUIEM.value("requiem_line_speed").first().int();
/// Soul speed when the caster has a Scepter upgrade.
pub const REQUIEM_SCEPTER_LINE_SPEED: i32 = {
    let speed = NEVERMORE_REQUIEM.value("requiem_line_speed");
    speed
        .bonus("special_bonus_scepter")
        .applied(speed.first(), 0)
        .int()
};
/// How wide a line of the requiem catches as it sets out, in world units.
pub const REQUIEM_LINE_WIDTH_START: i32 = NEVERMORE_REQUIEM
    .value("requiem_line_width_start")
    .first()
    .int();
/// How wide a line of the requiem catches at the end of its flight.
pub const REQUIEM_LINE_WIDTH_END: i32 = NEVERMORE_REQUIEM
    .value("requiem_line_width_end")
    .first()
    .int();
/// The most lines one requiem lets go: `max_soul_release`.
pub const REQUIEM_LINES_MAX: u32 = NEVERMORE_REQUIEM.value("max_soul_release").first().int() as u32;
/// Damage each line of the requiem lands on each it crosses, by level: its `AbilityDamage`.
pub const REQUIEM_LINE_DAMAGE: [i32; 3] = NEVERMORE_REQUIEM.value("AbilityDamage").ints::<3>();
/// Percent of speed the requiem takes from what it catches, by level: `requiem_reduction_ms`.
pub const REQUIEM_SLOW_PCT: [i32; 3] =
    negated(NEVERMORE_REQUIEM.value("requiem_reduction_ms").ints::<3>());
/// Percent of the magic it keeps out that the requiem takes from what it catches, by level:
/// none since 7.42 ("No longer reduces enemy Magic Resistance by 5/10/15%";
/// `requiem_reduction_mres` is gone from the script). The torn resistance stays in the rules, to
/// be offered again.
pub const REQUIEM_MAGIC_BREAK_PCT: [i32; 3] = [0; 3];
/// Ticks the requiem's fear and slow hold on what a first line crosses, by level:
/// `requiem_fear_duration` (and `requiem_slow_duration`, the same), 1.2/1.5/1.8 seconds.
pub const REQUIEM_HOLD_TICKS: [u32; 3] = NEVERMORE_REQUIEM
    .value("requiem_fear_duration")
    .ticks::<3>();
/// Ticks the hold grows by for each line that crosses: `requiem_duration_per_soul`, 0.1
/// seconds (a line is a soul). The hold has no most since 7.42; it is bounded by the lines.
pub const REQUIEM_TICKS_PER_LINE: u32 = NEVERMORE_REQUIEM
    .value("requiem_duration_per_soul")
    .ticks::<1>()[0];
/// Milliseconds of the requiem's cast point.
pub const REQUIEM_CAST_POINT_MS: u32 = NEVERMORE_REQUIEM.cast_point[0].0;
/// Milliseconds of the requiem's backswing: the script holds none; Liquipedia's
/// *Shadow Fiend* gives 0.9 seconds.
pub const REQUIEM_BACKSWING_MS: u32 = 900;
/// Percent of the souls kept through a death that the requiem cast on that
/// death lets fly as lines, rounded down: `soul_death_release`.
pub const REQUIEM_DEATH_LINES_PCT: u32 = NEVERMORE_REQUIEM
    .value("soul_death_release")
    .first()
    .hundredths() as u32;
/// Mana the requiem costs, by level.
pub const REQUIEM_MANA: [i32; 3] = ints_of(NEVERMORE_REQUIEM.mana);
/// Ticks between requiems, by level.
pub const REQUIEM_COOLDOWN: [u32; 3] = ticks_of(NEVERMORE_REQUIEM.cooldown);
/// Attack speed the feast of souls adds, by level: `bonus_attack_speed`.
pub const FEAST_ATTACK_SPEED: [i32; 4] = NEVERMORE_FRENZY.value("bonus_attack_speed").ints::<4>();
/// Percent the feast of souls adds to movement speed, by level: `bonus_move_speed`.
pub const FEAST_MOVE_PCT: [i32; 4] = NEVERMORE_FRENZY.value("bonus_move_speed").ints::<4>();
/// Ticks the feast of souls runs: `duration`.
pub const FEAST_TICKS: u32 = NEVERMORE_FRENZY.value("duration").ticks::<1>()[0];
/// Ticks between two gatherings of a feast, the first one that long after
/// the cast: `soul_collection_interval`.
pub const FEAST_PERIOD_TICKS: u32 = NEVERMORE_FRENZY
    .value("soul_collection_interval")
    .ticks::<1>()[0];
/// Enemies one gathering of a feast takes souls from at most: `soul_collection_per_interval`.
pub const FEAST_PER_PERIOD: usize = NEVERMORE_FRENZY
    .value("soul_collection_per_interval")
    .first()
    .int() as usize;
/// How far from its caster a feast takes souls, in world units: `soul_collection_radius`.
pub const FEAST_RADIUS: i32 = NEVERMORE_FRENZY
    .value("soul_collection_radius")
    .first()
    .int();
/// Souls a feast takes from an enemy hero: `soul_collection_per_hero`.
pub const FEAST_SOULS_PER_HERO: u32 = NEVERMORE_FRENZY
    .value("soul_collection_per_hero")
    .first()
    .int() as u32;
/// Souls a feast takes from any other enemy: `soul_collection_per_creep`.
pub const FEAST_SOULS_PER_UNIT: u32 = NEVERMORE_FRENZY
    .value("soul_collection_per_creep")
    .first()
    .int() as u32;
/// Enemies one feast takes souls from in all, by level: `max_collection_count`.
pub const FEAST_TARGETS: [usize; 4] = {
    let counts = NEVERMORE_FRENZY.value("max_collection_count").ints::<4>();
    [
        counts[0] as usize,
        counts[1] as usize,
        counts[2] as usize,
        counts[3] as usize,
    ]
};
/// Ticks the souls of the enemies that died during a feast are kept past
/// the cap once the feast is over: `linger_duration`.
pub const FEAST_LINGER_TICKS: u32 = NEVERMORE_FRENZY.value("linger_duration").ticks::<1>()[0];
/// Mana the feast of souls costs, by level.
pub const FEAST_MANA: [i32; 4] = ints_of(NEVERMORE_FRENZY.mana);
/// Ticks between two feasts.
pub const FEAST_COOLDOWN: [u32; 4] = ticks_of(NEVERMORE_FRENZY.cooldown);
/// What Shadow Fiend's talents give, read from their bonuses in his values, for the clients to
/// word and the formulas to add.
///
/// Attack speed the level-10 talent `special_bonus_unique_nevermore_4` added to a feast: 7.41f's
/// `bonus_attack_speed` bonus. 7.42 took it out of his tree ("Level 10 Talent +30 Feast of
/// Souls Attack Speed replaced with +2 Feast of Souls Collected") and his script no longer
/// names it; the talent and its number stay, to be offered again.
pub const TALENT_FEAST_ATTACK_SPEED: i32 = 30;
/// Health the level-15 talent heals for every soul a feast takes:
/// `special_bonus_unique_nevermore_frenzy_heal_per_soul` on `heal_per_soul`.
pub const TALENT_FEAST_HEAL_PER_SOUL: i32 = NEVERMORE_FRENZY
    .value("heal_per_soul")
    .bonus("special_bonus_unique_nevermore_frenzy_heal_per_soul")
    .added()
    .int();
/// Raze damage a stack the level-10 talent adds: `special_bonus_unique_nevermore_7` on
/// `stack_bonus_damage`.
pub const TALENT_RAZE_STACK_DAMAGE: i32 = NEVERMORE_SHADOWRAZE1
    .value("stack_bonus_damage")
    .bonus("special_bonus_unique_nevermore_7")
    .added()
    .int();
/// Souls a feast takes from a hero on top, with the level-15 talent:
/// `special_bonus_unique_nevermore_frenzy_max_collection_count` on `soul_collection_per_hero`.
pub const TALENT_FEAST_SOULS_PER_HERO: u32 = NEVERMORE_FRENZY
    .value("soul_collection_per_hero")
    .bonus("special_bonus_unique_nevermore_frenzy_max_collection_count")
    .added()
    .int() as u32;
/// Armor the presence takes on top, with the level-15 talent: `special_bonus_unique_nevermore_3`
/// on `presence_armor_reduction`.
pub const TALENT_PRESENCE_ARMOR: Fixed = fixed(Dec(-NEVERMORE_DARK_LORD
    .value("presence_armor_reduction")
    .bonus("special_bonus_unique_nevermore_3")
    .added()
    .0));
/// Magic resistance, in percent, the presence takes on top, with the level-15 talent:
/// `special_bonus_unique_nevermore_3` on `presence_magic_resistance_reduction`.
pub const TALENT_PRESENCE_MAGIC_PCT: i32 = -NEVERMORE_DARK_LORD
    .value("presence_magic_resistance_reduction")
    .bonus("special_bonus_unique_nevermore_3")
    .added()
    .int();
/// Ticks the requiem's first hold lasts longer, with the level-20 talent:
/// `special_bonus_unique_nevermore_6` on `requiem_fear_duration` (and on
/// `requiem_slow_duration`, the same).
pub const TALENT_REQUIEM_HOLD_TICKS: u32 = NEVERMORE_REQUIEM
    .value("requiem_fear_duration")
    .bonus("special_bonus_unique_nevermore_6")
    .added()
    .secs()
    .ticks();
/// Souls the necromastery holds on top, with the level-20 talent:
/// `special_bonus_unique_nevermore_1` on `necromastery_max_souls`.
pub const TALENT_NECRO_SOUL_CAP: u32 = NEVERMORE_NECROMASTERY
    .value("necromastery_max_souls")
    .bonus("special_bonus_unique_nevermore_1")
    .added()
    .int() as u32;
/// Percent of its length a cast point lasts while a feast runs, with the level-25 talent:
/// `special_bonus_unique_nevermore_frenzy_castspeed` adds to `cast_speed_pct`, taken as that
/// many percent off the cast point.
pub const TALENT_FEAST_CAST_POINT_PCT: u32 = {
    let faster = NEVERMORE_FRENZY
        .value("cast_speed_pct")
        .bonus("special_bonus_unique_nevermore_frenzy_castspeed")
        .added()
        .int();
    (100 - faster) as u32
};
/// Souls a Shadow Fiend can hold at once: the cap and every soul the
/// largest feast can take on top of it, talents included.
pub const SOULS_HELD_MAX: u32 = NECRO_SOUL_CAP
    + TALENT_NECRO_SOUL_CAP
    + FEAST_TARGETS[3] as u32 * (FEAST_SOULS_PER_HERO + TALENT_FEAST_SOULS_PER_HERO);

/// Every number of a list, negated: the script writes reductions as negative values.
const fn negated<const N: usize>(values: [i32; N]) -> [i32; N] {
    let mut out = values;
    let mut at = 0;
    while at < N {
        out[at] = -values[at];
        at += 1;
    }
    out
}

/// The requiem's slow holds as long as its fear, the talent lengthening both alike: bota lays
/// both for one span.
const _: () = {
    let slow = NEVERMORE_REQUIEM
        .value("requiem_slow_duration")
        .ticks::<3>();
    let mut level = 0;
    while level < 3 {
        assert!(slow[level] == REQUIEM_HOLD_TICKS[level]);
        level += 1;
    }
    let slow_talent = NEVERMORE_REQUIEM
        .value("requiem_slow_duration")
        .bonus("special_bonus_unique_nevermore_6")
        .added();
    assert!(slow_talent.secs().ticks() == TALENT_REQUIEM_HOLD_TICKS);
};

const _: () = {
    assert!(FEAST_TICKS.is_multiple_of(FEAST_PERIOD_TICKS));
    assert!(FEAST_PER_PERIOD > 0);
    assert!(FEAST_TARGETS[3] >= FEAST_TARGETS[0]);
    assert!(REQUIEM_TICKS_PER_LINE > 0);
    assert!(TALENT_FEAST_CAST_POINT_PCT > 0);
    assert!(TALENT_FEAST_CAST_POINT_PCT <= 100);
};
