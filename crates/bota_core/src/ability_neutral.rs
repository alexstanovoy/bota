//! Every ability a neutral creep carries: what it is called, how it is aimed,
//! what it costs, and what it does, as data a handful of shared mechanics
//! read.
//!
//! The numbers are the game's `scripts/npc/npc_abilities.txt` (and, for Ice
//! Armor, Ensnare and Raise Dead, the hero files that hold them), read by
//! `tools/import_items` into [`neutral_data`](crate::neutral_data) and taken
//! from it by Dota's own keys. What the scripts leave out is in the
//! importer's `neutral_overrides.rs`, each with its source: the backswings.
//! When the neutral AI casts an ability, and the parts of an effect the files
//! leave out, are Liquipedia's article of each creep (read 2026-10-18). A
//! neutral's abilities come in four levels (the scripts say `MaxLevel 1` and
//! list four numbers); one spawns with them at the level of
//! [`neutral_ability_level`] and keeps it.
//!
//! A new ability that one of the mechanics below already covers is one
//! variant appended to [`NeutralAbility`], one entry appended to
//! [`NEUTRAL_ABILITIES`] carrying the next free id, and that id's line in the
//! protocol's id registry (append-only: an id is never renumbered or handed
//! out twice).

use bota_proto::{AbilityId, Aim, DamageKind, EffectId};

use crate::neutral_data as script;
use crate::{
    ABILITY_COUNT, AbilityDef, Among, Aura, AuraModifier, BARE_ABILITY, CastRule, Dec, Mark, Need,
    NeutralKind, Pick, Reach, ScriptData, When, Whom, Within, rules,
};

/// One neutral ability. Its place is its entry's in [`NEUTRAL_ABILITIES`];
/// its [`AbilityId`] is the one that entry carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NeutralAbility {
    ProspectingAura,
    NeutralUpgrade,
    StealWeapon,
    SpeedAura,
    Break,
    Piercing,
    Heal,
    HealAmplificationAura,
    EnvenomedWeapon,
    Vex,
    FrostAttack,
    TakeOff,
    ChainLightning,
    CloakAura,
    WarStomp,
    Intimidate,
    CriticalStrike,
    PackleadersAura,
    Purge,
    ManaBurn,
    ManaAura,
    Shockwave,
    UnholyAura,
    OgreSmash,
    IceArmor,
    HurlBoulder,
    ShardSplit,
    SwiftnessAura,
    DeathThroeRush,
    ThunderClap,
    DeathThroePower,
    Tornado,
    Hurricane,
    ToughnessAura,
    Ensnare,
    RaiseDead,
    SeedShot,
    MagicAmplificationAura,
    Fireball,
    SplashAttack,
    DragonhideAura,
    WeakeningAura,
    GraniteAura,
    WarDrumsAura,
    Slam,
    Frenzy,
    TimeWarpAura,
    IcefireBomb,
    ProwlerAura,
    Desecrate,
    Petrify,
    RiverbornAura,
    ArmOfTheDeep,
    TendrilsOfTheDeep,
    CongregationsOfTheDeep,
    WaterBubbleSmall,
    WaterBubbleMedium,
    WaterBubbleLarge,
    Rally,
    Tempest,
    StrengthOfTheImmortal,
    Devotion,
    SpellBlock,
    Bash,
    RoshanSlam,
    RevengeRoar,
    GrabAndThrow,
    HurlBigBoulder,
    Reserved89,
    Eggstremism,
    OdobenusBlight,
    MinersAura,
    MinersSlam,
    SealFlop,
    SealDisapprovalAura,
    GoldUpgrade,
    StealWeaponCast,
    SnowballShot,
    SearingSwoop,
}

impl NeutralAbility {
    /// Its id on the wire.
    pub const fn id(self) -> AbilityId {
        self.entry().def.id
    }

    /// The neutral ability an id names, if it names one.
    pub const fn of(id: AbilityId) -> Option<NeutralAbility> {
        let at = id.0 as usize;
        if at < NEUTRAL_BY_ID.len() {
            NEUTRAL_BY_ID[at]
        } else {
            None
        }
    }

    /// Its entry.
    pub const fn entry(self) -> &'static NeutralAbilityDef {
        &NEUTRAL_ABILITIES[self as usize]
    }

    /// What it does.
    pub const fn spell(self) -> &'static Spell {
        &self.entry().spell
    }

    /// Whether its modifier is kept apart for each unit that put it on:
    /// the auras that stack, and what an attack lays on an enemy. Every
    /// other is one modifier on the bearer, whoever put it there.
    pub const fn per_source(self) -> bool {
        match self.spell() {
            Spell::Aura(aura) => aura.stacks,
            // Odobenus Blight is one instance on a target whoever laid it: a second blow, of the
            // same Footman or another, refreshes it.
            Spell::Strike(_) => !matches!(self, NeutralAbility::OdobenusBlight),
            _ => false,
        }
    }
}

/// A value by ability level, one to four.
pub type Lv = [i32; 4];

const fn lv(one: i32, two: i32, three: i32, four: i32) -> Lv {
    [one, two, three, four]
}

const fn all(value: i32) -> Lv {
    [value; 4]
}

/// The value of a level, counted from one.
pub const fn at_level(values: &Lv, level: u8) -> i32 {
    let at = if level == 0 { 0 } else { level as usize - 1 };
    values[if at > 3 { 3 } else { at }]
}

/// Ticks of `ms` milliseconds, to the nearest tick.
pub const fn ticks_of_ms(ms: i32) -> u32 {
    assert!(ms >= 0);
    ((ms as i64 * rules::TICKS_PER_SECOND as i64 + 500) / 1000) as u32
}

/// The level a neutral spawned at `tick` holds its abilities at: one until
/// 15:00, two until 30:00, three from then on. Level four is only ever given
/// by taking a neutral over, which nothing in bota does.
pub const fn neutral_ability_level(tick: u32, schedule: rules::Schedule) -> u8 {
    let since = tick.saturating_sub(schedule.horn);
    let steps = since / rules::NEUTRAL_ABILITY_LEVEL_PERIOD_TICKS;
    1 + if steps > 2 { 2 } else { steps as u8 }
}

/// One stat a spell's modifier changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stat {
    /// Percent of its move speed.
    MovePct,
    AttackSpeed,
    /// Whole points of armor.
    Armor,
    /// Percent of its base armor, the agility's included.
    BaseArmorPct,
    /// Magic resistance, percent, multiplying with what is already there:
    /// on a hero.
    MagicResistPct,
    /// The same on anything but a hero.
    MagicResistCreepPct,
    /// Percent of its main attack damage, its base and what its primary
    /// attribute pays: summed with every other before it is taken.
    DamagePct,
    /// Flat bonus attack damage, added after every percent.
    Damage,
    /// Percent of all the damage it deals.
    OutgoingPct,
    /// Tenths of a point of health a second.
    HpRegen,
    /// Hundredths of a point of mana a second.
    ManaRegen,
    /// Percent of the health regeneration it has, taken off.
    RegenCutPct,
    /// Percent of its attack damage healed back.
    LifestealPct,
    /// Percent of its maximum health.
    MaxHpPct,
    /// Percent more magical damage it takes.
    MagicTakenPct,
    /// Percent off every cooldown it starts.
    CooldownPct,
    /// Percent of its attacks that never miss.
    AccuracyPct,
    /// Percent resisted, multiplicatively across sources.
    SlowResistPct,
    StatusResistPct,
    /// Reduction of incoming illusion attacks, leaving spells unaffected.
    IllusionAttackReductionPct,
    IncomingDamagePct,
    /// World units added to movement speed before percentage changes.
    MoveFlat,
    /// Percent taken off all health restoration, including incoming healing.
    RestorationCutPct,
}

/// The stat changes of an ability or item modifier, a gain above zero and a loss below.
/// Ability values are selected by level; item values repeat across levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buff {
    pub stats: [(Stat, Lv); 8],
}

impl Buff {
    /// One stat changed.
    pub const fn of(stat: Stat, values: Lv) -> Buff {
        Self::many(&[(stat, values)])
    }

    /// Two stats changed.
    pub const fn two(first: Stat, firsts: Lv, second: Stat, seconds: Lv) -> Buff {
        Self::many(&[(first, firsts), (second, seconds)])
    }

    /// A bounded set of stat changes, shared by ability and item effects.
    pub const fn many(stats: &[(Stat, Lv)]) -> Buff {
        let mut buff = Buff {
            stats: [(Stat::MovePct, all(0)); 8],
        };
        assert!(stats.len() <= buff.stats.len(), "too many stats in a buff");
        let mut at = 0;
        while at < stats.len() {
            buff.stats[at] = stats[at];
            at += 1;
        }
        buff
    }

    pub const fn scaled(mut self, stacks: i32) -> Buff {
        let mut stat = 0;
        while stat < self.stats.len() {
            let mut level = 0;
            while level < self.stats[stat].1.len() {
                self.stats[stat].1[level] = self.stats[stat].1[level].saturating_mul(stacks);
                level += 1;
            }
            stat += 1;
        }
        self
    }

    /// How much it changes `stat` by at `level`.
    pub const fn get(&self, stat: Stat, level: u8) -> i32 {
        let mut sum = 0;
        let mut at = 0;
        while at < self.stats.len() {
            let (which, values) = &self.stats[at];
            if *which as u8 == stat as u8 {
                sum += at_level(values, level);
            }
            at += 1;
        }
        sum
    }
}

/// A slow of both move and attack speed.
const fn chill(move_pct: Lv, attack_speed: Lv) -> Buff {
    Buff::two(Stat::MovePct, move_pct, Stat::AttackSpeed, attack_speed)
}

/// A handed-out effect, its reach and its modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuraSpell {
    /// Its bearer's side or the others: living units of it, no building,
    /// no ward.
    pub whom: Whom,
    /// How far it reaches, in world units, before the neutral cap.
    pub radius: i32,
    pub buff: Buff,
    /// The effect its modifier shows.
    pub effect: EffectId,
    /// Whether one from each source counts: Dota's Dragonhide and Rally.
    /// Otherwise any number of sources hand out one.
    pub stacks: bool,
    /// Whether it only works while its bearer stands in the river.
    pub water: bool,
    /// Whether a Break silences it: the game's `IsBreakable`, which every
    /// neutral aura has but the Granite Aura.
    pub breakable: bool,
}

/// A passive on the bearer's own attacks: a neutral's passive ability, or an item's
/// ([`Carried::strikes`](crate::Carried::strikes)). Every chance rolls with Dota's
/// pseudo-random distribution, on a sequence of the source's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strike {
    /// Counts the blows landed on each enemy; the `hits`th, off cooldown,
    /// disarms it for `ms`.
    Disarm { hits: u8, ms: Lv, counter: EffectId },
    /// A blow off cooldown breaks the target's passives for `ms`.
    Break { ms: i32 },
    /// Every blow lays the spell's modifier: for `hero_ms` on a hero,
    /// `other_ms` on anything else, magical damage `dps` a second for as long.
    Lay {
        buff: Buff,
        effect: EffectId,
        hero_ms: i32,
        other_ms: i32,
        dps: Lv,
    },
    /// A `chance_pct` chance, held over every run of swings, to deal
    /// `mult_pct` percent damage. Of several critical strikes the strongest rolls first and the
    /// first that lands counts alone (Dota 7.30: "Multiple Critical Strike procs are now rolled
    /// from strongest to weakest and will stop rolling once a proc occurs").
    Crit {
        chance_pct: i32,
        mult_pct: Lv,
        /// Whether it leaves the swing as it is and lands `mult_pct` percent of its damage on
        /// top as magical damage (Revenant's Brooch's Phantom Critical, "considered a Critical
        /// Strike for other sources of Critical Strikes").
        magic: bool,
        /// What it rolls on: an item's on a swing at an enemy hero or creep alone ("Critical
        /// Strike does not work against buildings"; Dota's native callbacks check the target's
        /// unit type and side before they roll).
        targets: ProcTargets,
    },
    /// Every other enemy within `radius` of the struck takes `pct` percent of
    /// the blow, as physical damage.
    Splash { radius: i32, pct: i32 },
    /// A blow off cooldown begun below `below_pct` of its own health roots
    /// the struck for `ms` and deals physical `dps` a second for as long.
    Petrify {
        below_pct: i32,
        ms: Lv,
        dps: i32,
        effect: EffectId,
    },
    /// Rolled at the attack point and carried to impact as conditional attack damage. Of a
    /// unit's bashes one rolls (Skull Basher: "Does not stack with other sources of Bash"):
    /// the one that deals most, then the first.
    Bash {
        chance_pct: i32,
        damage: Lv,
        stun_ms: Lv,
        effect: EffectId,
        /// What it rolls on: an item's on every swing, whatever it is aimed at (Dota's native
        /// Skull Basher rolls before it looks at the target).
        rolls_at: ProcTargets,
        /// What a bash that rolled lands on: an item's on a hero or a creep of either side, a
        /// deny's too (no side check in Dota's native bash).
        lands_on: ProcTargets,
    },
    /// Every enemy unit but the struck (no building, ward or courier) in a trapezoid that opens
    /// at the attacker towards the struck takes `hero_pct` percent of the blow (`creep_pct` for a
    /// unit that is no hero) as physical damage: `start_radius` to either side of its axis at
    /// the attacker, `end_radius` at `distance` along it (Dota's cleave radii, 7.00 "Cleave
    /// Mechanics"; 7.20 widened the end to 360 and the distance to 650). Each cleave of a unit
    /// lands apart ("each Cleave's damage is applied separately"); an AoE bonus widens both radii
    /// (Dota 7.36: "AoE Bonuses now affect all auras and Cleave sources").
    Cleave {
        hero_pct: i32,
        creep_pct: i32,
        start_radius: i32,
        end_radius: i32,
        distance: i32,
    },
    /// A `chance_pct` chance, rolled at the attack point, that the swing goes through evasion
    /// and, once it lands, lets loose a bolt of `damage` magical damage that strikes the struck
    /// and leaps on to `strikes` targets in all, each within `radius` of the last, `delay_ms`
    /// apart. The source then waits `wait_ms` on a timer of its own, which no cooldown cut or
    /// refresh touches and no item shows (Maelstrom's Chain Lightning).
    Chain {
        chance_pct: i32,
        damage: i32,
        strikes: u8,
        radius: i32,
        delay_ms: i32,
        wait_ms: i32,
    },
    /// A `chance_pct` chance, rolled at the attack point, that the swing throws missiles at up
    /// to `count` more enemies besides its target, within the attack's range and `range_bonus`
    /// and `angle` degrees in front of the attacker, each dealing `base_damage` and `pct`
    /// percent of a plain swing; the swing itself deals `base_damage` more. The extra missiles
    /// carry nothing else of the swing (Specialist's Array's Splitshot). Only a swing at an enemy
    /// unit (no building, ward or courier) splits, and heroes are chosen before the rest.
    Split {
        chance_pct: i32,
        count: u8,
        range_bonus: i32,
        angle: i32,
        base_damage: i32,
        pct: i32,
    },
}

/// What a strike rolls on, or lands on, by what the swing is aimed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcTargets {
    /// Whatever it is aimed at: a neutral's critical strike, as it always was.
    Any,
    /// Anything but a ward: Roshan's bash, as it always was.
    NoWards,
    /// A hero or a creep of either side: no building, ward or courier.
    Units,
    /// A hero or a creep of the other side: no ally (a deny), building, ward or courier.
    EnemyUnits,
}

impl Strike {
    /// What its roll asks of the swing's target: an item's critical strike, chain lightning,
    /// split shot and cleave an enemy hero or creep (Dota's native callbacks compare the sides
    /// and the target's unit type before they roll: no deny, no courier, no building).
    pub const fn rolls_at(self) -> ProcTargets {
        match self {
            Strike::Crit { targets, .. } => targets,
            Strike::Bash { rolls_at, .. } => rolls_at,
            Strike::Cleave { .. } | Strike::Chain { .. } | Strike::Split { .. } => {
                ProcTargets::EnemyUnits
            }
            Strike::Disarm { .. }
            | Strike::Break { .. }
            | Strike::Lay { .. }
            | Strike::Splash { .. }
            | Strike::Petrify { .. } => ProcTargets::Any,
        }
    }

    /// Whether an illusion's attacks go through it. An illusion strikes critically (its
    /// weakened attack, and a Phantom Critical's magic as weakened), but it bashes, cleaves,
    /// lets loose chain lightning and splits its shots never: these procs of a unit's items are
    /// its own body's alone.
    pub const fn by_illusions(self) -> bool {
        match self {
            Strike::Crit { .. } => true,
            Strike::Bash { .. }
            | Strike::Cleave { .. }
            | Strike::Chain { .. }
            | Strike::Split { .. } => false,
            Strike::Disarm { .. }
            | Strike::Break { .. }
            | Strike::Lay { .. }
            | Strike::Splash { .. }
            | Strike::Petrify { .. } => true,
        }
    }
}

/// A passive that goes off as its bearer dies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Throe {
    /// Every ally within `radius` wears the spell's modifier for `ms`.
    Rouse {
        radius: i32,
        ms: i32,
        buff: Buff,
        effect: EffectId,
    },
    /// It falls apart into `count` of `kind`, each with `hp` health and
    /// `damage` attack damage, that live `life_ms` and go back to its camp;
    /// every enemy within `radius` is slowed by `slow_pct` for `slow_ms`.
    Split {
        kind: NeutralKind,
        count: Lv,
        hp: Lv,
        damage: Lv,
        life_ms: i32,
        radius: i32,
        slow_pct: i32,
        slow_ms: i32,
        effect: EffectId,
    },
}

/// What a cast does once it goes off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Grab(&'static Grab),
    Roar(&'static Roar),
    /// Every enemy within `radius` of the caster takes `damage` of `kind`
    /// and is stunned, or wears the spell's modifier, for `hero_ms` on a hero
    /// and `other_ms` on anything else.
    Stomp {
        growth: Option<crate::DamageGrowth>,
        other_pct: i32,
        radius: Lv,
        damage: Lv,
        kind: DamageKind,
        hero_ms: Lv,
        other_ms: Lv,
        then: Then,
    },
    /// A missile at the target at `speed`: `hero_damage` magical to a hero,
    /// `other_damage` to anything else, then stunned or rooted for `ms`.
    Bolt {
        speed: i32,
        hero_damage: i32,
        other_damage: i32,
        ms: i32,
        root: bool,
    },
    /// `damage` magical to the target, then to up to `targets - 1` more
    /// enemies, each the nearest unstruck within `radius` of the last,
    /// `delay_ms` apart, each keeping `keep_pct` percent of the last.
    Chain {
        damage: Lv,
        targets: u8,
        radius: i32,
        delay_ms: i32,
        keep_pct: Lv,
    },
    /// Raises `count` of `kind` beside the caster, with `hp` health and
    /// `damage` attack damage, for `life_ms`.
    Summon {
        kind: NeutralKind,
        count: u8,
        hp: Lv,
        damage: Lv,
        life_ms: i32,
    },
    /// The caster wears the spell's modifier for `ms`.
    Enrage {
        buff: Buff,
        effect: EffectId,
        ms: i32,
    },
    /// Every enemy within `radius` of the spot `reach` in front of the
    /// caster takes `damage` and `current_pct` percent of its current health,
    /// magical, and is stunned for `ms`.
    Smash {
        reach: i32,
        radius: Lv,
        damage: Lv,
        current_pct: i32,
        ms: i32,
    },
    /// `lines` spikes from the caster, `spread_deg` degrees apart and centred
    /// on its aim, each `length` long and `width` wide: every enemy they
    /// cross takes `damage` magical and, for `ms`, is stunned or broken as
    /// `afflicts` says.
    Spikes {
        lines: u8,
        spread_deg: i32,
        length: i32,
        width: i32,
        damage: Lv,
        afflicts: Afflicts,
        ms: Lv,
    },
    /// A big boulder that lands at the spot aimed at: every enemy within `radius` of it takes
    /// `damage` of `kind` and is stunned for `ms`. Flight time increases up to maximum range.
    Boulder {
        min_ms: i32,
        max_ms: i32,
        radius: i32,
        damage: Lv,
        kind: DamageKind,
        ms: Lv,
    },
    /// A barrier on an ally soaking `absorb` magical damage for `ms`; as it
    /// goes, it mends `heal_pct` percent of `absorb` on the ally and on every
    /// ally within `radius` of it.
    Bubble {
        absorb: Lv,
        ms: i32,
        heal_pct: i32,
        radius: i32,
    },
    /// Carries the target `distance` along the line from the caster through
    /// it, at `speed`.
    Push {
        distance: i32,
        speed: i32,
    },
    /// Takes the target's buffs off and slows it for `ms`, the slow falling from `start_pct` by
    /// `start_pct / rate` each second (`rate` is the script's `purge_rate`: the seconds it takes
    /// to fall away).
    Purge {
        start_pct: i32,
        rate: i32,
        ms: i32,
    },
    /// A tornado of `kind` stands at the target for `life_ms`.
    Tornado {
        kind: NeutralKind,
        life_ms: i32,
    },
    /// `count` pulses `interval_ms` apart, the first as the cast goes off, at the spot `reach` in
    /// front of the caster along its facing: each deals the whole `damage` (magical) to every
    /// enemy within `radius` of the spot, and slows it by `slow` for `slow_ms`.
    Pulses {
        reach: i32,
        radius: i32,
        damage: Lv,
        count: u8,
        interval_ms: i32,
        slow: Buff,
        slow_ms: i32,
        effect: EffectId,
    },
    /// `damage` (magical) and a `slow` for `slow_ms` to the enemy aimed at, or a heal of `damage`
    /// to an ally; then, `delay_ms` after each hit, on to a unit of either side picked at random
    /// within `range` of the last, never the one just hit, until it has bounced `bounces` times.
    Ricochet {
        damage: Lv,
        bounces: Lv,
        range: i32,
        delay_ms: i32,
        slow: Buff,
        slow_ms: i32,
        effect: EffectId,
    },
    /// The caster leaps `distance` forward along its facing, starting at `speed` and speeding up
    /// by `acceleration` each second, and lands with `damage` (magical) and `slow` for `slow_ms`
    /// on every enemy within `radius`; then it leaps again until it has made `leaps`.
    Leap {
        distance: i32,
        speed: i32,
        acceleration: i32,
        radius: i32,
        damage: i32,
        leaps: u8,
        slow: Buff,
        slow_ms: i32,
        effect: EffectId,
    },
}

/// What spikes do to the enemies they cross, besides their damage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Afflicts {
    /// A stun.
    Stun,
    /// A Break: their passives are silenced.
    Break,
}

/// A channeled grip followed by an airborne throw and a landing effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grab {
    pub duration: u32,
    pub contact_after: u32,
    pub distance: i32,
    pub flight: u32,
    pub height: i32,
    pub damage: i32,
    pub damage_per_throw: i32,
    pub health_growth: crate::DamageGrowth,
    pub stun: u32,
    pub grabbed_effect: EffectId,
    pub thrown_effect: EffectId,
    pub contact_ability: NeutralAbility,
}

/// A delayed area pulse, with a temporary incoming-damage aura.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Roar {
    pub windup: u32,
    pub windup_effect: EffectId,
    pub emitter_effect: EffectId,
    pub disarm_effect: EffectId,
    pub damage_effect: EffectId,
    pub radius: i32,
    pub damage: i32,
    pub growth: crate::DamageGrowth,
    pub disarm: u32,
    pub aura_duration: u32,
    pub aura: Aura,
    pub buff: Buff,
}

/// What a stomp leaves on what it hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Then {
    /// Stunned.
    Stun,
    /// The spell's modifier.
    Wear { buff: Buff, effect: EffectId },
}

/// A cast's hold on the others: once it goes off, every neutral within
/// `radius` carrying one of the same family waits `ms` before casting its
/// own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shared {
    pub family: u8,
    pub radius: i32,
    pub ms: i32,
}

/// The cast probability and attempt cooldown from the neutral's script values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CastChance {
    pub pct: i32,
    pub wait_ms: i32,
}

/// What a neutral ability does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spell {
    /// Shown as the game shows it, and never going off: the neutral AI never
    /// casts it ("does not attempt to cast this ability by default"), and
    /// nothing in bota takes command of a neutral.
    Uncast,
    /// A passive that reaches nothing a neutral meets: an aura for the
    /// heroes of its own side, or for heals its allies cast.
    Idle,
    /// The stats that come with the clock: [`rules::NEUTRAL_UPGRADE_PERIOD_TICKS`].
    Upgrade,
    /// Its attacks do 150% to creeps, 50% to heroes and buildings.
    Piercing,
    Aura(AuraSpell),
    /// Enemies within `radius` take magical damage each `period_ms`, from
    /// `far_dps` a second at the edge to `near_dps` within `near_radius`,
    /// and wear the spell's modifier.
    Storm {
        radius: i32,
        near_radius: i32,
        near_dps: i32,
        far_dps: i32,
        period_ms: i32,
        buff: Buff,
        effect: EffectId,
        /// Whether a Break silences it: the game's `IsBreakable`, which the
        /// tornado's Tempest has not.
        breakable: bool,
    },
    Strike(Strike),
    Throe(Throe),
    /// Cast by the neutral AI when `when` holds.
    Cast {
        act: Act,
        when: When,
        shared: Option<Shared>,
        /// How often the neutral AI follows through when the rule picks something: the script's
        /// `neutral_chance_to_cast`. Absent when the script has none: it casts at once.
        chance: Option<CastChance>,
    },
    /// A permanent self modifier, optionally also carrying an aura.
    Intrinsic {
        buff: Buff,
        effect: EffectId,
        aura: Option<Aura>,
        moving: Option<&'static crate::Migration>,
    },
    /// Absorbs an enemy unit-target spell while ready, with intrinsic self stats.
    Block {
        buff: Buff,
        effect: EffectId,
        breakable: bool,
    },
}

impl Spell {
    /// The aura a passive of `ability` is, for one that hands its modifier
    /// out to the units near its bearer: an aura or a storm. Each reaches
    /// living units only and lingers [`rules::AURA_LINGER_TICKS`]; a Break
    /// silences it as its spell's `breakable` says.
    const fn aura(&self, ability: NeutralAbility) -> Option<Aura> {
        if let Spell::Intrinsic { aura, .. } = *self {
            return aura;
        }
        let (whom, radius, water, breakable) = match *self {
            Spell::Aura(AuraSpell {
                whom,
                radius,
                water,
                breakable,
                ..
            }) => (whom, radius, water, breakable),
            Spell::Storm {
                radius, breakable, ..
            } => (Whom::Enemies, radius, false, breakable),
            _ => return None,
        };
        Some(Aura {
            growth_clock: None,
            whom,
            reaches: Reach::Living,
            radius,
            linger: rules::AURA_LINGER_TICKS,
            breakable,
            invulnerable: false,
            water,
            modifier: AuraModifier::Spell(ability),
        })
    }
}

/// The neutral ability of each [`AbilityId`], for those that name one.
const NEUTRAL_BY_ID: [Option<NeutralAbility>; ABILITY_COUNT] = {
    let mut by_id = [None; ABILITY_COUNT];
    let mut at = 0;
    while at < NEUTRAL_ABILITIES.len() {
        let entry = &NEUTRAL_ABILITIES[at];
        by_id[entry.def.id.0 as usize] = Some(entry.ability);
        at += 1;
    }
    by_id
};

/// One entry: the ability as any ability is, and what it does.
#[derive(Clone, Copy, Debug)]
pub struct NeutralAbilityDef {
    /// The ability the entry is for: its place in [`NEUTRAL_ABILITIES`].
    pub ability: NeutralAbility,
    /// The ability as any ability is, its id included.
    pub def: AbilityDef,
    /// The numbered cast animation named by the ability, when it has one.
    pub cast_animation_slot: Option<u8>,
    pub spell: Spell,
}

// Every ability row the generated table holds has an entry here: a patch that gives a unit an
// ability bota has no entry for fails the build.
const _: () = {
    let mut row = 0;
    while row < script::ABILITY_ROWS.len() {
        let mut at = 0;
        let mut found = false;
        while at < NEUTRAL_ABILITIES.len() {
            found |= crate::item_data::str_eq(
                NEUTRAL_ABILITIES[at].def.key,
                script::ABILITY_ROWS[row].key,
            );
            at += 1;
        }
        assert!(found, "a neutral ability of the script has no entry");
        row += 1;
    }
};

const _: () = {
    let mut at = 0;
    while at < NEUTRAL_ABILITIES.len() {
        let entry = &NEUTRAL_ABILITIES[at];
        assert!(entry.ability as usize == at);
        let casts = matches!(entry.spell, Spell::Cast { .. } | Spell::Uncast);
        assert!(entry.def.passive != casts);
        at += 1;
    }
};

/// A passive.
const fn passive() -> AbilityDef {
    AbilityDef {
        max_level: 4,
        ..BARE_ABILITY
    }
}

/// A passive with a cooldown, in seconds by level.
const fn passive_cooling(cooldown: &'static [u32]) -> AbilityDef {
    AbilityDef {
        cooldown,
        ..passive()
    }
}

/// An ability that is cast.
#[allow(clippy::too_many_arguments)]
const fn cast(
    aim: Aim,
    at_an_enemy: bool,
    mana: &'static [i32],
    cooldown: &'static [u32],
    range: i32,
    point: u32,
    backswing: u32,
) -> AbilityDef {
    AbilityDef {
        aim,
        max_level: 4,
        passive: false,
        at_an_enemy,
        block: if at_an_enemy && matches!(aim, Aim::Unit) {
            crate::BlockTiming::Cast
        } else {
            crate::BlockTiming::Never
        },
        mana,
        cooldown,
        range,
        point,
        backswing,
        ..BARE_ABILITY
    }
}

/// One entry, its ability carrying the aura its spell is, if it is one.
const fn entry(
    id: AbilityId,
    data: &'static ScriptData,
    ability: NeutralAbility,
    def: AbilityDef,
    spell: Spell,
) -> NeutralAbilityDef {
    let cast_animation_slot = match data.find("AbilityCastAnimationSlot") {
        Some(slots) => {
            let slot = slots[0].int();
            assert!(1 <= slot && slot <= 7);
            Some(slot as u8)
        }
        None => None,
    };
    NeutralAbilityDef {
        ability,
        cast_animation_slot,
        def: AbilityDef {
            id,
            key: data.key,
            aura: spell.aura(ability),
            block: if matches!(
                spell,
                Spell::Cast {
                    act: Act::Bolt { .. },
                    ..
                }
            ) && matches!(def.block, crate::BlockTiming::Cast)
            {
                crate::BlockTiming::Impact
            } else {
                def.block
            },
            ..def
        },
        spell,
    }
}

/// An aura for its own side, reaching its script's `radius`.
const fn allies(data: &ScriptData, buff: Buff, effect: u16) -> Spell {
    Spell::Aura(AuraSpell::allies(data, buff, effect))
}

impl AuraSpell {
    /// An aura for its own side: one modifier whatever the sources, silenced by a Break when its
    /// script says `IsBreakable`, working anywhere.
    const fn allies(data: &ScriptData, buff: Buff, effect: u16) -> AuraSpell {
        AuraSpell {
            whom: Whom::Allies,
            radius: data.int("radius"),
            buff,
            effect: EffectId(effect),
            stacks: false,
            water: false,
            breakable: data.breakable(),
        }
    }
}

/// Families of casts that hold each other off.
const FROG_SPIKES: u8 = 0;
const GOLEM_BOULDER: u8 = 1;
const OGRE_SMASH: u8 = 2;

/// The hold the frogs' spikes put on each other (7.41: "all copies ... on
/// other neutral Boglets, Croakers, and Ancient Croakers within 1200 range
/// are put on 5s cooldown": their scripts' `neutral_shared_cooldown`). The
/// golems and the ogres "can't do it if recently other golem used this
/// ability", with no numbers given; the frogs' are used for them.
const fn hold(family: u8) -> Option<Shared> {
    Some(Shared {
        family,
        radius: script::FROGMEN_ARM_OF_THE_DEEP.int("neutral_shared_radius"),
        ms: script::FROGMEN_ARM_OF_THE_DEEP.ms("neutral_shared_cooldown"),
    })
}

/// When a Marshmage puts up its bubble.
const BUBBLE_WHEN: When = When::Think(CastRule {
    among: Among::Allies(Within::CastRange),
    needs: &[],
    marks: &[Mark::HurtBelow { pct: 50 }, Mark::Unmarked],
    pick: Pick::LeastHealthPct,
});

/// What a script row of an ability gives, by level: a list shorter than the four levels holds
/// its last number for the rest. Anything a row lacks that the code asks for fails the build:
/// the importer writes what a script leaves out into its row, with the reason (an override or a
/// default of its override file), so nothing is defaulted here.
impl ScriptData {
    /// The first level's number, whole.
    const fn int(&self, key: &str) -> i32 {
        self.first(key).int()
    }

    /// The first level's number of seconds, in milliseconds.
    const fn ms(&self, key: &str) -> i32 {
        self.first(key).0
    }

    /// Whole numbers, a level.
    const fn lv(&self, key: &str) -> Lv {
        self.lv_per(key, 1)
    }

    /// Seconds as milliseconds, a level.
    const fn lv_ms(&self, key: &str) -> Lv {
        self.lv_per(key, 1000)
    }

    /// `per_unit` of the result for each unit of the script's number, a level: a tenth of a
    /// point a second is `10`. Fails the build for a number that does not come out whole.
    const fn lv_per(&self, key: &str, per_unit: i32) -> Lv {
        let levels = self.value(key);
        let mut out = [0; 4];
        let mut level = 0;
        while level < 4 {
            let at = if level < levels.len() {
                level
            } else {
                levels.len() - 1
            };
            let scaled = levels[at].0 as i64 * per_unit as i64;
            assert!(
                scaled % 1000 == 0,
                "a number that is not whole in the unit asked"
            );
            out[level] = (scaled / 1000) as i32;
            level += 1;
        }
        out
    }

    /// The numbers of `key` in the row, if it has them: the one key a row may lack by design.
    const fn find(&self, key: &str) -> Option<&'static [Dec]> {
        let mut at = 0;
        while at < self.values.len() {
            if crate::item_data::str_eq(self.values[at].0, key) {
                return Some(self.values[at].1);
            }
            at += 1;
        }
        None
    }

    /// The hesitation the script gives a neutral before this cast, if it gives one.
    const fn chance(&self) -> Option<CastChance> {
        match (
            self.find("neutral_chance_to_cast"),
            self.find("neutral_cast_attempt_cooldown"),
        ) {
            (Some(pct), Some(wait)) => Some(CastChance {
                pct: pct[0].int(),
                wait_ms: wait[0].0,
            }),
            (None, None) => None,
            _ => panic!("a script gives a cast chance without its wait, or the wait without it"),
        }
    }

    /// Whether the script says `IsBreakable`: a Break silences the ability.
    const fn breakable(&self) -> bool {
        self.first("IsBreakable").0 == 1000
    }
}

/// `value`, which the build refuses unless it is above zero (a rate a slow falls by).
const fn positive(value: i32) -> i32 {
    assert!(value > 0, "a rate that is not above zero");
    value
}

/// `100` less each level's number.
const fn complement(values: Lv) -> Lv {
    lv(
        100 - values[0],
        100 - values[1],
        100 - values[2],
        100 - values[3],
    )
}

/// Each level's number, its sign reversed.
const fn negated(values: Lv) -> Lv {
    lv(-values[0], -values[1], -values[2], -values[3])
}

/// The whole numbers of a key of the row, one for each level it lists: a slice for an
/// [`AbilityDef`].
macro_rules! whole_levels {
    ($data:expr, $key:literal) => {{
        const LEVELS: &[Dec] = $data.value($key);
        &whole_array::<{ LEVELS.len() }>(LEVELS)
    }};
}

/// The cooldowns of a key of the row in ticks, one for each level it lists.
macro_rules! tick_levels {
    ($data:expr, $key:literal) => {{
        const LEVELS: &[Dec] = $data.value($key);
        &tick_array::<{ LEVELS.len() }>(LEVELS)
    }};
}

const fn whole_array<const N: usize>(levels: &[Dec]) -> [i32; N] {
    let mut out = [0; N];
    let mut level = 0;
    while level < N {
        out[level] = levels[level].int();
        level += 1;
    }
    out
}

const fn tick_array<const N: usize>(levels: &[Dec]) -> [u32; N] {
    let mut out = [0; N];
    let mut level = 0;
    while level < N {
        out[level] = levels[level].secs().ticks();
        level += 1;
    }
    out
}

/// An ability that is cast, as its script has it: its mana and cooldown a level, its cast range
/// (that of the fourth argument when given, else its `AbilityCastRange`: a no-target cast has none,
/// and one that reaches by a value of its own names it), cast point, and the backswing the scripts
/// do not hold. The range is that of the first level: `AbilityDef` has one range, not one a level.
macro_rules! cast_of {
    ($data:expr, $aim:expr, $enemy:expr) => {
        cast_of!($data, $aim, $enemy, $data.int("AbilityCastRange"))
    };
    ($data:expr, $aim:expr, $enemy:expr, $range:expr) => {
        cast(
            $aim,
            $enemy,
            whole_levels!($data, "AbilityManaCost"),
            tick_levels!($data, "AbilityCooldown"),
            $range,
            $data.ms("AbilityCastPoint") as u32,
            $data.ms("AbilityCastBackswing") as u32,
        )
    };
}

const fn count(value: i32) -> u8 {
    assert!(
        value >= 0 && value <= u8::MAX as i32,
        "neutral cast count is out of range"
    );
    value as u8
}

/// An entry over the row of its script, `D`.
macro_rules! with {
    ($data:path, $entry:expr) => {{
        const D: &ScriptData = &$data;
        $entry
    }};
}

use NeutralAbility as N;

const RESERVED_89: ScriptData = ScriptData {
    key: "bota_kobold_disarm_passive",
    values: &[],
};

/// Every neutral ability, in the order of [`NeutralAbility`].
pub const NEUTRAL_ABILITIES: &[NeutralAbilityDef] = &[
    with!(
        script::KOBOLD_TUNNELER_PROSPECTING,
        entry(AbilityId(17), D, N::ProspectingAura, passive(), Spell::Idle,)
    ),
    with!(
        script::NEUTRAL_UPGRADE,
        entry(
            AbilityId(18),
            D,
            N::NeutralUpgrade,
            passive(),
            Spell::Upgrade
        )
    ),
    with!(
        script::RETIRED_KOBOLD_DISARM,
        entry(
            AbilityId(19),
            D,
            N::StealWeapon,
            passive_cooling(tick_levels!(D, "AbilityCooldown")),
            Spell::Strike(Strike::Disarm {
                hits: D.int("hits") as u8,
                ms: D.lv_ms("duration"),
                counter: EffectId(25),
            }),
        )
    ),
    with!(
        script::KOBOLD_TASKMASTER_SPEED_AURA,
        entry(
            AbilityId(20),
            D,
            N::SpeedAura,
            passive(),
            allies(D, Buff::of(Stat::MovePct, D.lv("bonus_movement_speed")), 26),
        )
    ),
    // Retired by Dota 7.42, frozen at its last live values (the importer's retired table): the
    // key `berserker_troll_break` names another ability in the new scripts.
    with!(
        script::RETIRED_BERSERKER_TROLL_BREAK,
        entry(
            AbilityId(21),
            D,
            N::Break,
            passive_cooling(tick_levels!(D, "AbilityCooldown")),
            Spell::Strike(Strike::Break {
                ms: D.ms("duration")
            }),
        )
    ),
    with!(
        script::CREEP_PIERCING,
        entry(AbilityId(22), D, N::Piercing, passive(), Spell::Piercing)
    ),
    with!(
        script::FOREST_TROLL_HIGH_PRIEST_HEAL,
        entry(
            AbilityId(23),
            D,
            N::Heal,
            cast_of!(D, Aim::Unit, false),
            Spell::Uncast,
        )
    ),
    with!(
        script::FOREST_TROLL_HIGH_PRIEST_HEAL_AMP_AURA,
        entry(
            AbilityId(24),
            D,
            N::HealAmplificationAura,
            passive(),
            Spell::Idle,
        )
    ),
    with!(
        script::GNOLL_ASSASSIN_ENVENOMED_WEAPON,
        entry(
            AbilityId(25),
            D,
            N::EnvenomedWeapon,
            passive(),
            Spell::Strike(Strike::Lay {
                buff: Buff::of(Stat::RegenCutPct, D.lv("regen_reduction")),
                effect: EffectId(27),
                hero_ms: D.ms("hero_duration"),
                other_ms: D.ms("non_hero_duration"),
                dps: D.lv("damage_per_second"),
            }),
        )
    ),
    with!(
        script::FEL_BEAST_HAUNT,
        entry(
            AbilityId(26),
            D,
            N::Vex,
            cast_of!(D, Aim::Unit, true),
            Spell::Uncast,
        )
    ),
    with!(
        script::GHOST_FROST_ATTACK,
        entry(
            AbilityId(27),
            D,
            N::FrostAttack,
            passive(),
            Spell::Strike(Strike::Lay {
                buff: chill(D.lv("movespeed_slow"), D.lv("attackspeed_slow")),
                effect: EffectId(28),
                hero_ms: D.ms("duration"),
                other_ms: D.ms("duration"),
                dps: all(0),
            }),
        )
    ),
    with!(
        script::HARPY_SCOUT_TAKE_OFF,
        entry(
            AbilityId(28),
            D,
            N::TakeOff,
            cast_of!(D, Aim::Own, false),
            Spell::Uncast,
        )
    ),
    with!(
        script::HARPY_STORM_CHAIN_LIGHTNING,
        entry(
            AbilityId(29),
            D,
            N::ChainLightning,
            cast_of!(D, Aim::Unit, true),
            Spell::Cast {
                act: Act::Chain {
                    damage: D.lv("initial_damage"),
                    targets: D.int("max_targets") as u8,
                    radius: D.int("jump_range"),
                    delay_ms: D.ms("jump_delay"),
                    keep_pct: complement(D.lv("damage_percent_loss")),
                },
                when: When::Think(CastRule {
                    among: Among::Foes(Within::CastRange),
                    needs: &[],
                    marks: &[Mark::HpBelow { hp: 330 }, Mark::AttacksCaster],
                    pick: Pick::LeastHp,
                }),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::MUDGOLEM_CLOAK_AURA,
        entry(
            AbilityId(30),
            D,
            N::CloakAura,
            passive(),
            allies(
                D,
                Buff::two(
                    Stat::MagicResistPct,
                    D.lv("bonus_magical_armor"),
                    Stat::MagicResistCreepPct,
                    D.lv("bonus_magical_armor_creeps"),
                ),
                29,
            ),
        )
    ),
    with!(
        script::CENTAUR_KHAN_WAR_STOMP,
        entry(
            AbilityId(31),
            D,
            N::WarStomp,
            cast_of!(D, Aim::Own, false),
            Spell::Cast {
                act: Act::Stomp {
                    growth: None,
                    other_pct: 100,
                    radius: D.lv("radius"),
                    damage: D.lv("AbilityDamage"),
                    kind: DamageKind::Magical,
                    hero_ms: D.lv_ms("hero_stun_duration"),
                    other_ms: D.lv_ms("non_hero_stun_duration"),
                    then: Then::Stun,
                },
                when: When::Think(CastRule::on_itself(&[Need::Crowd {
                    radius: 250,
                    at_least: 3,
                    heroes: 0,
                    on_it: false,
                }])),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::GIANT_WOLF_INTIMIDATE,
        entry(
            AbilityId(32),
            D,
            N::Intimidate,
            cast_of!(D, Aim::Own, false),
            Spell::Cast {
                act: Act::Stomp {
                    growth: None,
                    other_pct: 100,
                    radius: D.lv("radius"),
                    damage: all(0),
                    kind: DamageKind::Magical,
                    hero_ms: D.lv_ms("duration"),
                    other_ms: D.lv_ms("duration"),
                    then: Then::Wear {
                        buff: Buff::of(Stat::DamagePct, negated(D.lv("damage_reduction"))),
                        effect: EffectId(30),
                    },
                },
                when: When::Think(CastRule {
                    among: Among::Foes(Within::Radius(300)),
                    needs: &[Need::HurtBelow { pct: 51 }],
                    marks: &[],
                    pick: Pick::Nearest,
                }),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::ALPHA_WOLF_CRITICAL_STRIKE,
        entry(
            AbilityId(33),
            D,
            N::CriticalStrike,
            passive(),
            Spell::Strike(Strike::Crit {
                chance_pct: D.int("crit_chance"),
                mult_pct: D.lv("crit_mult"),
                magic: false,
                targets: ProcTargets::Any,
            }),
        )
    ),
    with!(
        script::ALPHA_WOLF_COMMAND_AURA,
        entry(
            AbilityId(34),
            D,
            N::PackleadersAura,
            passive(),
            allies(D, Buff::of(Stat::DamagePct, D.lv("bonus_damage_pct")), 31),
        )
    ),
    with!(
        script::SATYR_TRICKSTER_PURGE,
        entry(
            AbilityId(35),
            D,
            N::Purge,
            cast_of!(D, Aim::Unit, false),
            Spell::Cast {
                act: Act::Purge {
                    start_pct: D.int("base_slow"),
                    rate: positive(D.int("purge_rate")),
                    ms: D.ms("duration"),
                },
                when: When::Think(CastRule {
                    among: Among::Foes(Within::Radius(500)),
                    needs: &[],
                    marks: &[Mark::Hero, Mark::Buffed, Mark::InCastRange],
                    pick: Pick::Nearest,
                }),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::SATYR_SOULSTEALER_MANA_BURN,
        entry(
            AbilityId(36),
            D,
            N::ManaBurn,
            cast_of!(D, Aim::Unit, true),
            Spell::Uncast,
        )
    ),
    with!(
        script::FOREST_TROLL_HIGH_PRIEST_MANA_AURA,
        entry(
            AbilityId(37),
            D,
            N::ManaAura,
            passive(),
            allies(
                D,
                Buff::of(Stat::ManaRegen, D.lv_per("mana_regen", 100)),
                32
            ),
        )
    ),
    with!(
        script::SATYR_HELLCALLER_SHOCKWAVE,
        entry(
            AbilityId(38),
            D,
            N::Shockwave,
            cast_of!(D, Aim::Point, true),
            Spell::Uncast,
        )
    ),
    with!(
        script::SATYR_HELLCALLER_UNHOLY_AURA,
        entry(
            AbilityId(39),
            D,
            N::UnholyAura,
            passive(),
            allies(D, Buff::of(Stat::HpRegen, D.lv_per("health_regen", 10)), 33),
        )
    ),
    with!(
        script::OGRE_BRUISER_OGRE_SMASH,
        entry(
            AbilityId(40),
            D,
            N::OgreSmash,
            cast_of!(D, Aim::Point, true),
            Spell::Cast {
                act: Act::Smash {
                    reach: D.int("reach"),
                    radius: D.lv("radius"),
                    damage: D.lv("damage"),
                    current_pct: D.int("damage_pct"),
                    ms: D.ms("hero_stun_duration"),
                },
                when: When::Think(CastRule {
                    among: Among::Foes(Within::Radius(400)),
                    needs: &[Need::HurtBelow { pct: 70 }],
                    marks: &[],
                    pick: Pick::Nearest,
                }),
                shared: hold(OGRE_SMASH),
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::OGRE_MAGI_FROST_ARMOR,
        entry(
            AbilityId(41),
            D,
            N::IceArmor,
            cast_of!(D, Aim::Unit, false),
            Spell::Uncast,
        )
    ),
    with!(
        script::MUD_GOLEM_HURL_BOULDER,
        entry(
            AbilityId(42),
            D,
            N::HurlBoulder,
            cast_of!(D, Aim::Unit, true),
            Spell::Cast {
                act: Act::Bolt {
                    speed: D.int("speed"),
                    hero_damage: D.int("damage"),
                    other_damage: D.int("damage_creeps"),
                    ms: D.ms("duration"),
                    root: false,
                },
                when: When::Think(CastRule {
                    among: Among::Foes(Within::CastRange),
                    needs: &[],
                    marks: &[Mark::HpBelow { hp: 300 }],
                    pick: Pick::LeastHp,
                }),
                shared: hold(GOLEM_BOULDER),
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::MUD_GOLEM_ROCK_DESTROY,
        entry(
            AbilityId(43),
            D,
            N::ShardSplit,
            passive(),
            Spell::Throe(Throe::Split {
                kind: NeutralKind::MudGolemSplit,
                count: D.lv("shard_amount"),
                hp: D.lv("shard_health"),
                damage: D.lv("shard_damage"),
                life_ms: D.ms("shard_duration"),
                radius: D.int("radius"),
                slow_pct: -D.int("movespeed"),
                slow_ms: D.ms("duration"),
                effect: EffectId(34),
            }),
        )
    ),
    with!(
        script::CENTAUR_KHAN_ENDURANCE_AURA,
        entry(
            AbilityId(44),
            D,
            N::SwiftnessAura,
            passive(),
            allies(
                D,
                Buff::of(Stat::AttackSpeed, D.lv("bonus_attack_speed")),
                35
            ),
        )
    ),
    with!(
        script::FURBOLG_ENRAGE_ATTACK_SPEED,
        entry(
            AbilityId(45),
            D,
            N::DeathThroeRush,
            passive(),
            Spell::Throe(Throe::Rouse {
                radius: D.int("radius"),
                ms: D.ms("duration"),
                buff: Buff::of(Stat::AttackSpeed, D.lv("bonus_aspd")),
                effect: EffectId(36),
            }),
        )
    ),
    with!(
        script::POLAR_FURBOLG_URSA_WARRIOR_THUNDER_CLAP,
        entry(
            AbilityId(46),
            D,
            N::ThunderClap,
            cast_of!(D, Aim::Own, false),
            Spell::Cast {
                act: Act::Stomp {
                    growth: None,
                    other_pct: 100,
                    radius: D.lv("radius"),
                    damage: D.lv("AbilityDamage"),
                    kind: DamageKind::Magical,
                    hero_ms: D.lv_ms("duration"),
                    other_ms: D.lv_ms("duration"),
                    then: Then::Wear {
                        buff: chill(D.lv("movespeed_slow"), D.lv("attackspeed_slow")),
                        effect: EffectId(37),
                    },
                },
                when: When::Think(CastRule::on_itself(&[Need::Crowd {
                    radius: 300,
                    at_least: 2,
                    heroes: 0,
                    on_it: false,
                }])),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::FURBOLG_ENRAGE_DAMAGE,
        entry(
            AbilityId(47),
            D,
            N::DeathThroePower,
            passive(),
            Spell::Throe(Throe::Rouse {
                radius: D.int("radius"),
                ms: D.ms("duration"),
                buff: Buff::of(Stat::DamagePct, D.lv("bonus_dmg_pct")),
                effect: EffectId(38),
            }),
        )
    ),
    with!(
        script::ENRAGED_WILDKIN_TORNADO,
        entry(
            AbilityId(48),
            D,
            N::Tornado,
            cast_of!(D, Aim::Point, true),
            Spell::Cast {
                act: Act::Tornado {
                    kind: NeutralKind::WildkinTornado,
                    life_ms: D.ms("duration"),
                },
                when: When::MateFalls {
                    kind: NeutralKind::EnragedWildkin,
                    chance_pct: D.int("neutral_chance_to_cast"),
                },
                shared: None,
                chance: None,
            },
        )
    ),
    with!(
        script::ENRAGED_WILDKIN_HURRICANE,
        entry(
            AbilityId(49),
            D,
            N::Hurricane,
            cast_of!(D, Aim::Unit, true),
            Spell::Cast {
                act: Act::Push {
                    distance: D.int("distance"),
                    speed: D.int("speed"),
                },
                when: When::Think(CastRule::on_held(&[
                    Mark::Hero,
                    Mark::InCastRange,
                    Mark::Chasing { radius: 1200 },
                ])),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::ENRAGED_WILDKIN_TOUGHNESS_AURA,
        entry(
            AbilityId(50),
            D,
            N::ToughnessAura,
            passive(),
            allies(D, Buff::of(Stat::Armor, D.lv("bonus_armor")), 39),
        )
    ),
    with!(
        script::DARK_TROLL_WARLORD_ENSNARE,
        entry(
            AbilityId(51),
            D,
            N::Ensnare,
            cast_of!(D, Aim::Unit, true),
            Spell::Cast {
                act: Act::Bolt {
                    speed: D.int("net_speed"),
                    hero_damage: 0,
                    other_damage: 0,
                    ms: D.ms("duration"),
                    root: true,
                },
                when: When::Think(CastRule::on_held(&[
                    Mark::Hero,
                    Mark::InCastRange,
                    Mark::RivalNear { radius: 1200 },
                ])),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::DARK_TROLL_WARLORD_RAISE_DEAD,
        entry(
            AbilityId(52),
            D,
            N::RaiseDead,
            cast_of!(D, Aim::Own, false),
            Spell::Cast {
                act: Act::Summon {
                    kind: NeutralKind::SkeletonWarrior,
                    count: D.int("skeletons_created") as u8,
                    hp: D.lv("skeletons_health"),
                    damage: D.lv("skeletons_damage"),
                    life_ms: D.ms("duration"),
                },
                when: When::Think(CastRule {
                    among: Among::Held,
                    needs: &[Need::HurtBelow { pct: 50 }],
                    marks: &[],
                    pick: Pick::First,
                }),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::RETIRED_WARPINE_RAIDER_SEED_SHOT,
        entry(
            AbilityId(53),
            D,
            N::SeedShot,
            cast_of!(D, Aim::Unit, true),
            Spell::Uncast
        )
    ),
    with!(
        script::BLACK_DRAKE_MAGIC_AMPLIFICATION_AURA,
        entry(
            AbilityId(54),
            D,
            N::MagicAmplificationAura,
            passive(),
            Spell::Aura(AuraSpell {
                whom: Whom::Enemies,
                radius: D.int("radius"),
                buff: Buff::of(Stat::MagicTakenPct, D.lv("spell_amp")),
                effect: EffectId(40),
                stacks: false,
                water: false,
                breakable: D.breakable(),
            }),
        )
    ),
    with!(
        script::RETIRED_BLACK_DRAGON_FIREBALL,
        entry(
            AbilityId(55),
            D,
            N::Fireball,
            cast_of!(D, Aim::Point, true),
            Spell::Uncast
        )
    ),
    with!(
        script::BLACK_DRAGON_SPLASH_ATTACK,
        entry(
            AbilityId(56),
            D,
            N::SplashAttack,
            passive(),
            Spell::Strike(Strike::Splash {
                radius: D.int("range"),
                pct: D.int("damage_percent"),
            }),
        )
    ),
    with!(
        script::BLACK_DRAGON_DRAGONHIDE_AURA,
        entry(
            AbilityId(57),
            D,
            N::DragonhideAura,
            passive(),
            Spell::Aura(AuraSpell {
                whom: Whom::Allies,
                radius: D.int("radius"),
                buff: Buff::of(Stat::Armor, D.lv("bonus_armor")),
                effect: EffectId(41),
                stacks: true,
                water: false,
                breakable: D.breakable(),
            }),
        )
    ),
    with!(
        script::ANCIENT_ROCK_GOLEM_WEAKENING_AURA,
        entry(
            AbilityId(58),
            D,
            N::WeakeningAura,
            passive(),
            Spell::Aura(AuraSpell {
                whom: Whom::Enemies,
                radius: D.int("radius"),
                buff: Buff::of(Stat::Armor, negated(D.lv("armor_reduction"))),
                effect: EffectId(42),
                stacks: false,
                water: false,
                breakable: D.breakable(),
            }),
        )
    ),
    with!(
        script::GRANITE_GOLEM_HP_AURA,
        entry(
            AbilityId(59),
            D,
            N::GraniteAura,
            passive(),
            Spell::Aura(AuraSpell::allies(
                D,
                Buff::of(Stat::MaxHpPct, D.lv("bonus_hp")),
                43
            )),
        )
    ),
    with!(
        script::BIG_THUNDER_LIZARD_WARDRUMS_AURA,
        entry(
            AbilityId(60),
            D,
            N::WarDrumsAura,
            passive(),
            allies(
                D,
                Buff::two(
                    Stat::AttackSpeed,
                    D.lv("speed_bonus"),
                    Stat::AccuracyPct,
                    D.lv("accuracy"),
                ),
                44,
            ),
        )
    ),
    with!(
        script::BIG_THUNDER_LIZARD_SLAM,
        entry(
            AbilityId(61),
            D,
            N::Slam,
            cast_of!(D, Aim::Own, false),
            Spell::Cast {
                act: Act::Stomp {
                    growth: None,
                    other_pct: 100,
                    radius: D.lv("radius"),
                    damage: D.lv("AbilityDamage"),
                    kind: DamageKind::Magical,
                    hero_ms: D.lv_ms("hero_duration"),
                    other_ms: D.lv_ms("non_hero_duration"),
                    then: Then::Wear {
                        buff: chill(D.lv("movespeed_slow"), D.lv("attack_slow_tooltip")),
                        effect: EffectId(45),
                    },
                },
                when: When::Think(CastRule::on_itself(&[
                    Need::Swinging,
                    Need::Crowd {
                        radius: 350,
                        at_least: 3,
                        heroes: 0,
                        on_it: true,
                    },
                ])),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::BIG_THUNDER_LIZARD_FRENZY,
        entry(
            AbilityId(62),
            D,
            N::Frenzy,
            cast_of!(D, Aim::Unit, false),
            Spell::Cast {
                act: Act::Enrage {
                    buff: Buff::of(Stat::AttackSpeed, D.lv("attackspeed_bonus")),
                    effect: EffectId(46),
                    ms: D.ms("duration"),
                },
                when: When::Think(CastRule::on_itself(&[Need::Fighting])),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::FROSTBITTEN_GOLEM_TIME_WARP_AURA,
        entry(
            AbilityId(63),
            D,
            N::TimeWarpAura,
            passive(),
            allies(D, Buff::of(Stat::CooldownPct, D.lv("bonus_cdr")), 47),
        )
    ),
    with!(
        script::ICE_SHAMAN_INCENDIARY_BOMB,
        entry(
            AbilityId(64),
            D,
            N::IcefireBomb,
            cast_of!(D, Aim::Unit, true),
            Spell::Uncast,
        )
    ),
    with!(
        script::SPAWNLORD_AURA,
        entry(
            AbilityId(65),
            D,
            N::ProwlerAura,
            passive(),
            allies(
                D,
                Buff::two(
                    Stat::LifestealPct,
                    D.lv("lifesteal"),
                    Stat::HpRegen,
                    D.lv_per("hp_regen", 10),
                ),
                48,
            ),
        )
    ),
    with!(
        script::SPAWNLORD_MASTER_STOMP,
        entry(
            AbilityId(66),
            D,
            N::Desecrate,
            cast_of!(D, Aim::Own, false),
            Spell::Cast {
                act: Act::Stomp {
                    growth: None,
                    other_pct: 100,
                    radius: D.lv("radius"),
                    damage: D.lv("damage"),
                    kind: DamageKind::Physical,
                    hero_ms: D.lv_ms("duration"),
                    other_ms: D.lv_ms("duration"),
                    then: Then::Wear {
                        buff: Buff::of(Stat::BaseArmorPct, negated(D.lv("armor_reduction_pct"))),
                        effect: EffectId(49),
                    },
                },
                when: When::Think(CastRule::on_itself(&[Need::Crowd {
                    radius: 300,
                    at_least: 3,
                    heroes: 0,
                    on_it: false,
                }])),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::SPAWNLORD_MASTER_FREEZE,
        entry(
            AbilityId(67),
            D,
            N::Petrify,
            passive_cooling(tick_levels!(D, "AbilityCooldown")),
            Spell::Strike(Strike::Petrify {
                below_pct: D.int("health_threshold_pct"),
                ms: D.lv_ms("duration"),
                dps: D.int("damage"),
                effect: EffectId(50),
            }),
        )
    ),
    with!(
        script::FROGMEN_RIVERBORN_AURA,
        entry(
            AbilityId(68),
            D,
            N::RiverbornAura,
            passive(),
            Spell::Aura(AuraSpell {
                whom: Whom::Allies,
                radius: D.int("radius"),
                buff: Buff::two(
                    Stat::MovePct,
                    D.lv("bonus_movement_speed"),
                    Stat::OutgoingPct,
                    D.lv("bonus_outgoing_damage"),
                ),
                effect: EffectId(51),
                stacks: false,
                water: true,
                breakable: D.breakable(),
            }),
        )
    ),
    with!(
        script::FROGMEN_ARM_OF_THE_DEEP,
        entry(
            AbilityId(69),
            D,
            N::ArmOfTheDeep,
            cast_of!(D, Aim::Point, true),
            Spell::Cast {
                act: Act::Spikes {
                    lines: 1,
                    spread_deg: 0,
                    length: D.int("range"),
                    width: D.int("projectile_width"),
                    damage: D.lv("damage"),
                    afflicts: Afflicts::Stun,
                    ms: D.lv_ms("duration"),
                },
                when: When::Think(CastRule::on_held(&[Mark::InCastRange])),
                shared: hold(FROG_SPIKES),
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::FROGMEN_TENDRILS_OF_THE_DEEP,
        entry(
            AbilityId(70),
            D,
            N::TendrilsOfTheDeep,
            cast_of!(D, Aim::Point, true),
            Spell::Cast {
                act: Act::Spikes {
                    lines: 2,
                    spread_deg: 2 * D.int("yaw_offset"),
                    length: D.int("range"),
                    width: D.int("projectile_width"),
                    damage: D.lv("damage"),
                    afflicts: Afflicts::Stun,
                    ms: D.lv_ms("duration"),
                },
                when: When::Think(CastRule::on_held(&[Mark::InCastRange])),
                shared: hold(FROG_SPIKES),
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::FROGMEN_CONGREGATION_OF_THE_DEEP,
        entry(
            AbilityId(71),
            D,
            N::CongregationsOfTheDeep,
            cast_of!(D, Aim::Own, false, D.int("range")),
            Spell::Cast {
                act: Act::Spikes {
                    lines: D.int("projectile_count") as u8,
                    spread_deg: D.int("spread_angle"),
                    length: D.int("range"),
                    width: D.int("projectile_width"),
                    damage: D.lv("damage"),
                    afflicts: Afflicts::Stun,
                    ms: D.lv_ms("duration"),
                },
                when: When::Think(CastRule::on_held(&[Mark::InCastRange])),
                shared: hold(FROG_SPIKES),
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::FROGMEN_WATER_BUBBLE_SMALL,
        entry(
            AbilityId(72),
            D,
            N::WaterBubbleSmall,
            cast_of!(D, Aim::Unit, false),
            Spell::Cast {
                act: Act::Bubble {
                    absorb: D.lv("damage_absorb"),
                    ms: D.ms("duration"),
                    heal_pct: D.int("heal_pct"),
                    radius: D.int("radius"),
                },
                when: BUBBLE_WHEN,
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::FROGMEN_WATER_BUBBLE_MEDIUM,
        entry(
            AbilityId(73),
            D,
            N::WaterBubbleMedium,
            cast_of!(D, Aim::Unit, false),
            Spell::Cast {
                act: Act::Bubble {
                    absorb: D.lv("damage_absorb"),
                    ms: D.ms("duration"),
                    heal_pct: D.int("heal_pct"),
                    radius: D.int("radius"),
                },
                when: BUBBLE_WHEN,
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::FROGMEN_WATER_BUBBLE_LARGE,
        entry(
            AbilityId(74),
            D,
            N::WaterBubbleLarge,
            cast_of!(D, Aim::Unit, false),
            Spell::Cast {
                act: Act::Bubble {
                    absorb: D.lv("damage_absorb"),
                    ms: D.ms("duration"),
                    heal_pct: D.int("heal_pct"),
                    radius: D.int("radius"),
                },
                when: BUBBLE_WHEN,
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::HILL_TROLL_RALLY,
        entry(
            AbilityId(75),
            D,
            N::Rally,
            passive(),
            Spell::Aura(AuraSpell {
                whom: Whom::Allies,
                radius: D.int("radius"),
                buff: Buff::of(Stat::Damage, D.lv("damage_bonus")),
                effect: EffectId(52),
                stacks: true,
                water: false,
                breakable: D.breakable(),
            }),
        )
    ),
    with!(
        script::TORNADO_TEMPEST,
        entry(
            AbilityId(76),
            D,
            N::Tempest,
            passive(),
            Spell::Storm {
                radius: D.int("far_radius"),
                near_radius: D.int("near_radius"),
                near_dps: D.int("near_damage"),
                far_dps: D.int("far_damage"),
                period_ms: D.ms("tick_rate"),
                buff: chill(D.lv("movespeed_slow"), D.lv("attackspeed_slow")),
                effect: EffectId(53),
                breakable: D.breakable(),
            },
        )
    ),
    entry(
        AbilityId(80),
        &crate::roshan_data::ROSHAN_INHERENT_BUFFS,
        NeutralAbility::StrengthOfTheImmortal,
        AbilityDef {
            max_level: 1,
            hidden: true,
            pierces: crate::Pierces::Yes,
            ..passive()
        },
        Spell::Intrinsic {
            moving: Some(&crate::ROSHAN_MIGRATION),
            buff: Buff::of(Stat::IllusionAttackReductionPct, all(80)),
            effect: EffectId(83),
            aura: None,
        },
    ),
    entry(
        AbilityId(81),
        &crate::roshan_data::ROSHAN_DEVOTION,
        NeutralAbility::Devotion,
        AbilityDef {
            max_level: 1,
            hidden: true,
            ..passive()
        },
        Spell::Intrinsic {
            moving: None,
            buff: Buff::of(
                Stat::SlowResistPct,
                all(crate::roshan_data::ROSHAN_DEVOTION
                    .first("slow_resistance")
                    .int()),
            ),
            effect: EffectId(84),
            aura: Some(crate::ROSHAN_ARMOR_AURA),
        },
    ),
    entry(
        AbilityId(82),
        &crate::roshan_data::ROSHAN_SPELL_BLOCK,
        NeutralAbility::SpellBlock,
        AbilityDef {
            max_level: 1,
            cooldown: &[crate::roshan_data::ROSHAN_SPELL_BLOCK
                .first("AbilityCooldown")
                .secs()
                .ticks()],
            ..passive()
        },
        Spell::Block {
            buff: Buff::of(
                Stat::StatusResistPct,
                all(crate::roshan_data::ROSHAN_SPELL_BLOCK
                    .first("status_resistance")
                    .int()),
            ),
            effect: EffectId(85),
            breakable: false,
        },
    ),
    entry(
        AbilityId(83),
        &crate::roshan_data::ROSHAN_BASH,
        N::Bash,
        AbilityDef {
            max_level: 1,
            pierces: crate::Pierces::Yes,
            ..passive()
        },
        Spell::Strike(Strike::Bash {
            chance_pct: crate::roshan_data::ROSHAN_BASH.first("bash_chance").int(),
            damage: all(crate::roshan_data::ROSHAN_BASH.first("bonus_damage").int()),
            stun_ms: all(crate::roshan_data::ROSHAN_BASH.first("stun_duration").0),
            effect: EffectId::BASHED,
            rolls_at: ProcTargets::NoWards,
            lands_on: ProcTargets::NoWards,
        }),
    ),
    entry(
        AbilityId(84),
        &crate::roshan_data::ROSHAN_SLAM,
        N::RoshanSlam,
        AbilityDef {
            max_level: 1,
            ..cast(
                Aim::Own,
                false,
                &[0],
                &[crate::roshan_data::ROSHAN_SLAM
                    .first("AbilityCooldown")
                    .secs()
                    .ticks()],
                0,
                crate::roshan_data::ROSHAN_SLAM.first("AbilityCastPoint").0 as u32,
                570,
            )
        },
        Spell::Cast {
            act: Act::Stomp {
                radius: all(crate::roshan_data::ROSHAN_SLAM.first("radius").int()),
                damage: all(crate::roshan_data::ROSHAN_SLAM.first("damage").int()),
                growth: Some(crate::DamageGrowth {
                    per_step: all(crate::roshan_data::ROSHAN_SLAM.first("damage_growth").int()),
                    clock: crate::GrowthClock {
                        first_after_horn: 60 * rules::TICKS_PER_SECOND,
                        period: 60 * rules::TICKS_PER_SECOND,
                        cap: u32::MAX,
                    },
                }),
                other_pct: 100
                    + crate::roshan_data::ROSHAN_SLAM
                        .first("damage_bonus_creeps")
                        .int(),
                kind: DamageKind::Magical,
                hero_ms: all(crate::roshan_data::ROSHAN_SLAM.first("slow_duration").0),
                other_ms: all(crate::roshan_data::ROSHAN_SLAM.first("slow_duration").0),
                then: Then::Wear {
                    buff: chill(
                        all(-crate::roshan_data::ROSHAN_SLAM.first("slow_amount").int()),
                        all(-crate::roshan_data::ROSHAN_SLAM.first("slow_amount").int()),
                    ),
                    effect: EffectId(86),
                },
            },
            when: When::Think(CastRule::on_itself(&[Need::Crowd {
                radius: 350,
                at_least: 3,
                heroes: 1,
                on_it: false,
            }])),
            shared: None,
            chance: None,
        },
    ),
    entry(
        AbilityId(85),
        &crate::roshan_data::ROSHAN_REVENGEROAR,
        N::RevengeRoar,
        AbilityDef {
            max_level: 1,
            immediate: true,
            cast_while_stunned: true,
            cast_while_silenced: true,
            ..cast(
                Aim::Own,
                false,
                &[0],
                &[crate::roshan_data::ROSHAN_REVENGEROAR
                    .first("AbilityCooldown")
                    .secs()
                    .ticks()],
                0,
                0,
                crate::roshan_data::ROSHAN_REVENGEROAR.ms("AbilityCastBackswing") as u32,
            )
        },
        Spell::Cast {
            when: When::Think(CastRule::on_itself(&[
                Need::HurtBelow { pct: 85 },
                Need::PreviousKillerHero { radius: 800 },
            ])),
            act: Act::Roar(&ROSHAN_ROAR),
            shared: None,
            chance: None,
        },
    ),
    entry(
        AbilityId(86),
        &crate::roshan_data::ROSHAN_GRAB,
        N::GrabAndThrow,
        AbilityDef {
            max_level: 1,
            pierces: crate::Pierces::Yes,
            block: crate::BlockTiming::Cast,
            duration: crate::roshan_data::ROSHAN_GRAB
                .first("channel_time")
                .secs()
                .ticks(),
            targets: crate::TargetFilter::of(&[bota_proto::UnitKind::Hero], false),
            ..cast(
                Aim::Unit,
                true,
                &[0],
                &[crate::roshan_data::ROSHAN_GRAB
                    .first("AbilityCooldown")
                    .secs()
                    .ticks()],
                crate::roshan_data::ROSHAN_GRAB
                    .first("AbilityCastRange")
                    .int(),
                crate::roshan_data::ROSHAN_GRAB.first("AbilityCastPoint").0 as u32,
                530,
            )
        },
        Spell::Cast {
            when: When::Think(CastRule {
                among: Among::Foes(Within::Radius(
                    crate::roshan_data::ROSHAN_GRAB.first("cast_range").int(),
                )),
                needs: &[Need::Migrating],
                marks: &[Mark::RealHero],
                pick: Pick::Nearest,
            }),
            act: Act::Grab(&ROSHAN_THROW),
            shared: None,
            chance: None,
        },
    ),
    with!(
        script::MUD_GOLEM_HURL_BIG_BOULDER,
        entry(
            AbilityId(88),
            D,
            N::HurlBigBoulder,
            cast_of!(D, Aim::Point, true),
            Spell::Cast {
                act: Act::Boulder {
                    min_ms: D.ms("min_travel_time"),
                    max_ms: D.ms("max_travel_time"),
                    radius: D.int("impact_radius"),
                    damage: D.lv("base_damage"),
                    kind: DamageKind::Physical,
                    ms: D.lv_ms("stun_duration"),
                },
                when: When::Think(CastRule::on_held(&[Mark::InCastRange])),
                shared: hold(GOLEM_BOULDER),
                chance: D.chance(),
            },
        )
    ),
    entry(
        AbilityId(89),
        &RESERVED_89,
        N::Reserved89,
        AbilityDef {
            retired: true,
            ..passive()
        },
        Spell::Idle,
    ),
    // Dota 7.42's Radigull. Present in the data, not cast: Eggstremism lays eggs (units of one pip
    // of health that a blow destroys) which explode after a fuse; bota has no such unit yet.
    with!(
        script::RADIGULL_EGGSTREMISM,
        entry(
            AbilityId(90),
            D,
            N::Eggstremism,
            cast_of!(D, Aim::Unit, true),
            Spell::Uncast,
        )
    ),
    // Odobenus Blight: attacks reduce the target's armor for a while.
    with!(
        script::ODOBENUS_BLIGHT,
        entry(
            AbilityId(91),
            D,
            N::OdobenusBlight,
            AbilityDef {
                pierces: crate::Pierces::Yes,
                ..passive()
            },
            Spell::Strike(Strike::Lay {
                buff: Buff::of(Stat::Armor, negated(D.lv("armor_reduction"))),
                effect: EffectId(127),
                hero_ms: D.ms("armor_reduction_duration"),
                other_ms: D.ms("armor_reduction_duration"),
                dps: all(0),
            }),
        )
    ),
    // Miner's Aura: enemies near an Eimermole get less health restoration.
    with!(
        script::EIMERMOLE_MINERS_AURA,
        entry(
            AbilityId(92),
            D,
            N::MinersAura,
            passive(),
            Spell::Aura(AuraSpell {
                whom: Whom::Enemies,
                radius: D.int("radius"),
                buff: Buff::of(Stat::RestorationCutPct, D.lv("restoration_reduction")),
                effect: EffectId(128),
                stacks: false,
                water: false,
                breakable: D.breakable(),
            }),
        )
    ),
    // Present in the data, not cast: Miner's Slam hammers the ground in front of the Foreman
    // several times (pulses), and bota's slams strike once.
    with!(
        script::EIMERMOLE_MINERS_SLAM,
        entry(
            AbilityId(93),
            D,
            N::MinersSlam,
            cast_of!(D, Aim::Point, true),
            Spell::Cast {
                act: Act::Pulses {
                    // The script has one `radius`: the spot is that far in front of the
                    // Foreman and the pulse reaches that far round it.
                    reach: D.int("radius"),
                    radius: D.int("radius"),
                    damage: D.lv("damage"),
                    count: count(D.int("pulse_count")),
                    interval_ms: D.ms("pulse_interval"),
                    slow: Buff::of(Stat::MovePct, negated(D.lv("slow_pct"))),
                    slow_ms: D.ms("slow_duration"),
                    effect: EffectId(130),
                },
                // The wild acquisition condition is still unknown.
                when: When::Manual,
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    // Seal Flop: the seal leaps forward, lands with a slam, and leaps again (`max_bounces` leaps
    // in all). The script's `slow` is negative (-100); a pulse's slow is a positive percent.
    with!(
        script::OGRE_SEAL_FLOP,
        entry(
            AbilityId(94),
            D,
            N::SealFlop,
            cast_of!(D, Aim::Own, false),
            Spell::Cast {
                act: Act::Leap {
                    distance: D.int("leap_distance"),
                    speed: D.int("leap_speed"),
                    acceleration: D.int("leap_acceleration"),
                    radius: D.int("radius"),
                    damage: D.int("flop_damage"),
                    leaps: count(D.int("max_bounces")),
                    slow: Buff::of(Stat::MovePct, D.lv("slow")),
                    slow_ms: D.ms("slow_duration"),
                    effect: EffectId(132),
                },
                // The wild acquisition condition is still unknown.
                when: When::Manual,
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    // Seal of Disapproval: enemies near the pup are slowed.
    with!(
        script::OGRE_SEAL_PUP_DISAPPROVAL_AURA,
        entry(
            AbilityId(95),
            D,
            N::SealDisapprovalAura,
            passive(),
            Spell::Aura(AuraSpell {
                whom: Whom::Enemies,
                radius: D.int("radius"),
                buff: Buff::of(Stat::MovePct, negated(D.lv("movespeed_slow"))),
                effect: EffectId(129),
                stacks: false,
                water: false,
                breakable: D.breakable(),
            }),
        )
    ),
    // The golden neutrals' multipliers (Dota 7.42: experience x2, one gold roll x2 with the
    // least and greatest end both set to it, damage and maximum health +200%), present in the data
    // and not applied: no camp table row holds a golden neutral and nothing found in the
    // game spawns one, so none is ever spawned.
    with!(
        script::NEUTRAL_GOLD_UPGRADE_ABILITY,
        entry(AbilityId(96), D, N::GoldUpgrade, passive(), Spell::Idle)
    ),
    with!(
        script::BOTA_KOBOLD_STEAL_WEAPON,
        entry(
            AbilityId(97),
            D,
            N::StealWeaponCast,
            cast_of!(D, Aim::Point, true),
            Spell::Cast {
                act: Act::Spikes {
                    lines: 1,
                    spread_deg: 0,
                    length: D.int("AbilityCastRange"),
                    width: D.int("radius"),
                    damage: D.lv("base_damage"),
                    afflicts: Afflicts::Break,
                    ms: D.lv_ms("break_duration"),
                },
                when: When::Think(CastRule::on_held(&[Mark::InCastRange])),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::BOTA_ODOBENUS_SNOWBALL_SHOT,
        entry(
            AbilityId(98),
            D,
            N::SnowballShot,
            cast_of!(D, Aim::Unit, true),
            Spell::Cast {
                act: Act::Ricochet {
                    damage: D.lv("damage"),
                    bounces: D.lv("bounces"),
                    range: D.int("bounce_range"),
                    delay_ms: D.ms("bounce_delay"),
                    slow: Buff::of(Stat::MovePct, negated(D.lv("slow"))),
                    slow_ms: D.ms("duration"),
                    effect: EffectId(131),
                },
                when: When::Think(CastRule {
                    among: Among::Foes(Within::CastRange),
                    needs: &[],
                    marks: &[],
                    pick: Pick::Nearest,
                }),
                shared: None,
                chance: D.chance(),
            },
        )
    ),
    with!(
        script::BOTA_BLACK_DRAGON_SEARING_SWOOP,
        entry(
            AbilityId(99),
            D,
            N::SearingSwoop,
            cast_of!(D, Aim::Point, true),
            Spell::Uncast,
        )
    ),
];

/// The bubble the Marshmages put up: one modifier whichever of them cast
/// it, as the game has it.
pub const WATER_BUBBLE_EFFECT: EffectId = EffectId(54);

/// The modifier a purge leaves on an enemy.
pub const PURGE_EFFECT: EffectId = EffectId(55);

const ROSHAN_ROAR: Roar = Roar {
    windup: crate::roshan_data::ROSHAN_REVENGEROAR
        .first("windup_time")
        .secs()
        .ticks(),
    windup_effect: EffectId(88),
    emitter_effect: EffectId(89),
    disarm_effect: EffectId(90),
    damage_effect: EffectId(91),
    radius: crate::roshan_data::ROSHAN_REVENGEROAR.first("radius").int(),
    damage: crate::roshan_data::ROSHAN_REVENGEROAR.first("damage").int(),
    growth: crate::DamageGrowth {
        clock: crate::GrowthClock {
            first_after_horn: 60 * rules::TICKS_PER_SECOND,
            period: 60 * rules::TICKS_PER_SECOND,
            cap: u32::MAX,
        },
        per_step: all(crate::roshan_data::ROSHAN_REVENGEROAR
            .first("damage_growth")
            .int()),
    },
    disarm: crate::roshan_data::ROSHAN_REVENGEROAR
        .first("disarm_duration")
        .secs()
        .ticks(),
    aura_duration: crate::roshan_data::ROSHAN_REVENGEROAR
        .first("duration")
        .secs()
        .ticks(),
    aura: Aura {
        growth_clock: None,
        whom: Whom::Enemies,
        reaches: Reach::Living,
        radius: crate::roshan_data::ROSHAN_REVENGEROAR.first("radius").int(),
        linger: 15,
        modifier: AuraModifier::Spell(N::RevengeRoar),
        breakable: false,
        water: false,
        invulnerable: false,
    },
    buff: Buff::of(
        Stat::IncomingDamagePct,
        all(crate::roshan_data::ROSHAN_REVENGEROAR
            .first("damage_pct")
            .int()),
    ),
};

/// Longest cast windup a snapshot may show, including a spell's declared delayed pulse.
pub fn ability_cast_view_ticks(id: AbilityId) -> u32 {
    let point = crate::ability_def(id).map_or(0, |d| d.point);
    let ordinary = (u64::from(point) * u64::from(rules::TICKS_PER_SECOND))
        .div_ceil(1000)
        .min(u32::MAX.into()) as u32;
    let delayed = match NeutralAbility::of(id).map(NeutralAbility::spell) {
        Some(Spell::Cast {
            act: Act::Roar(spec),
            ..
        }) => spec.windup,
        _ => 0,
    };
    ordinary.max(delayed)
}

/// Override for an ability cast by a player-controlled neutral, in milliseconds.
pub fn controlled_neutral_cast_point(id: AbilityId) -> Option<u32> {
    match NeutralAbility::of(id)? {
        NeutralAbility::OgreSmash => Some(
            u32::try_from(script::OGRE_BRUISER_OGRE_SMASH.ms("player_cast_point"))
                .expect("a nonnegative cast point"),
        ),
        _ => None,
    }
}

pub const fn migration_of(ability: NeutralAbility) -> Option<&'static crate::Migration> {
    match *ability.spell() {
        Spell::Intrinsic { moving, .. } => moving,
        _ => None,
    }
}
const ROSHAN_THROW: Grab = Grab {
    duration: crate::roshan_data::ROSHAN_GRAB
        .first("channel_time")
        .secs()
        .ticks(),
    contact_after: crate::roshan_data::ROSHAN_GRAB
        .first("interval")
        .secs()
        .ticks(),
    distance: crate::roshan_data::ROSHAN_GRAB
        .first("max_knockback_distance")
        .int(),
    flight: crate::roshan_data::ROSHAN_GRAB
        .first("knockback_duration")
        .secs()
        .ticks(),
    height: crate::roshan_data::ROSHAN_GRAB
        .first("knockback_height")
        .int(),
    damage: crate::roshan_data::ROSHAN_GRAB.first("damage").int(),
    damage_per_throw: crate::roshan_data::ROSHAN_GRAB
        .first("damage_per_combo")
        .int(),
    health_growth: crate::DamageGrowth {
        clock: crate::GrowthClock {
            first_after_horn: 0,
            period: 60 * rules::TICKS_PER_SECOND,
            cap: u32::MAX,
        },
        per_step: all(crate::roshan_data::ROSHAN_GRAB
            .first("damage_growth_curr_hp_pct")
            .0
            / 10),
    },
    stun: crate::roshan_data::ROSHAN_GRAB
        .first("stun_duration")
        .secs()
        .ticks(),
    grabbed_effect: EffectId(93),
    thrown_effect: EffectId(94),
    contact_ability: N::StrengthOfTheImmortal,
};
