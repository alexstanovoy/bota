//! Heroes as Dota's `scripts/npc/heroes/` give them, in their own units: the shape of the
//! generated table `heroes_data.rs`, and the conversions of its numbers into bota's. A hero
//! module reads its numbers from here by Dota's keys, so a number of a hero is never copied
//! by hand.

use bota_proto::{Attribute, DamageKind, Fixed};

use crate::heroes_data::{ABILITY_ROWS, HERO_ROWS};
use crate::item_data::str_eq;
use crate::{Behavior, Dec, Dispel, Ms, rules};

/// One hero as its script has it, over `npc_dota_hero_base`'s defaults.
#[derive(Clone, Copy, Debug)]
pub struct HeroData {
    /// Its Dota key: `npc_dota_hero_pudge`.
    pub key: &'static str,
    /// `AttributePrimary`.
    pub primary: Attribute,
    /// `AttributeBaseStrength` and `AttributeStrengthGain`.
    pub strength: Dec,
    pub strength_gain: Dec,
    /// `AttributeBaseAgility` and `AttributeAgilityGain`.
    pub agility: Dec,
    pub agility_gain: Dec,
    /// `AttributeBaseIntelligence` and `AttributeIntelligenceGain`.
    pub intelligence: Dec,
    pub intelligence_gain: Dec,
    /// `ArmorPhysical`, before agility.
    pub armor: Dec,
    /// `MagicalResistance`, in percent, before intelligence.
    pub magic_resist: Dec,
    /// `AttackCapabilities` is `DOTA_UNIT_CAP_MELEE_ATTACK`.
    pub melee: bool,
    /// `AttackDamageMin` and `AttackDamageMax`, before the primary attribute.
    pub damage_min: i32,
    pub damage_max: i32,
    /// `AttackRate`: the base attack time.
    pub attack_rate: Ms,
    /// `AttackAnimationPoint`.
    pub attack_point: Ms,
    /// `AttackRange`.
    pub attack_range: i32,
    /// `AttackAcquisitionRange`.
    pub acquisition: i32,
    /// `ProjectileSpeed`; zero for a melee hero.
    pub projectile_speed: i32,
    /// `StatusHealth` and `StatusHealthRegen` a second, before strength.
    pub health: i32,
    pub health_regen: Dec,
    /// `StatusMana` and `StatusManaRegen` a second, before intelligence.
    pub mana: i32,
    pub mana_regen: Dec,
    /// `MovementSpeed`.
    pub move_speed: i32,
    /// `MovementTurnRate`, in radians per 0.03 seconds.
    pub turn_rate: Dec,
    /// `VisionDaytimeRange` and `VisionNighttimeRange`.
    pub day_vision: i32,
    pub night_vision: i32,
    /// The abilities of its slots `Ability1` to `Ability9`, in slot order, Dota's empty
    /// `generic_hidden` slots left out.
    pub abilities: &'static [&'static str],
    /// Its talents in [`TalentSlot`](bota_proto::TalentSlot) order: tier by tier, the left one
    /// (Dota's odd `AbilityN`) first.
    pub talents: [&'static str; 8],
}

/// One ability as its script has it: a hero's, or a talent's of `npc_abilities.txt`.
#[derive(Clone, Copy, Debug)]
pub struct AbilityData {
    /// Its Dota key.
    pub key: &'static str,
    /// `AbilityBehavior`.
    pub behavior: Behavior,
    /// `AbilityType` is `ABILITY_TYPE_ULTIMATE`.
    pub ultimate: bool,
    /// `Innate`.
    pub innate: bool,
    /// `MaxLevel`, or Dota's default: 3 for an ultimate, 1 for a talent, 4 for any other.
    pub max_level: u8,
    /// `AbilityCastPoint` a level.
    pub cast_point: &'static [Ms],
    /// `AbilityCastRange` a level, from the ability or its values.
    pub cast_range: &'static [i32],
    /// `AbilityChannelTime` a level, from the ability or its values.
    pub channel: &'static [Ms],
    /// `AbilityCooldown` a level, from the ability or its values.
    pub cooldown: &'static [Ms],
    /// `AbilityManaCost` a level, from the ability or its values.
    pub mana: &'static [i32],
    /// `AbilityUnitDamageType`.
    pub damage: Option<DamageKind>,
    /// `SpellImmunityType`: whether it reaches a debuff-immune enemy.
    pub pierces_immunity: Option<bool>,
    /// `SpellDispellableType`.
    pub dispel: Dispel,
    /// `AbilityUnitTargetTeam`.
    pub target_team: Option<TargetTeam>,
    /// `AbilityUnitTargetType`.
    pub target_types: UnitTargets,
    /// `AbilityUnitTargetFlags`.
    pub target_flags: TargetFlags,
    /// `AbilityValues`, by Dota's key.
    pub values: &'static [AbilityValue],
}

/// Whose units an ability is aimed at: Dota's `AbilityUnitTargetTeam`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetTeam {
    Enemy,
    Friendly,
    Both,
}

/// Dota's `AbilityUnitTargetType` flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitTargets(u32);

impl UnitTargets {
    pub const NONE: UnitTargets = UnitTargets(0);
    pub const HERO: UnitTargets = UnitTargets(1 << 0);
    pub const CREEP: UnitTargets = UnitTargets(1 << 1);
    pub const BUILDING: UnitTargets = UnitTargets(1 << 2);
    pub const COURIER: UnitTargets = UnitTargets(1 << 4);
    pub const OTHER: UnitTargets = UnitTargets(1 << 5);
    pub const TREE: UnitTargets = UnitTargets(1 << 6);
    /// Dota's `DOTA_UNIT_TARGET_BASIC`: creeps and couriers.
    pub const BASIC: UnitTargets = UnitTargets(Self::CREEP.0 | Self::COURIER.0);

    /// Both sets of flags.
    pub const fn and(self, other: UnitTargets) -> UnitTargets {
        UnitTargets(self.0 | other.0)
    }

    /// Whether the two sets are the same.
    pub const fn is(self, other: UnitTargets) -> bool {
        self.0 == other.0
    }
}

/// Dota's `AbilityUnitTargetFlags`, those an imported ability carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetFlags(u32);

impl TargetFlags {
    pub const NONE: TargetFlags = TargetFlags(0);
    pub const MAGIC_IMMUNE_ENEMIES: TargetFlags = TargetFlags(1 << 0);

    /// Both sets of flags.
    pub const fn and(self, other: TargetFlags) -> TargetFlags {
        TargetFlags(self.0 | other.0)
    }

    /// Whether every flag of `flags` is set.
    pub const fn has(self, flags: TargetFlags) -> bool {
        self.0 & flags.0 == flags.0
    }
}

/// One of an ability's `AbilityValues`: a number a level, and what talents and upgrades make
/// of it.
#[derive(Clone, Copy, Debug)]
pub struct AbilityValue {
    /// Its Dota key.
    pub name: &'static str,
    /// One number a level; a shorter list holds its last number for the levels past it.
    pub levels: &'static [Dec],
    /// What a talent (`special_bonus_unique_pudge_7`), Aghanim's Scepter
    /// (`special_bonus_scepter`) or Shard (`special_bonus_shard`) makes of it, by its key.
    pub bonuses: &'static [(&'static str, Bonus)],
}

/// What a talent or an upgrade makes of a value, as a script writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bonus {
    /// `+150`, `+-10`, or a bare `-4`: so much more, a level.
    Add(&'static [Dec]),
    /// `x1.5`: so many times as much.
    Times(Dec),
    /// `=80 =120`: this instead, a level.
    Set(&'static [Dec]),
    /// `+15%`: so many percent more.
    Percent(Dec),
}

impl AbilityData {
    /// The value of `key`. Fails the build when the ability has none.
    pub const fn value(&self, key: &str) -> &'static AbilityValue {
        let mut at = 0;
        while at < self.values.len() {
            if str_eq(self.values[at].name, key) {
                return &self.values[at];
            }
            at += 1;
        }
        panic!("the ability has no such value");
    }
}

/// The imported row of the ability `key`: an imported hero's ability, or a generic talent.
pub fn ability_data(key: &str) -> Option<&'static AbilityData> {
    ABILITY_ROWS.iter().copied().find(|row| row.key == key)
}

impl AbilityData {
    /// The value of `key`, if the ability has one.
    pub fn find(&self, key: &str) -> Option<&'static AbilityValue> {
        self.values.iter().find(|value| value.name == key)
    }
}

/// The number of a level list at `level`, counted from zero; a shorter list holds its last.
const fn level_of(levels: &[Dec], level: usize) -> Dec {
    assert!(!levels.is_empty(), "a value without a number");
    let last = levels.len() - 1;
    levels[if level < last { level } else { last }]
}

impl AbilityValue {
    /// The number at level `level`, counted from zero.
    pub const fn at(&self, level: usize) -> Dec {
        level_of(self.levels, level)
    }

    /// The number at `level` with the bonus of every key `learned` holds for applied in the
    /// script's order: what a hero that has learned those talents gets.
    pub fn talented(&self, level: usize, learned: impl Fn(&str) -> bool) -> Dec {
        self.bonuses
            .iter()
            .filter(|(key, _)| learned(key))
            .fold(self.at(level), |value, (_, bonus)| {
                bonus.applied(value, level)
            })
    }

    /// The first level's number.
    pub const fn first(&self) -> Dec {
        self.at(0)
    }

    /// The whole number a level, for `N` levels.
    pub const fn ints<const N: usize>(&self) -> [i32; N] {
        let mut out = [0; N];
        let mut level = 0;
        while level < N {
            out[level] = self.at(level).int();
            level += 1;
        }
        out
    }

    /// The span in ticks a level, for `N` levels, the number read as seconds.
    pub const fn ticks<const N: usize>(&self) -> [u32; N] {
        let mut out = [0; N];
        let mut level = 0;
        while level < N {
            out[level] = self.at(level).secs().ticks();
            level += 1;
        }
        out
    }

    /// The number a level as bota's fixed point, for `N` levels.
    pub const fn fixeds<const N: usize>(&self) -> [Fixed; N] {
        let mut out = [Fixed::ZERO; N];
        let mut level = 0;
        while level < N {
            out[level] = fixed(self.at(level));
            level += 1;
        }
        out
    }

    /// What `bonus` (a talent's or an upgrade's key) makes of it. Fails the build when it has
    /// no such bonus.
    pub const fn bonus(&self, bonus: &str) -> Bonus {
        let mut at = 0;
        while at < self.bonuses.len() {
            if str_eq(self.bonuses[at].0, bonus) {
                return self.bonuses[at].1;
            }
            at += 1;
        }
        panic!("the value has no such bonus");
    }
}

impl Bonus {
    /// What it makes of `value` at `level`, counted from zero.
    pub const fn applied(self, value: Dec, level: usize) -> Dec {
        match self {
            Bonus::Add(levels) => Dec(value.0 + level_of(levels, level).0),
            Bonus::Times(factor) => Dec((value.0 as i64 * factor.0 as i64 / 1000) as i32),
            Bonus::Set(levels) => level_of(levels, level),
            Bonus::Percent(pct) => Dec(value.0 + (value.0 as i64 * pct.0 as i64 / 100_000) as i32),
        }
    }

    /// What an [`Add`](Bonus::Add) adds at its first level. Fails the build for another kind.
    pub const fn added(self) -> Dec {
        match self {
            Bonus::Add(levels) => {
                assert!(levels.len() == 1, "an addition that differs by level");
                levels[0]
            }
            _ => panic!("not an addition"),
        }
    }

    /// The factor of a [`Times`](Bonus::Times). Fails the build for another kind.
    pub const fn factor(self) -> Dec {
        match self {
            Bonus::Times(factor) => factor,
            _ => panic!("not a factor"),
        }
    }
}

/// A script's number as bota's fixed point.
pub const fn fixed(value: Dec) -> Fixed {
    Fixed::from_ratio(value.0, 1000)
}

/// A rate a second as so much a tick.
pub const fn per_tick_of(value: Dec) -> Fixed {
    Fixed::from_ratio(value.0, 1000 * rules::TICKS_PER_SECOND as i32)
}

/// Every span of a level list in ticks, for `N` levels.
pub const fn ticks_of<const N: usize>(levels: &[Ms]) -> [u32; N] {
    assert!(!levels.is_empty(), "a span without a number");
    let mut out = [0; N];
    let mut level = 0;
    while level < N {
        let at = if level < levels.len() {
            level
        } else {
            levels.len() - 1
        };
        out[level] = levels[at].ticks();
        level += 1;
    }
    out
}

/// Every number of a level list, for `N` levels.
pub const fn ints_of<const N: usize>(levels: &[i32]) -> [i32; N] {
    assert!(!levels.is_empty(), "a list without a number");
    let mut out = [0; N];
    let mut level = 0;
    while level < N {
        out[level] = levels[if level < levels.len() {
            level
        } else {
            levels.len() - 1
        }];
        level += 1;
    }
    out
}

/// A `MovementTurnRate` (radians per 0.03 seconds) in brads a tick, rounded up: how
/// [`rules::TURN_RATE_BRADS`] reads 0.5.
pub const fn turn_rate_brads(rate: Dec) -> u16 {
    // Brads a radian, 65536 / 2π, in billionths.
    const BRADS_PER_RADIAN_E9: u64 = 10_430_378_350_470;
    assert!(rate.0 > 0, "a unit that cannot turn");
    // Thousandths of a radian per 0.03 seconds, over 30 of those a second, are radians a tick
    // over 900 at 30 ticks a second.
    let per_tick_e9 = rate.0 as u64 * BRADS_PER_RADIAN_E9 / 30 / rules::TICKS_PER_SECOND as u64;
    let brads = per_tick_e9.div_ceil(1_000_000_000);
    assert!(
        brads <= u16::MAX as u64,
        "a turn past a whole circle a tick"
    );
    brads as u16
}

const _: () = {
    assert!(turn_rate_brads(Dec::of("0.5")) == rules::TURN_RATE_BRADS);
    assert!(turn_rate_brads(Dec::of("0.9")) == rules::FAST_TURN_RATE_BRADS);
    assert!(turn_rate_brads(Dec::of("1")) == rules::TOWER_TURN_RATE_BRADS);
};

/// Whether the imported table holds a row of the ability `key`.
const fn has_row(key: &str) -> bool {
    let mut at = 0;
    while at < ABILITY_ROWS.len() {
        if str_eq(ABILITY_ROWS[at].key, key) {
            return true;
        }
        at += 1;
    }
    false
}

/// The imported table is whole: every ability a hero's slots name has its row, and no two rows
/// share a key.
const _: () = {
    let mut hero = 0;
    while hero < HERO_ROWS.len() {
        let mut slot = 0;
        while slot < HERO_ROWS[hero].abilities.len() {
            assert!(
                has_row(HERO_ROWS[hero].abilities[slot]),
                "a slotted ability has no row"
            );
            slot += 1;
        }
        hero += 1;
    }
    let mut one = 0;
    while one < ABILITY_ROWS.len() {
        let mut other = one + 1;
        while other < ABILITY_ROWS.len() {
            assert!(!str_eq(ABILITY_ROWS[one].key, ABILITY_ROWS[other].key));
            other += 1;
        }
        one += 1;
    }
};
