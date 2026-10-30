//! Every ability a neutral creep carries: what it is called, how it is aimed,
//! what it costs, and what it does, as data a handful of shared mechanics
//! read.
//!
//! The numbers are the game's `scripts/npc/npc_abilities.txt` (and, for Ice
//! Armor, Ensnare and Raise Dead, the hero files that hold them), client 6957
//! (7.41f). When the neutral AI casts an ability, and the parts of an effect
//! the files leave out, are Liquipedia's article of each creep (read
//! 2026-10-18). A neutral's abilities come in four levels; one spawns with
//! them at the level of [`neutral_ability_level`] and keeps it.
//!
//! A new ability that one of the mechanics below already covers is one
//! variant appended to [`NeutralAbility`], one entry appended to
//! [`NEUTRAL_ABILITIES`] carrying the next free id, and that id's line in the
//! protocol's id registry (append-only: an id is never renumbered or handed
//! out twice).

use bota_proto::{AbilityId, Aim, DamageKind, EffectId};

use crate::{
    ABILITY_COUNT, AbilityDef, Aura, AuraModifier, BARE_ABILITY, NeutralKind, Reach, Whom, rules,
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
            Spell::Strike(_) => true,
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
pub const fn neutral_ability_level(tick: u32) -> u8 {
    let since = tick.saturating_sub(rules::tick_at_game_minute(0));
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
}

/// What a spell's own modifier does to the stats of whatever wears it: up
/// to two stats, each changed by its value at the spell's level, a gain
/// above zero and a loss below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buff {
    pub stats: [(Stat, Lv); 2],
}

impl Buff {
    /// One stat changed.
    pub const fn of(stat: Stat, values: Lv) -> Buff {
        Buff {
            stats: [(stat, values), (stat, all(0))],
        }
    }

    /// Two stats changed.
    pub const fn two(first: Stat, firsts: Lv, second: Stat, seconds: Lv) -> Buff {
        Buff {
            stats: [(first, firsts), (second, seconds)],
        }
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

/// A passive on the bearer's own attacks.
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
    /// `mult_pct` percent damage.
    Crit { chance_pct: i32, mult_pct: Lv },
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
    /// Every enemy within `radius` of the caster takes `damage` of `kind`
    /// and is stunned, or wears the spell's modifier, for `hero_ms` on a hero
    /// and `other_ms` on anything else.
    Stomp {
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
    /// cross takes `damage` magical and is stunned for `ms`.
    Spikes {
        lines: u8,
        spread_deg: i32,
        length: i32,
        width: i32,
        damage: Lv,
        ms: i32,
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
    Push { distance: i32, speed: i32 },
    /// Takes the target's buffs off and slows it, the slow falling from
    /// `start_pct` by a fifth of it each second over `ms`.
    Purge { start_pct: i32, ms: i32 },
    /// A tornado of `kind` stands at the target for `life_ms`.
    Tornado { kind: NeutralKind, life_ms: i32 },
}

/// What a stomp leaves on what it hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Then {
    /// Stunned.
    Stun,
    /// The spell's modifier.
    Wear { buff: Buff, effect: EffectId },
}

/// When the neutral AI casts a spell, besides its being off cooldown and
/// paid for, and its caster awake, set on somebody, and neither stunned nor
/// disarmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// At least `at_least` enemies stand within `radius`.
    Crowd { radius: i32, at_least: u8 },
    /// While it swings, at least `at_least` enemies within `radius` are set
    /// on it.
    Beset { radius: i32, at_least: u8 },
    /// Below `below_pct` of its health, with an enemy within `radius` (any
    /// distance for zero).
    Hurt { below_pct: i32, radius: i32 },
    /// At an enemy in reach with less than `below_hp` health; `attacker`:
    /// only one set on the caster.
    Weak { below_hp: i32, attacker: bool },
    /// At itself, whenever it fights.
    Fighting,
    /// At an ally in reach below `below_pct` of its health.
    AllyHurt { below_pct: i32 },
    /// At the hero it is set on, while a hero of the other side stands near.
    Duel { radius: i32 },
    /// At a hero set on it that chases another hero within `radius`.
    Chaser { radius: i32 },
    /// At an enemy hero within `radius` wearing a buff a purge takes off.
    Buffed { radius: i32 },
    /// At the enemy it is set on, once in reach.
    Target,
    /// A `chance_pct` chance as a neutral of `kind` of its camp dies, at
    /// whoever killed it.
    Avenge { kind: NeutralKind, chance_pct: i32 },
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
    },
}

impl Spell {
    /// The aura a passive of `ability` is, for one that hands its modifier
    /// out to the units near its bearer: an aura or a storm. Each reaches
    /// living units only and lingers [`rules::AURA_LINGER_TICKS`]; a Break
    /// silences it as its spell's `breakable` says.
    const fn aura(&self, ability: NeutralAbility) -> Option<Aura> {
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
    pub spell: Spell,
}

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
const fn passive(name: &'static str) -> AbilityDef {
    AbilityDef {
        name,
        max_level: 4,
        ..BARE_ABILITY
    }
}

/// A passive with a cooldown, in seconds by level.
const fn passive_cooling(name: &'static str, cooldown: &'static [u32]) -> AbilityDef {
    AbilityDef {
        cooldown,
        ..passive(name)
    }
}

/// An ability that is cast.
#[allow(clippy::too_many_arguments)]
const fn cast(
    name: &'static str,
    aim: Aim,
    at_an_enemy: bool,
    mana: &'static [i32],
    cooldown: &'static [u32],
    range: i32,
    point: u32,
    backswing: u32,
) -> AbilityDef {
    AbilityDef {
        name,
        aim,
        max_level: 4,
        passive: false,
        at_an_enemy,
        mana,
        cooldown,
        range,
        point,
        backswing,
        ..BARE_ABILITY
    }
}

/// Seconds as ticks, for a cooldown.
const fn secs(seconds: u32) -> u32 {
    seconds * rules::TICKS_PER_SECOND
}

/// One entry, its ability carrying the aura its spell is, if it is one.
const fn entry(
    id: AbilityId,
    key: &'static str,
    ability: NeutralAbility,
    def: AbilityDef,
    spell: Spell,
) -> NeutralAbilityDef {
    NeutralAbilityDef {
        ability,
        def: AbilityDef {
            id,
            key,
            aura: spell.aura(ability),
            ..def
        },
        spell,
    }
}

/// An aura for its own side.
const fn allies(radius: i32, buff: Buff, effect: u16) -> Spell {
    Spell::Aura(AuraSpell::allies(radius, buff, effect))
}

impl AuraSpell {
    /// An aura for its own side: one modifier whatever the sources, silenced
    /// by a Break, working anywhere.
    const fn allies(radius: i32, buff: Buff, effect: u16) -> AuraSpell {
        AuraSpell {
            whom: Whom::Allies,
            radius,
            buff,
            effect: EffectId(effect),
            stacks: false,
            water: false,
            breakable: true,
        }
    }
}

/// Families of casts that hold each other off.
const FROG_SPIKES: u8 = 0;
const GOLEM_BOULDER: u8 = 1;
const OGRE_SMASH: u8 = 2;

/// The hold the frogs' spikes put on each other (7.41: "all copies ... on
/// other neutral Boglets, Croakers, and Ancient Croakers within 1200 range
/// are put on 5s cooldown"). The golems and the ogres "can't do it if
/// recently other golem used this ability", with no numbers given; the frogs'
/// are used for them.
const fn hold(family: u8) -> Option<Shared> {
    Some(Shared {
        family,
        radius: 1200,
        ms: 5000,
    })
}

use NeutralAbility as N;

/// Every neutral ability, in the order of [`NeutralAbility`].
pub const NEUTRAL_ABILITIES: &[NeutralAbilityDef] = &[
    entry(
        AbilityId(17),
        "kobold_tunneler_prospecting",
        N::ProspectingAura,
        passive("Prospecting Aura"),
        Spell::Idle,
    ),
    entry(
        AbilityId(18),
        "neutral_upgrade",
        N::NeutralUpgrade,
        passive("Neutral Upgrade"),
        Spell::Upgrade,
    ),
    entry(
        AbilityId(19),
        "kobold_disarm",
        N::StealWeapon,
        passive_cooling("Steal Weapon", &[secs(9)]),
        Spell::Strike(Strike::Disarm {
            hits: 3,
            ms: lv(3000, 3500, 4000, 5000),
            counter: EffectId(25),
        }),
    ),
    entry(
        AbilityId(20),
        "kobold_taskmaster_speed_aura",
        N::SpeedAura,
        passive("Speed Aura"),
        allies(1200, Buff::of(Stat::MovePct, lv(12, 13, 14, 16)), 26),
    ),
    entry(
        AbilityId(21),
        "berserker_troll_break",
        N::Break,
        passive_cooling("Break", &[secs(10), secs(9), secs(8), secs(6)]),
        Spell::Strike(Strike::Break { ms: 3000 }),
    ),
    entry(
        AbilityId(22),
        "creep_piercing",
        N::Piercing,
        passive("Piercing"),
        Spell::Piercing,
    ),
    entry(
        AbilityId(23),
        "forest_troll_high_priest_heal",
        N::Heal,
        cast(
            "Heal",
            Aim::Unit,
            false,
            &[60],
            &[secs(10), secs(9), secs(8), secs(6)],
            350,
            500,
            700,
        ),
        Spell::Uncast,
    ),
    entry(
        AbilityId(24),
        "forest_troll_high_priest_heal_amp_aura",
        N::HealAmplificationAura,
        passive("Heal Amplification Aura"),
        Spell::Idle,
    ),
    entry(
        AbilityId(25),
        "gnoll_assassin_envenomed_weapon",
        N::EnvenomedWeapon,
        passive("Envenomed Weapon"),
        Spell::Strike(Strike::Lay {
            buff: Buff::of(Stat::RegenCutPct, lv(75, 80, 85, 90)),
            effect: EffectId(27),
            hero_ms: 2000,
            other_ms: 20000,
            dps: lv(0, 20, 40, 80),
        }),
    ),
    entry(
        AbilityId(26),
        "fel_beast_haunt",
        N::Vex,
        cast(
            "Vex",
            Aim::Unit,
            true,
            &[75],
            &[secs(15), secs(13), secs(11), secs(7)],
            600,
            0,
            500,
        ),
        Spell::Uncast,
    ),
    entry(
        AbilityId(27),
        "ghost_frost_attack",
        N::FrostAttack,
        passive("Frost Attack"),
        Spell::Strike(Strike::Lay {
            buff: chill(lv(-25, -28, -31, -37), lv(-25, -28, -31, -37)),
            effect: EffectId(28),
            hero_ms: 1500,
            other_ms: 1500,
            dps: all(0),
        }),
    ),
    entry(
        AbilityId(28),
        "harpy_scout_take_off",
        N::TakeOff,
        cast("Take Off", Aim::Own, false, &[20], &[secs(10)], 0, 1500, 0),
        Spell::Uncast,
    ),
    entry(
        AbilityId(29),
        "harpy_storm_chain_lightning",
        N::ChainLightning,
        cast(
            "Chain Lightning",
            Aim::Unit,
            true,
            &[60],
            &[secs(4)],
            900,
            300,
            670,
        ),
        Spell::Cast {
            act: Act::Chain {
                damage: lv(120, 170, 220, 270),
                targets: 4,
                radius: 500,
                delay_ms: 250,
                keep_pct: lv(75, 80, 85, 90),
            },
            when: When::Weak {
                below_hp: 330,
                attacker: true,
            },
            shared: None,
        },
    ),
    entry(
        AbilityId(30),
        "mudgolem_cloak_aura",
        N::CloakAura,
        passive("Cloak Aura"),
        allies(
            1200,
            Buff::two(
                Stat::MagicResistPct,
                lv(10, 12, 14, 16),
                Stat::MagicResistCreepPct,
                lv(20, 24, 28, 32),
            ),
            29,
        ),
    ),
    entry(
        AbilityId(31),
        "centaur_khan_war_stomp",
        N::WarStomp,
        cast(
            "War Stomp",
            Aim::Own,
            false,
            &[50],
            &[secs(12)],
            0,
            400,
            570,
        ),
        Spell::Cast {
            act: Act::Stomp {
                radius: all(250),
                damage: lv(25, 50, 75, 150),
                kind: DamageKind::Magical,
                hero_ms: all(1600),
                other_ms: all(3000),
                then: Then::Stun,
            },
            when: When::Crowd {
                radius: 250,
                at_least: 3,
            },
            shared: None,
        },
    ),
    entry(
        AbilityId(32),
        "giant_wolf_intimidate",
        N::Intimidate,
        cast(
            "Intimidate",
            Aim::Own,
            false,
            &[50],
            &[secs(16)],
            0,
            200,
            800,
        ),
        Spell::Cast {
            act: Act::Stomp {
                radius: lv(300, 350, 400, 500),
                damage: all(0),
                kind: DamageKind::Magical,
                hero_ms: all(4000),
                other_ms: all(4000),
                then: Then::Wear {
                    buff: Buff::of(Stat::DamagePct, all(-60)),
                    effect: EffectId(30),
                },
            },
            when: When::Hurt {
                below_pct: 51,
                radius: 300,
            },
            shared: None,
        },
    ),
    entry(
        AbilityId(33),
        "alpha_wolf_critical_strike",
        N::CriticalStrike,
        passive("Critical Strike"),
        Spell::Strike(Strike::Crit {
            chance_pct: 20,
            mult_pct: lv(200, 225, 250, 300),
        }),
    ),
    entry(
        AbilityId(34),
        "alpha_wolf_command_aura",
        N::PackleadersAura,
        passive("Packleader's Aura"),
        allies(1200, Buff::of(Stat::DamagePct, all(20)), 31),
    ),
    entry(
        AbilityId(35),
        "satyr_trickster_purge",
        N::Purge,
        cast(
            "Purge",
            Aim::Unit,
            false,
            &[120, 115, 110, 100],
            &[secs(3)],
            350,
            200,
            800,
        ),
        Spell::Cast {
            act: Act::Purge {
                start_pct: 50,
                ms: 5000,
            },
            when: When::Buffed { radius: 500 },
            shared: None,
        },
    ),
    entry(
        AbilityId(36),
        "satyr_soulstealer_mana_burn",
        N::ManaBurn,
        cast(
            "Mana Burn",
            Aim::Unit,
            true,
            &[50],
            &[secs(18)],
            600,
            300,
            800,
        ),
        Spell::Uncast,
    ),
    entry(
        AbilityId(37),
        "forest_troll_high_priest_mana_aura",
        N::ManaAura,
        passive("Mana Aura"),
        allies(1200, Buff::of(Stat::ManaRegen, all(175)), 32),
    ),
    entry(
        AbilityId(38),
        "satyr_hellcaller_shockwave",
        N::Shockwave,
        cast(
            "Shockwave",
            Aim::Point,
            true,
            &[100],
            &[secs(8)],
            700,
            500,
            830,
        ),
        Spell::Uncast,
    ),
    entry(
        AbilityId(39),
        "satyr_hellcaller_unholy_aura",
        N::UnholyAura,
        passive("Unholy Aura"),
        allies(1200, Buff::of(Stat::HpRegen, lv(30, 50, 70, 110)), 33),
    ),
    entry(
        AbilityId(40),
        "ogre_bruiser_ogre_smash",
        N::OgreSmash,
        cast(
            "Ogre Smash!",
            Aim::Point,
            true,
            &[0],
            &[secs(12)],
            0,
            2800,
            850,
        ),
        Spell::Cast {
            act: Act::Smash {
                reach: 200,
                radius: lv(200, 210, 220, 230),
                damage: lv(200, 250, 300, 400),
                current_pct: 8,
                ms: 2400,
            },
            when: When::Hurt {
                below_pct: 70,
                radius: 400,
            },
            shared: hold(OGRE_SMASH),
        },
    ),
    entry(
        AbilityId(41),
        "ogre_magi_frost_armor",
        N::IceArmor,
        cast(
            "Ice Armor",
            Aim::Unit,
            false,
            &[40],
            &[secs(5)],
            800,
            560,
            500,
        ),
        Spell::Uncast,
    ),
    entry(
        AbilityId(42),
        "mud_golem_hurl_boulder",
        N::HurlBoulder,
        cast(
            "Hurl Boulder",
            Aim::Unit,
            true,
            &[0],
            &[secs(30)],
            800,
            300,
            600,
        ),
        Spell::Cast {
            act: Act::Bolt {
                speed: 800,
                hero_damage: 75,
                other_damage: 150,
                ms: 500,
                root: false,
            },
            when: When::Weak {
                below_hp: 300,
                attacker: false,
            },
            shared: hold(GOLEM_BOULDER),
        },
    ),
    entry(
        AbilityId(43),
        "mud_golem_rock_destroy",
        N::ShardSplit,
        passive("Shard Split"),
        Spell::Throe(Throe::Split {
            kind: NeutralKind::MudGolemSplit,
            count: lv(2, 2, 3, 3),
            hp: lv(250, 280, 310, 370),
            damage: lv(12, 16, 20, 28),
            life_ms: 60_000,
            radius: 250,
            slow_pct: 25,
            slow_ms: 2000,
            effect: EffectId(34),
        }),
    ),
    entry(
        AbilityId(44),
        "centaur_khan_endurance_aura",
        N::SwiftnessAura,
        passive("Swiftness Aura"),
        allies(1200, Buff::of(Stat::AttackSpeed, lv(15, 18, 21, 27)), 35),
    ),
    entry(
        AbilityId(45),
        "furbolg_enrage_attack_speed",
        N::DeathThroeRush,
        passive("Death Throe: Rush"),
        Spell::Throe(Throe::Rouse {
            radius: 700,
            ms: 5000,
            buff: Buff::of(Stat::AttackSpeed, all(120)),
            effect: EffectId(36),
        }),
    ),
    entry(
        AbilityId(46),
        "polar_furbolg_ursa_warrior_thunder_clap",
        N::ThunderClap,
        cast(
            "Thunder Clap",
            Aim::Own,
            false,
            &[50],
            &[secs(12), secs(11), secs(10), secs(9)],
            0,
            400,
            600,
        ),
        Spell::Cast {
            act: Act::Stomp {
                radius: all(300),
                damage: lv(150, 200, 250, 350),
                kind: DamageKind::Magical,
                hero_ms: all(3000),
                other_ms: all(3000),
                then: Then::Wear {
                    buff: chill(all(-25), all(-25)),
                    effect: EffectId(37),
                },
            },
            when: When::Crowd {
                radius: 300,
                at_least: 2,
            },
            shared: None,
        },
    ),
    entry(
        AbilityId(47),
        "furbolg_enrage_damage",
        N::DeathThroePower,
        passive("Death Throe: Power"),
        Spell::Throe(Throe::Rouse {
            radius: 700,
            ms: 5000,
            buff: Buff::of(Stat::DamagePct, all(60)),
            effect: EffectId(38),
        }),
    ),
    entry(
        AbilityId(48),
        "enraged_wildkin_tornado",
        N::Tornado,
        cast(
            "Tornado",
            Aim::Point,
            true,
            &[100],
            &[secs(40), secs(36), secs(32), secs(26)],
            500,
            400,
            0,
        ),
        Spell::Cast {
            act: Act::Tornado {
                kind: NeutralKind::WildkinTornado,
                life_ms: 15_000,
            },
            when: When::Avenge {
                kind: NeutralKind::EnragedWildkin,
                chance_pct: 33,
            },
            shared: None,
        },
    ),
    entry(
        AbilityId(49),
        "enraged_wildkin_hurricane",
        N::Hurricane,
        cast(
            "Hurricane",
            Aim::Unit,
            true,
            &[100],
            &[secs(30), secs(28), secs(26), secs(22)],
            400,
            0,
            800,
        ),
        Spell::Cast {
            act: Act::Push {
                distance: 400,
                speed: 800,
            },
            when: When::Chaser { radius: 1200 },
            shared: None,
        },
    ),
    entry(
        AbilityId(50),
        "enraged_wildkin_toughness_aura",
        N::ToughnessAura,
        passive("Toughness Aura"),
        allies(1200, Buff::of(Stat::Armor, all(3)), 39),
    ),
    entry(
        AbilityId(51),
        "dark_troll_warlord_ensnare",
        N::Ensnare,
        cast(
            "Ensnare",
            Aim::Unit,
            true,
            &[75],
            &[secs(15)],
            550,
            300,
            360,
        ),
        Spell::Cast {
            act: Act::Bolt {
                speed: 1500,
                hero_damage: 0,
                other_damage: 0,
                ms: 1750,
                root: true,
            },
            when: When::Duel { radius: 1200 },
            shared: None,
        },
    ),
    entry(
        AbilityId(52),
        "dark_troll_warlord_raise_dead",
        N::RaiseDead,
        cast(
            "Raise Dead",
            Aim::Own,
            false,
            &[50],
            &[secs(20)],
            0,
            300,
            1200,
        ),
        Spell::Cast {
            act: Act::Summon {
                kind: NeutralKind::SkeletonWarrior,
                count: 3,
                hp: lv(250, 275, 300, 375),
                damage: lv(12, 15, 18, 21),
                life_ms: 35_000,
            },
            when: When::Hurt {
                below_pct: 50,
                radius: 0,
            },
            shared: None,
        },
    ),
    entry(
        AbilityId(53),
        "warpine_raider_seed_shot",
        N::SeedShot,
        cast(
            "Seed Shot",
            Aim::Unit,
            true,
            &[100],
            &[secs(15)],
            575,
            200,
            0,
        ),
        Spell::Uncast,
    ),
    entry(
        AbilityId(54),
        "black_drake_magic_amplification_aura",
        N::MagicAmplificationAura,
        passive("Magic Amplification Aura"),
        Spell::Aura(AuraSpell {
            whom: Whom::Enemies,
            radius: 1200,
            buff: Buff::of(Stat::MagicTakenPct, lv(5, 6, 7, 9)),
            effect: EffectId(40),
            stacks: false,
            water: false,
            breakable: true,
        }),
    ),
    entry(
        AbilityId(55),
        "black_dragon_fireball",
        N::Fireball,
        cast(
            "Fireball",
            Aim::Point,
            true,
            &[200],
            &[secs(15)],
            1000,
            300,
            1100,
        ),
        Spell::Uncast,
    ),
    entry(
        AbilityId(56),
        "black_dragon_splash_attack",
        N::SplashAttack,
        passive("Splash Attack"),
        Spell::Strike(Strike::Splash {
            radius: 250,
            pct: 100,
        }),
    ),
    entry(
        AbilityId(57),
        "black_dragon_dragonhide_aura",
        N::DragonhideAura,
        passive("Dragonhide Aura"),
        Spell::Aura(AuraSpell {
            whom: Whom::Allies,
            radius: 1200,
            buff: Buff::of(Stat::Armor, all(3)),
            effect: EffectId(41),
            stacks: true,
            water: false,
            breakable: true,
        }),
    ),
    entry(
        AbilityId(58),
        "ancient_rock_golem_weakening_aura",
        N::WeakeningAura,
        passive("Weakening Aura"),
        Spell::Aura(AuraSpell {
            whom: Whom::Enemies,
            radius: 1200,
            buff: Buff::of(Stat::Armor, lv(-3, -4, -5, -6)),
            effect: EffectId(42),
            stacks: false,
            water: false,
            breakable: true,
        }),
    ),
    entry(
        AbilityId(59),
        "granite_golem_hp_aura",
        N::GraniteAura,
        passive("Granite Aura"),
        Spell::Aura(AuraSpell {
            breakable: false,
            ..AuraSpell::allies(1200, Buff::of(Stat::MaxHpPct, lv(16, 17, 18, 19)), 43)
        }),
    ),
    entry(
        AbilityId(60),
        "big_thunder_lizard_wardrums_aura",
        N::WarDrumsAura,
        passive("War Drums Aura"),
        allies(
            1200,
            Buff::two(
                Stat::AttackSpeed,
                all(25),
                Stat::AccuracyPct,
                lv(40, 43, 46, 51),
            ),
            44,
        ),
    ),
    entry(
        AbilityId(61),
        "big_thunder_lizard_slam",
        N::Slam,
        cast("Slam", Aim::Own, false, &[150], &[secs(6)], 0, 300, 430),
        Spell::Cast {
            act: Act::Stomp {
                radius: all(350),
                damage: all(250),
                kind: DamageKind::Magical,
                hero_ms: lv(2000, 2250, 2500, 3000),
                other_ms: all(4000),
                then: Then::Wear {
                    buff: chill(all(-60), all(-60)),
                    effect: EffectId(45),
                },
            },
            when: When::Beset {
                radius: 350,
                at_least: 3,
            },
            shared: None,
        },
    ),
    entry(
        AbilityId(62),
        "big_thunder_lizard_frenzy",
        N::Frenzy,
        cast("Frenzy", Aim::Unit, false, &[75], &[secs(8)], 900, 0, 800),
        Spell::Cast {
            act: Act::Enrage {
                buff: Buff::of(Stat::AttackSpeed, all(75)),
                effect: EffectId(46),
                ms: 8000,
            },
            when: When::Fighting,
            shared: None,
        },
    ),
    entry(
        AbilityId(63),
        "frostbitten_golem_time_warp_aura",
        N::TimeWarpAura,
        passive("Time Warp Aura"),
        allies(1200, Buff::of(Stat::CooldownPct, lv(8, 9, 10, 11)), 47),
    ),
    entry(
        AbilityId(64),
        "ice_shaman_incendiary_bomb",
        N::IcefireBomb,
        cast(
            "Icefire Bomb",
            Aim::Unit,
            true,
            &[100, 95, 90, 80],
            &[secs(30)],
            700,
            350,
            650,
        ),
        Spell::Uncast,
    ),
    entry(
        AbilityId(65),
        "spawnlord_aura",
        N::ProwlerAura,
        passive("Prowler Aura"),
        allies(
            1200,
            Buff::two(
                Stat::LifestealPct,
                lv(9, 10, 11, 12),
                Stat::HpRegen,
                lv(90, 100, 110, 120),
            ),
            48,
        ),
    ),
    entry(
        AbilityId(66),
        "spawnlord_master_stomp",
        N::Desecrate,
        cast(
            "Desecrate",
            Aim::Own,
            false,
            &[100],
            &[secs(12)],
            0,
            400,
            1370,
        ),
        Spell::Cast {
            act: Act::Stomp {
                radius: all(300),
                damage: all(200),
                kind: DamageKind::Physical,
                hero_ms: all(6000),
                other_ms: all(6000),
                then: Then::Wear {
                    buff: Buff::of(Stat::BaseArmorPct, all(-50)),
                    effect: EffectId(49),
                },
            },
            when: When::Crowd {
                radius: 300,
                at_least: 3,
            },
            shared: None,
        },
    ),
    entry(
        AbilityId(67),
        "spawnlord_master_freeze",
        N::Petrify,
        passive_cooling("Petrify", &[secs(15)]),
        Spell::Strike(Strike::Petrify {
            below_pct: 90,
            ms: lv(1200, 1300, 1400, 1500),
            dps: 100,
            effect: EffectId(50),
        }),
    ),
    entry(
        AbilityId(68),
        "frogmen_riverborn_aura",
        N::RiverbornAura,
        passive("Riverborn Aura"),
        Spell::Aura(AuraSpell {
            whom: Whom::Allies,
            radius: 1200,
            buff: Buff::two(
                Stat::MovePct,
                lv(10, 12, 14, 16),
                Stat::OutgoingPct,
                lv(10, 12, 14, 16),
            ),
            effect: EffectId(51),
            stacks: false,
            water: true,
            breakable: true,
        }),
    ),
    entry(
        AbilityId(69),
        "frogmen_arm_of_the_deep",
        N::ArmOfTheDeep,
        cast(
            "Arm of the Deep",
            Aim::Point,
            true,
            &[40],
            &[secs(16)],
            275,
            300,
            0,
        ),
        Spell::Cast {
            act: Act::Spikes {
                lines: 1,
                spread_deg: 0,
                length: 275,
                width: 100,
                damage: lv(80, 100, 120, 140),
                ms: 800,
            },
            when: When::Target,
            shared: hold(FROG_SPIKES),
        },
    ),
    entry(
        AbilityId(70),
        "frogmen_tendrils_of_the_deep",
        N::TendrilsOfTheDeep,
        cast(
            "Tendrils of the Deep",
            Aim::Point,
            true,
            &[50],
            &[secs(18)],
            300,
            300,
            0,
        ),
        Spell::Cast {
            act: Act::Spikes {
                lines: 2,
                spread_deg: 34,
                length: 300,
                width: 100,
                damage: lv(120, 140, 160, 180),
                ms: 1200,
            },
            when: When::Target,
            shared: hold(FROG_SPIKES),
        },
    ),
    entry(
        AbilityId(71),
        "frogmen_congregation_of_the_deep",
        N::CongregationsOfTheDeep,
        cast(
            "Congregations of the Deep",
            Aim::Own,
            false,
            &[70],
            &[secs(20)],
            300,
            300,
            200,
        ),
        Spell::Cast {
            act: Act::Spikes {
                lines: 6,
                spread_deg: 60,
                length: 300,
                width: 100,
                damage: lv(250, 275, 300, 325),
                ms: 1400,
            },
            when: When::Target,
            shared: hold(FROG_SPIKES),
        },
    ),
    entry(
        AbilityId(72),
        "frogmen_water_bubble_small",
        N::WaterBubbleSmall,
        cast(
            "Water Bubble (Small)",
            Aim::Unit,
            false,
            &[90],
            &[secs(16)],
            550,
            300,
            1100,
        ),
        Spell::Cast {
            act: Act::Bubble {
                absorb: lv(100, 120, 140, 160),
                ms: 10_000,
                heal_pct: 0,
                radius: 0,
            },
            when: When::AllyHurt { below_pct: 50 },
            shared: None,
        },
    ),
    entry(
        AbilityId(73),
        "frogmen_water_bubble_medium",
        N::WaterBubbleMedium,
        cast(
            "Water Bubble (Medium)",
            Aim::Unit,
            false,
            &[100],
            &[secs(16)],
            550,
            300,
            1100,
        ),
        Spell::Cast {
            act: Act::Bubble {
                absorb: lv(150, 180, 210, 240),
                ms: 10_000,
                heal_pct: 50,
                radius: 0,
            },
            when: When::AllyHurt { below_pct: 50 },
            shared: None,
        },
    ),
    entry(
        AbilityId(74),
        "frogmen_water_bubble_large",
        N::WaterBubbleLarge,
        cast(
            "Water Bubble (Large)",
            Aim::Unit,
            false,
            &[110],
            &[secs(16)],
            550,
            300,
            1100,
        ),
        Spell::Cast {
            act: Act::Bubble {
                absorb: lv(210, 240, 270, 300),
                ms: 10_000,
                heal_pct: 50,
                radius: 675,
            },
            when: When::AllyHurt { below_pct: 50 },
            shared: None,
        },
    ),
    entry(
        AbilityId(75),
        "hill_troll_rally",
        N::Rally,
        passive("Rally"),
        Spell::Aura(AuraSpell {
            whom: Whom::Allies,
            radius: 1200,
            buff: Buff::of(Stat::Damage, all(2)),
            effect: EffectId(52),
            stacks: true,
            water: false,
            breakable: true,
        }),
    ),
    entry(
        AbilityId(76),
        "tornado_tempest",
        N::Tempest,
        passive("Tempest"),
        Spell::Storm {
            radius: 600,
            near_radius: 150,
            near_dps: 45,
            far_dps: 15,
            period_ms: 250,
            buff: chill(all(-15), all(-15)),
            effect: EffectId(53),
            breakable: false,
        },
    ),
];

/// The bubble the Marshmages put up: one modifier whichever of them cast
/// it, as the game has it.
pub const WATER_BUBBLE_EFFECT: EffectId = EffectId(54);

/// The modifier a purge leaves on an enemy.
pub const PURGE_EFFECT: EffectId = EffectId(55);
