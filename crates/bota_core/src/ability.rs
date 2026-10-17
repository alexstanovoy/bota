//! Every ability a hero may carry: what it is called, how it is aimed, and
//! what it asks for. What it does is the server's: it keeps a hook table
//! with one row to each entry here.

use bota_proto::{AbilityId, Aim};

use crate::heroes::{pudge, shadow_fiend};
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
pub(crate) const BARE_ABILITY: AbilityDef = AbilityDef {
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

/// A courier's ability: one level, cast for no mana.
const fn courier(name: &'static str, cooldown: &'static [u32]) -> AbilityDef {
    AbilityDef {
        name,
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown,
        ..BARE_ABILITY
    }
}

/// How many abilities the game has: every table indexed by [`AbilityId`] is
/// this long.
pub const ABILITY_COUNT: usize = 17;

/// Every ability, indexed by [`AbilityId`].
pub const ABILITIES: [AbilityDef; ABILITY_COUNT] = [
    pudge::MEAT_HOOK,
    pudge::ROT,
    pudge::FLESH_HEAP,
    pudge::DISMEMBER,
    courier("Burst", &[rules::COURIER_BURST_COOLDOWN]),
    courier("Return", &[0]),
    courier("Take Stash", &[0]),
    courier("Deliver", &[0]),
    courier("Shield", &[rules::COURIER_SHIELD_COOLDOWN]),
    shadow_fiend::RAZE_NEAR,
    shadow_fiend::RAZE_MID,
    shadow_fiend::RAZE_FAR,
    shadow_fiend::REQUIEM,
    shadow_fiend::NECROMASTERY,
    shadow_fiend::PRESENCE,
    shadow_fiend::FEAST_OF_SOULS,
    AbilityDef {
        name: "Throw Grenade",
        ..BARE_ABILITY
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
