//! What the neutral creeps' scripts do not hold, in ONE place: numbers bota plays by that no
//! script of the install carries (an animation's backswing, how many blows a passive counts), each
//! with the place it comes from, and the defaults Dota gives a key a script leaves out. Nothing
//! is defaulted or filled in anywhere else: a script that lacks a key bota reads, and no entry
//! here says what to do about it, fails the import naming the unit or ability and the key. An
//! entry the install holds already is refused, so the table never outlives its reason; one
//! naming a unit or ability bota does not carry is refused too. A backswing nobody knows is an
//! entry that says so.

use std::fmt::Write;

use crate::dec;

/// Which units an override is for.
#[derive(Clone, Copy, Debug)]
pub enum Units {
    /// These, by their keys.
    Named(&'static [&'static str]),
    /// Every unit that attacks and no other entry of the same number names.
    OtherAttackers,
}

/// One number of a unit bota plays by, beside its script's.
pub struct UnitNumber {
    pub units: Units,
    /// The key it is stored under, as the generated table lists it.
    pub name: &'static str,
    /// As a script writes it.
    pub value: &'static str,
    /// Where the number comes from.
    pub evidence: &'static str,
}

/// One number of an ability bota plays by, that its script does not hold.
pub struct AbilityNumber {
    pub ability: &'static str,
    /// The value's name, as the generated table lists it.
    pub name: &'static str,
    /// One number a level, as a script writes them.
    pub levels: &'static str,
    pub evidence: &'static str,
}

/// The animation table of Liquipedia's *Attack Animation* article: the npc files carry no
/// backswing.
const BACKSWING: &str = "Liquipedia, Attack Animation: the neutral creeps' backswings (the npc \
                         files carry none)";

/// Where a cast's backswing comes from: the ability scripts carry none.
const CAST_BACKSWING: &str = "Liquipedia, the article of the creep carrying it (read 2026-10-18): the \
                              ability scripts carry no backswing";

/// Where a number the ability scripts leave out comes from, when no more precise source is known.
const LIQUIPEDIA: &str = "Liquipedia, the article of the creep carrying it (read 2026-10-18): \
                          the ability scripts hold no such number";

/// A backswing that no source gives.
const UNKNOWN_BACKSWING: &str =
    "No sourced recovery interval is available. The cast adds no recovery animation";

/// A number bota's table kept that no source of its own says where it came from.
const CARRIED: &str = "carried over from the hand-written table, which recorded no source";

/// A default Dota gives a unit key a script leaves out.
pub struct UnitDefault {
    pub name: &'static str,
    pub value: &'static str,
    pub evidence: &'static str,
}

/// Every unit default: written into the row of a unit whose script lacks the key.
pub const UNIT_DEFAULTS: &[UnitDefault] = &[
    UnitDefault {
        name: "BaseAttackSpeed",
        value: "100",
        evidence: "Dota's base attack speed of a unit that names none",
    },
    UnitDefault {
        name: "MovementTurnRate",
        value: "0.5",
        evidence: "Dota's turn rate of a unit that names none",
    },
    UnitDefault {
        name: "MagicalResistance",
        value: "0",
        evidence: "Dota's magic resistance of a unit that names none",
    },
    UnitDefault {
        name: "IsAncient",
        value: "0",
        evidence: "a unit is no ancient unless its script says so",
    },
];

/// Which abilities an ability default is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    /// Every ability.
    All,
    /// The abilities that are cast: those whose behavior is not passive.
    Casts,
}

/// A default Dota gives an ability key a script leaves out.
pub struct AbilityDefault {
    pub which: Which,
    pub name: &'static str,
    pub value: &'static str,
    pub evidence: &'static str,
}

/// Every ability default: written into the row of an ability whose script lacks the key.
pub const ABILITY_DEFAULTS: &[AbilityDefault] = &[
    AbilityDefault {
        which: Which::Casts,
        name: "AbilityCastPoint",
        value: "0",
        evidence: "a cast without a cast point goes off at once",
    },
    AbilityDefault {
        which: Which::Casts,
        name: "AbilityCastRange",
        value: "0",
        evidence: "a cast without a cast range is aimed at nothing at a distance",
    },
    AbilityDefault {
        which: Which::All,
        name: "IsBreakable",
        value: "0",
        evidence: "an ability is not silenced by a Break unless its script says so",
    },
];

/// Roshan's ability scripts, whose rows `boss.rs` writes with these overrides applied.
pub const ROSHAN_ABILITIES: &[&str] = &[
    "roshan_devotion",
    "roshan_inherent_buffs",
    "roshan_spell_block",
    "roshan_bash",
    "roshan_slam",
    "roshan_revengeroar",
    "roshan_grab_and_throw",
];

/// A number a unit's script holds that nothing of bota's reads, named so that the importer does not
/// refuse it: a melee unit's missile speed, which no attack of it launches.
pub struct Ignored {
    pub unit: &'static str,
    pub name: &'static str,
    /// The text of the script, exactly: a patch that changes it fails the import again.
    pub script: &'static str,
    pub evidence: &'static str,
}

/// Every ignored unit number.
pub const IGNORED: &[Ignored] = &[Ignored {
    unit: "npc_dota_neutral_tadpole",
    name: "ProjectileSpeed",
    script: "1500",
    evidence: "the Pollywog attacks in melee: its script holds the missile speed of the Marshmages, \
               which no attack of it launches",
}];

/// Whether an entry ignores `script` as the `name` of `unit`.
pub fn ignored(unit: &str, name: &str, script: &str) -> bool {
    IGNORED
        .iter()
        .any(|i| i.unit == unit && i.name == name && i.script == script)
}

/// A unit number a script writes finer than a thousandth, with the thousandth bota plays.
pub struct Rounded {
    pub unit: &'static str,
    pub name: &'static str,
    /// The text of the script, exactly: a patch that changes it fails the import again.
    pub script: &'static str,
    /// The number played, as a script writes numbers.
    pub value: &'static str,
    pub evidence: &'static str,
}

/// Every rounded unit number.
pub const ROUNDED: &[Rounded] = &[
    Rounded {
        unit: "npc_dota_neutral_ogre_seal",
        name: "AttackAnimationPoint",
        script: "0.5714",
        value: "0.571",
        evidence: "the attack point of 4/7 s is 571.4 ms, played in whole milliseconds",
    },
    Rounded {
        unit: "npc_dota_neutral_ogre_seal_pup",
        name: "AttackAnimationPoint",
        script: "0.5714",
        value: "0.571",
        evidence: "the attack point of 4/7 s is 571.4 ms, played in whole milliseconds",
    },
];

/// The number `script` is for `name` of `unit`, if an entry rounds it.
pub fn rounded(unit: &str, name: &str, script: &str) -> Option<&'static str> {
    ROUNDED
        .iter()
        .find(|r| r.unit == unit && r.name == name && r.script == script)
        .map(|r| r.value)
}

/// Every unit override.
pub const UNIT_NUMBERS: &[UnitNumber] = &[
    UnitNumber {
        units: Units::Named(&[
            "npc_dota_neutral_kobold",
            "npc_dota_neutral_kobold_tunneler",
            "npc_dota_neutral_kobold_taskmaster",
        ]),
        name: "AttackBackswing",
        value: "0.6",
        evidence: BACKSWING,
    },
    UnitNumber {
        units: Units::Named(&["npc_dota_neutral_giant_wolf", "npc_dota_neutral_alpha_wolf"]),
        name: "AttackBackswing",
        value: "0.64",
        evidence: BACKSWING,
    },
    UnitNumber {
        units: Units::Named(&[
            "npc_dota_neutral_black_drake",
            "npc_dota_neutral_black_dragon",
            "npc_dota_neutral_small_thunder_lizard",
            "npc_dota_neutral_big_thunder_lizard",
        ]),
        name: "AttackBackswing",
        value: "0.56",
        evidence: BACKSWING,
    },
    UnitNumber {
        units: Units::Named(&["npc_dota_neutral_granite_golem"]),
        name: "AttackBackswing",
        value: "0.5",
        evidence: BACKSWING,
    },
    UnitNumber {
        units: Units::Named(&["npc_dota_dark_troll_warlord_skeleton_warrior"]),
        name: "AttackBackswing",
        value: "0.44",
        evidence: BACKSWING,
    },
    UnitNumber {
        units: Units::OtherAttackers,
        name: "AttackBackswing",
        value: "0.3",
        evidence: "Liquipedia, Attack Animation: the 0.3 s most neutral creeps share, for a kind \
                   its table lacks",
    },
];

/// Retired content, frozen at its last live values: an ability a patch took off its units (or
/// whose key the patch gave to another ability), kept with its code, its id and its tests so that
/// it can be given to a unit again. Its row is written from here, not read from the install,
/// whose row of that key is the new patch's.
pub struct Retired {
    /// The registry key of its id: Dota's, or a `bota_` key once Dota's names another ability.
    pub ability: &'static str,
    /// Its numbers at the last live patch, each as a script writes it.
    pub values: &'static [(&'static str, &'static str)],
    /// Where the values come from, and what retired it.
    pub evidence: &'static str,
}

/// Every retired ability.
pub const RETIRED: &[Retired] = &[
    Retired {
        ability: "berserker_troll_break",
        values: &[
            ("MaxLevel", "1"),
            ("AbilityCooldown", "10 9 8 6"),
            ("IsBreakable", "1"),
            ("duration", "3"),
        ],
        evidence: "Dota 7.41f, the script row of berserker_troll_break, a passive that breaks \
                   the target of an attack; Dota 7.42 removed it from the Hill Troll Berserker \
                   (\"Removed Break Ability\") and reused the key for another ability",
    },
    Retired {
        ability: "kobold_disarm",
        values: &[
            ("MaxLevel", "1"),
            ("IsBreakable", "1"),
            ("AbilityCooldown", "9"),
            ("duration", "3 3.5 4 5"),
            ("hits", "3"),
        ],
        evidence: "Dota 7.41f, the script row of kobold_disarm, a passive that disarms on the \
                   third blow (its `hits` is Liquipedia's Kobold Soldier article, read 2026-10-18: the \
                   row holds none); Dota 7.42 reworked the ability into a cast and kept the key",
    },
    Retired {
        ability: "warpine_raider_seed_shot",
        values: &[
            ("AbilityManaCost", "100"),
            ("AbilityCooldown", "15"),
            ("AbilityCastRange", "575"),
            ("AbilityCastPoint", "0.2"),
            ("AbilityCastBackswing", "0"),
        ],
        evidence: "Seed Shot's former cast timing and cost. Snowball Shot uses a separate identity.",
    },
    Retired {
        ability: "black_dragon_fireball",
        values: &[
            ("AbilityManaCost", "200"),
            ("AbilityCooldown", "15"),
            ("AbilityCastRange", "1000"),
            ("AbilityCastPoint", "0.3"),
            ("AbilityCastBackswing", "1.1"),
        ],
        evidence: "Fireball's former cast timing and cost. Searing Swoop uses a separate identity.",
    },
];

/// Every ability override.
pub const ABILITY_NUMBERS: &[AbilityNumber] = &[
    AbilityNumber {
        ability: "radigull_eggstremism",
        name: "AbilityCastBackswing",
        levels: "0",
        evidence: UNKNOWN_BACKSWING,
    },
    AbilityNumber {
        ability: "eimermole_miners_slam",
        name: "AbilityCastBackswing",
        levels: "0",
        evidence: UNKNOWN_BACKSWING,
    },
    AbilityNumber {
        ability: "ogre_seal_flop",
        name: "AbilityCastBackswing",
        levels: "0",
        evidence: UNKNOWN_BACKSWING,
    },
    AbilityNumber {
        ability: "kobold_disarm",
        name: "AbilityCastBackswing",
        levels: "0",
        evidence: UNKNOWN_BACKSWING,
    },
    AbilityNumber {
        ability: "mud_golem_hurl_big_boulder",
        name: "AbilityCastBackswing",
        levels: "0",
        evidence: UNKNOWN_BACKSWING,
    },
    AbilityNumber {
        ability: "ogre_bruiser_ogre_smash",
        name: "reach",
        levels: "200",
        evidence: LIQUIPEDIA,
    },
    AbilityNumber {
        ability: "enraged_wildkin_hurricane",
        name: "speed",
        levels: "800",
        evidence: LIQUIPEDIA,
    },
    AbilityNumber {
        ability: "frogmen_congregation_of_the_deep",
        name: "spread_angle",
        levels: "60",
        evidence: LIQUIPEDIA,
    },
    AbilityNumber {
        ability: "frogmen_arm_of_the_deep",
        name: "neutral_shared_radius",
        levels: "1200",
        evidence: "Dota 7.41 patch notes: the copies of the frogs' spikes are put on cooldown \
                    on other neutrals within 1200 range",
    },
    AbilityNumber {
        ability: "frogmen_water_bubble_small",
        name: "heal_pct",
        levels: "0",
        evidence: "the Marshmage Apprentice's bubble mends nothing: its script has no heal_pct",
    },
    AbilityNumber {
        ability: "frogmen_water_bubble_small",
        name: "radius",
        levels: "0",
        evidence: "the Marshmage Apprentice's bubble mends no one else: its script has no radius",
    },
    AbilityNumber {
        ability: "frogmen_water_bubble_medium",
        name: "radius",
        levels: "0",
        evidence: "the Marshmage's bubble mends no one else: its script has no radius",
    },
    AbilityNumber {
        ability: "harpy_scout_take_off",
        name: "AbilityCastBackswing",
        levels: "0",
        evidence: UNKNOWN_BACKSWING,
    },
    AbilityNumber {
        ability: "enraged_wildkin_tornado",
        name: "AbilityCastBackswing",
        levels: "0",
        evidence: UNKNOWN_BACKSWING,
    },
    AbilityNumber {
        ability: "warpine_raider_seed_shot",
        name: "AbilityCastBackswing",
        levels: "0",
        evidence: UNKNOWN_BACKSWING,
    },
    AbilityNumber {
        ability: "frogmen_arm_of_the_deep",
        name: "AbilityCastBackswing",
        levels: "0",
        evidence: UNKNOWN_BACKSWING,
    },
    AbilityNumber {
        ability: "frogmen_tendrils_of_the_deep",
        name: "AbilityCastBackswing",
        levels: "0",
        evidence: UNKNOWN_BACKSWING,
    },
    AbilityNumber {
        ability: "roshan_slam",
        name: "AbilityCastBackswing",
        levels: "0.57",
        evidence: CARRIED,
    },
    AbilityNumber {
        ability: "roshan_grab_and_throw",
        name: "AbilityCastBackswing",
        levels: "0.53",
        evidence: CARRIED,
    },
    AbilityNumber {
        ability: "roshan_revengeroar",
        name: "AbilityCastBackswing",
        levels: "0",
        evidence: UNKNOWN_BACKSWING,
    },
    AbilityNumber {
        ability: "forest_troll_high_priest_heal",
        name: "AbilityCastBackswing",
        levels: "0.7",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "fel_beast_haunt",
        name: "AbilityCastBackswing",
        levels: "0.5",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "harpy_storm_chain_lightning",
        name: "AbilityCastBackswing",
        levels: "0.67",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "centaur_khan_war_stomp",
        name: "AbilityCastBackswing",
        levels: "0.57",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "giant_wolf_intimidate",
        name: "AbilityCastBackswing",
        levels: "0.8",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "satyr_trickster_purge",
        name: "AbilityCastBackswing",
        levels: "0.8",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "satyr_soulstealer_mana_burn",
        name: "AbilityCastBackswing",
        levels: "0.8",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "satyr_hellcaller_shockwave",
        name: "AbilityCastBackswing",
        levels: "0.83",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "ogre_bruiser_ogre_smash",
        name: "AbilityCastBackswing",
        levels: "0.85",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "ogre_magi_frost_armor",
        name: "AbilityCastBackswing",
        levels: "0.5",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "mud_golem_hurl_boulder",
        name: "AbilityCastBackswing",
        levels: "0.6",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "polar_furbolg_ursa_warrior_thunder_clap",
        name: "AbilityCastBackswing",
        levels: "0.6",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "enraged_wildkin_hurricane",
        name: "AbilityCastBackswing",
        levels: "0.8",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "dark_troll_warlord_ensnare",
        name: "AbilityCastBackswing",
        levels: "0.36",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "dark_troll_warlord_raise_dead",
        name: "AbilityCastBackswing",
        levels: "1.2",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "black_dragon_fireball",
        name: "AbilityCastBackswing",
        levels: "1.1",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "big_thunder_lizard_slam",
        name: "AbilityCastBackswing",
        levels: "0.43",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "big_thunder_lizard_frenzy",
        name: "AbilityCastBackswing",
        levels: "0.8",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "ice_shaman_incendiary_bomb",
        name: "AbilityCastBackswing",
        levels: "0.65",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "spawnlord_master_stomp",
        name: "AbilityCastBackswing",
        levels: "1.37",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "frogmen_congregation_of_the_deep",
        name: "AbilityCastBackswing",
        levels: "0.2",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "frogmen_water_bubble_small",
        name: "AbilityCastBackswing",
        levels: "1.1",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "frogmen_water_bubble_medium",
        name: "AbilityCastBackswing",
        levels: "1.1",
        evidence: CAST_BACKSWING,
    },
    AbilityNumber {
        ability: "frogmen_water_bubble_large",
        name: "AbilityCastBackswing",
        levels: "1.1",
        evidence: CAST_BACKSWING,
    },
];

/// The overrides of `unit`, those of `OtherAttackers` only when it `attacks`.
pub fn unit_numbers(unit: &str, attacks: bool) -> Vec<&'static UnitNumber> {
    let named = |number: &UnitNumber| match number.units {
        Units::Named(units) => units.contains(&unit),
        Units::OtherAttackers => false,
    };
    UNIT_NUMBERS
        .iter()
        .filter(|number| match number.units {
            Units::Named(_) => named(number),
            Units::OtherAttackers => {
                attacks
                    && !UNIT_NUMBERS
                        .iter()
                        .any(|other| other.name == number.name && named(other))
            }
        })
        .collect()
}

/// Refuses an override that names a unit bota does not carry.
pub fn check_units(units: &[String]) -> Result<(), String> {
    for r in ROUNDED {
        if !units.iter().any(|carried| carried == r.unit) {
            return Err(format!(
                "a rounding of {} names {}, which bota does not carry",
                r.name, r.unit
            ));
        }
    }
    for i in IGNORED {
        if !units.iter().any(|carried| carried == i.unit) {
            return Err(format!(
                "an ignored {} names {}, which bota does not carry",
                i.name, i.unit
            ));
        }
    }
    for number in UNIT_NUMBERS {
        if let Units::Named(named) = number.units {
            for unit in named {
                if !units.iter().any(|carried| carried == unit) {
                    return Err(format!(
                        "an override of {} names {unit}, which bota does not carry",
                        number.name
                    ));
                }
            }
        }
        dec::milli(number.value)?;
    }
    Ok(())
}

/// The retired abilities, as lines of the generated table's header.
pub fn describe_retired() -> String {
    let mut out = String::new();
    for retired in RETIRED {
        let _ = writeln!(
            out,
            "//! - retired {}, frozen: {}",
            retired.ability, retired.evidence
        );
    }
    out
}

/// The name of the constant of a retired ability's row.
pub fn retired_constant(ability: &str) -> String {
    format!("RETIRED_{}", ability.to_uppercase())
}

/// Refuses an override that names an ability no carried unit holds.
pub fn check_abilities(abilities: &[&str]) -> Result<(), String> {
    for number in ABILITY_NUMBERS {
        if !abilities.contains(&number.ability) && !ROSHAN_ABILITIES.contains(&number.ability) {
            return Err(format!(
                "an override of {} names {}, which no neutral holds",
                number.name, number.ability
            ));
        }
    }
    Ok(())
}

/// The overrides of the ability `key`.
pub fn ability_numbers(key: &str) -> impl Iterator<Item = &'static AbilityNumber> + '_ {
    ABILITY_NUMBERS
        .iter()
        .filter(move |number| number.ability == key)
}

/// The overrides as the lines of the generated table's header.
pub fn describe() -> String {
    let mut out = String::new();
    for r in ROUNDED {
        let _ = writeln!(
            out,
            "//! - {}: {} {} played as {}: {}",
            r.unit, r.name, r.script, r.value, r.evidence
        );
    }
    for i in IGNORED {
        let _ = writeln!(
            out,
            "//! - {}: {} {} is ignored: {}",
            i.unit, i.name, i.script, i.evidence
        );
    }
    for default in UNIT_DEFAULTS {
        let _ = writeln!(
            out,
            "//! - default of a unit without it: {} {}: {}",
            default.name, default.value, default.evidence
        );
    }
    for default in ABILITY_DEFAULTS {
        let _ = writeln!(
            out,
            "//! - default of {} without it: {} {}: {}",
            match default.which {
                Which::All => "an ability",
                Which::Casts => "a cast",
            },
            default.name,
            default.value,
            default.evidence
        );
    }
    for number in UNIT_NUMBERS {
        let units = match number.units {
            Units::Named(units) => units.join(", "),
            Units::OtherAttackers => "every other attacker".to_owned(),
        };
        let _ = writeln!(
            out,
            "//! - {units}: {} {}: {}",
            number.name, number.value, number.evidence
        );
    }
    for number in ABILITY_NUMBERS {
        let _ = writeln!(
            out,
            "//! - {}: {} {}: {}",
            number.ability, number.name, number.levels, number.evidence
        );
    }
    out
}

/// Units kept in the catalogue while art or the gold spawn rule is unavailable.
pub fn held(key: &str) -> bool {
    key.ends_with("_gold")
        || matches!(
            key,
            "npc_dota_neutral_radigull"
                | "npc_dota_neutral_odobenus_footman"
                | "npc_dota_neutral_odobenus_frostmage"
                | "npc_dota_neutral_eimermole"
                | "npc_dota_neutral_eimermole_foreman"
                | "npc_dota_neutral_ogre_seal"
                | "npc_dota_neutral_ogre_seal_pup"
        )
}
