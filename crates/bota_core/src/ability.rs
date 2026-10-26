//! Every ability a hero may carry: what it is called, how it is aimed, and
//! what it asks for. What it does is the server's: it keeps a hook table
//! with one row to each entry here.

use bota_proto::{AbilityId, Aim, UnitKind};

use crate::heroes::{pudge, shadow_fiend};
use crate::{Aura, NEUTRAL_ABILITIES, objective, rules};

/// Whether an ability reaches an enemy that is spell immune: its
/// `SpellImmunityType` in Dota's ability files, and the extra protection
/// couriers have against a few that pierce (Liquipedia, Courier, "Pierce
/// Spell Immunity, affect ancient creeps, but can't affect courier").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pierces {
    /// `SPELL_IMMUNITY_ENEMIES_NO`: it passes a spell-immune enemy by.
    No,
    /// `SPELL_IMMUNITY_ENEMIES_YES`: it reaches one all the same.
    Yes,
    /// `SPELL_IMMUNITY_ENEMIES_YES`, but a spell-immune courier it does not.
    NotCouriers,
}

/// Which units an ability aimed at a unit may be aimed at, beyond what
/// [`AbilityDef::at_an_enemy`] asks: Dota's `AbilityUnitTargetType` and
/// `AbilityUnitTargetTeam` where they narrow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetFilter {
    /// The kinds it may be aimed at, bit `n` the [`UnitKind`] of variant
    /// index `n`. Zero: any kind.
    kinds: u32,
    /// Whether a unit of its caster's own side may be the target.
    own_side: bool,
}

impl TargetFilter {
    /// Any unit at all.
    pub const ANY: TargetFilter = TargetFilter {
        kinds: 0,
        own_side: true,
    };

    /// Units of these kinds, of the caster's own side too when `own_side`.
    pub const fn of(kinds: &[UnitKind], own_side: bool) -> TargetFilter {
        let mut bits = 0;
        let mut at = 0;
        while at < kinds.len() {
            assert!((kinds[at] as u32) < u32::BITS);
            bits |= 1 << kinds[at] as u32;
            at += 1;
        }
        TargetFilter {
            kinds: bits,
            own_side,
        }
    }

    /// Whether it narrows the kinds at all.
    pub const fn any_kind(&self) -> bool {
        self.kinds == 0
    }

    /// Whether a unit of a kind, on the caster's side or not, passes.
    pub const fn takes(&self, kind: UnitKind, own: bool) -> bool {
        (self.any_kind() || self.kinds & (1 << kind as u32) != 0) && (self.own_side || !own)
    }
}

/// One entry of the ability list.
#[derive(Clone, Copy, Debug)]
pub struct AbilityDef {
    /// The id it answers to on the wire: its line of the protocol's id
    /// registry, which [`ABILITIES`] places it at.
    pub id: AbilityId,
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
    /// an ability with no cast point, backswing or duration may be. It is
    /// also what Dota's `DOTA_ABILITY_BEHAVIOR_IGNORE_PSEUDO_QUEUE` asks for:
    /// every ability of 7.41f that carries that flag is immediate, so no
    /// flag of its own is kept.
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
    /// Whether a cast switches it on or off: Dota's
    /// `DOTA_ABILITY_BEHAVIOR_TOGGLE`. It is on while its caster wears a
    /// modifier it put on him.
    pub toggle: bool,
    /// Whether it reaches an enemy that is spell immune.
    pub pierces: Pierces,
    /// What its passive hands out to the units standing near its carrier:
    /// Dota's `DOTA_ABILITY_BEHAVIOR_AURA`. Absent for one that hands out
    /// nothing.
    pub aura: Option<Aura>,
    /// The level of its bearer from which it holds its one level, which no
    /// point ever buys: Dota's courier abilities, opening with the
    /// courier's level (Liquipedia, Courier: Speed Burst "Requires hero
    /// level 10"). Absent for one a point learns.
    pub granted_at: Option<u8>,
    /// Whether the entry is retired: its id keeps its place, but nothing
    /// carries it and the wire never names it again.
    pub retired: bool,
    /// Whether it is Dota's `DOTA_ABILITY_BEHAVIOR_HIDDEN`: never shown in a
    /// view, never learned and never cast by an order naming its slot. An
    /// order at a unit whose kind names it as its
    /// [`interact`](crate::KindDef::interact) casts it.
    pub hidden: bool,
    /// Which units it may be aimed at.
    pub targets: TargetFilter,
}

/// Nothing at all, so an entry names only what sets it apart.
/// Its id is no id: [`ABILITIES`] refuses an entry that names none.
pub(crate) const BARE_ABILITY: AbilityDef = AbilityDef {
    id: AbilityId(u16::MAX),
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
    toggle: false,
    pierces: Pierces::No,
    aura: None,
    granted_at: None,
    retired: false,
    hidden: false,
    targets: TargetFilter::ANY,
};

/// A courier's ability: one level, cast for no mana, held from its owner's
/// first level.
const fn courier(id: AbilityId, name: &'static str, cooldown: &'static [u32]) -> AbilityDef {
    AbilityDef {
        id,
        name,
        max_level: 1,
        passive: false,
        mana: &[0],
        cooldown,
        granted_at: Some(1),
        ..BARE_ABILITY
    }
}

/// How many abilities the game has: every table indexed by [`AbilityId`] is
/// this long.
pub const ABILITY_COUNT: usize = HERO_ABILITIES.len() + NEUTRAL_ABILITIES.len();

/// Every ability, indexed by [`AbilityId`]: each entry placed at the id it
/// carries, the heroes', the couriers' and the neutrals' alike. Every id
/// below [`ABILITY_COUNT`] holds exactly one entry.
pub const ABILITIES: [AbilityDef; ABILITY_COUNT] = {
    let mut every = [BARE_ABILITY; ABILITY_COUNT];
    let mut placed = [false; ABILITY_COUNT];
    let mut at = 0;
    while at < ABILITY_COUNT {
        let def = if at < HERO_ABILITIES.len() {
            HERO_ABILITIES[at]
        } else {
            NEUTRAL_ABILITIES[at - HERO_ABILITIES.len()].def
        };
        let id = def.id.0 as usize;
        assert!(id < ABILITY_COUNT, "an ability id runs past the table");
        assert!(!placed[id], "two abilities share an id");
        placed[id] = true;
        every[id] = def;
        at += 1;
    }
    every
};

/// The abilities of the heroes and the couriers, each carrying its id.
const HERO_ABILITIES: [AbilityDef; 20] = [
    pudge::MEAT_HOOK,
    pudge::ROT,
    pudge::FLESH_HEAP,
    pudge::DISMEMBER,
    pudge::MEAT_SHIELD,
    objective::CAPTURE,
    objective::LAMP_USE,
    // Burst and Shield are Dota's DOTA_ABILITY_BEHAVIOR_IMMEDIATE
    // (`courier_burst`, `courier_shield`): they break off nothing.
    AbilityDef {
        immediate: true,
        granted_at: Some(rules::COURIER_BURST_LEVEL),
        ..courier(AbilityId::BURST, "Burst", &[rules::COURIER_BURST_COOLDOWN])
    },
    courier(AbilityId::RETURN_ITEMS, "Return", &[0]),
    courier(AbilityId::TAKE_STASH, "Take Stash", &[0]),
    courier(AbilityId::DELIVER, "Deliver", &[0]),
    AbilityDef {
        immediate: true,
        granted_at: Some(rules::COURIER_SHIELD_LEVEL),
        ..courier(
            AbilityId::SHIELD,
            "Shield",
            &[rules::COURIER_SHIELD_COOLDOWN],
        )
    },
    shadow_fiend::RAZE_NEAR,
    shadow_fiend::RAZE_MID,
    shadow_fiend::RAZE_FAR,
    shadow_fiend::REQUIEM,
    shadow_fiend::NECROMASTERY,
    shadow_fiend::PRESENCE,
    shadow_fiend::FEAST_OF_SOULS,
    // Retired: a Blood Grenade in flight named a pseudo-ability; it names
    // its item now. Its id is never handed out again.
    AbilityDef {
        id: AbilityId(16),
        name: "Throw Grenade",
        retired: true,
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

/// What one ability is, or nothing if no such ability exists or it is
/// retired.
#[inline]
pub fn ability_def(id: AbilityId) -> Option<&'static AbilityDef> {
    ABILITIES.get(usize::from(id.0)).filter(|def| !def.retired)
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
