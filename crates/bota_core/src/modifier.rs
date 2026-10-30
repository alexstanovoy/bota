//! What a modifier does to whatever wears it, and how much of it there is.

use bota_proto::{DamageKind, EffectId, Fixed, StatusFlags, Team};

use crate::{Act, Buff, NeutralAbility, Spell, Strike, Then, Throe, WATER_BUBBLE_EFFECT, rules};

/// One kind of modifier, with what there is of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierKind {
    /// Health mends faster.
    Mending {
        /// Points a tick, rounded up to the raw unit so a whole drink gives
        /// the points the item names.
        per_tick: Fixed,
        /// Whether a blow that [`breaks_regen`](crate::breaks_regen) puts it out.
        breaks: bool,
    },
    /// Mana mends faster.
    Clarity {
        /// Points a tick, rounded up to the raw unit so a whole drink gives
        /// the points the item names.
        per_tick: Fixed,
        /// Whether a blow that [`breaks_regen`](crate::breaks_regen) puts it out.
        breaks: bool,
    },
    /// Health and mana both mend faster, for standing in a fountain.
    Fountain {
        /// Percent of the maximum health mended a second.
        hp_pct: i32,
        /// Percent of the maximum mana mended a second.
        mana_pct: i32,
    },
    /// Cannot act at all: it neither walks, nor turns, nor swings, nor casts.
    Stunned,
    /// Nothing gets through: damage passes it by while it holds.
    Shielded,
    /// Walks through the bodies in its way, and nothing eases it out of one.
    Phased,
    /// Flies: closed ground, trees and bodies are nothing to it, and it sees
    /// past height and trees. Dota's `modifier_courier_flying`, which a
    /// courier wears from its owner's [`rules::COURIER_FLYING_LEVEL`] on.
    Flying,
    /// Invulnerable and untargetable: a hero risen at its fountain
    /// (`modifier_fountain_invulnerability`), "until the respawning hero is
    /// issued a command" (Liquipedia, *Hero (unit type)*, Respawning), and
    /// while the fountain's Rejuvenation Aura, which provides it, holds on it.
    FountainInvulnerable,
    /// The fountain's Fountain Damage, as Dota's `modifier_fountain_passive`
    /// (Liquipedia, *Fountain*): every shot of it first stacks
    /// [`ModifierKind::FountainDamage`] on what it lands on and deals
    /// [`rules::FOUNTAIN_STACK_DAMAGE`] more by the stack, splashes
    /// [`rules::FOUNTAIN_SPLASH_PCT`] percent of itself on every other enemy
    /// within [`rules::FOUNTAIN_SPLASH_RADIUS`], ward and building aside, and
    /// [`rules::FOUNTAIN_ACCURACY_PCT`] percent of its shots go through
    /// evasion.
    FountainPassive,
    /// The fountain's shots stacked on a unit, as Dota's
    /// `modifier_fountain_fury_swipes_damage_increase`: each landed shot adds
    /// one and refreshes them all to [`rules::FOUNTAIN_STACK_TICKS`].
    FountainDamage {
        /// Shots stacked, one at least.
        stacks: u32,
    },
    /// Walks slower, by `pct` percent of what it would.
    Slowed {
        /// Percent taken off its speed.
        pct: i32,
    },
    /// Walks faster, by `pct` percent of what it would.
    Hastened {
        /// Percent added to its speed.
        pct: i32,
    },
    /// Dota's hasted movement: walks faster by `pct` percent of what it
    /// would, and no slow takes anything from it while it holds
    /// (Liquipedia, Courier, Speed Burst: "Grants the Courier hasted
    /// movement, meaning it cannot be slowed while affected").
    Haste {
        /// Percent added to its speed.
        pct: i32,
    },
    /// Wears less armor for standing near a dark presence.
    ArmorBroken {
        /// Armor taken off.
        armor: Fixed,
    },
    /// Keeps less magic out for having been crossed by a requiem: what its
    /// resistance lets through grows by `pct` percent.
    MagicBroken {
        /// Percent added to the share of magic damage let through.
        pct: i32,
    },
    /// Wears more armor and mends faster for standing by a tower of its own.
    Guarded {
        /// Armor added, in whole points.
        armor: i32,
        /// Health mended, in hundredths of a point a second.
        hp_per_second: i32,
    },
    /// Mends faster and shrugs off more magic for marching beside the one
    /// carrying the flag.
    Inspired {
        /// Health mended, in hundredths of a point a second.
        hp_per_second: i32,
        /// Magic resistance, percent, multiplying with what is already there.
        magic_resist_pct: i32,
    },
    /// Losing health over time to whoever put it on.
    Burning {
        /// Damage each beat of [`rules::BURN_PERIOD_TICKS`].
        amount: i32,
        /// Which reduction the damage answers to.
        kind: DamageKind,
        /// Whether it may take the last point of health.
        lethal: bool,
    },
    /// Held by a dismember: bites at the beats [`crate::dismember_bite_tick`]
    /// names for as long as it runs, for the caster's level in it.
    Dismembered {
        /// The dismember's level, counted from zero.
        level: u8,
    },
    /// Takes `amount` off every blow that reaches it.
    MeatShield {
        /// Damage taken off each blow.
        amount: i32,
    },
    /// Additional damage from subsequent Shadowrazes by whoever put it on.
    Shadowraze {
        /// Successful hits held, in `1..=rules::RAZE_MAX_STACKS`.
        stacks: u8,
    },
    /// The rot, switched on, as Dota's `modifier_pudge_rot` on its owner: it
    /// burns him, never to death, and lays [`ModifierKind::Rotting`] on the
    /// enemies standing in it.
    Rot {
        /// Which level of it is running, counted from zero.
        level: u8,
    },
    /// Standing in an enemy's rot, as Dota's `modifier_pudge_rot` on them:
    /// burned on the beat by whoever's rot it is, and slowed, by the rot's
    /// level.
    Rotting {
        /// The rot's level, counted from zero.
        level: u8,
    },
    /// Runs from whoever put it on, and neither swings nor casts, until it
    /// lifts.
    Feared,
    /// Walks slower by [`rules::GRENADE_SLOW_PCT`] percent, and takes
    /// [`rules::GRENADE_DAMAGE_PER_SECOND`] magical damage from whoever put it
    /// on every whole second after `since`.
    BloodGrenade {
        /// The tick the first Blood Grenade of the run landed.
        since: u32,
    },
    /// Cannot walk; it may still swing and cast.
    Rooted,
    /// Cannot swing.
    Disarmed,
    /// Its passives do nothing, its auras included.
    Broken,
    /// What the flesh heap has kept: one for every enemy hero that fell near
    /// its bearer. Kept through death.
    FleshHeap {
        /// Heroes kept.
        count: u32,
    },
    /// The souls the necromastery has gathered, those a feast took
    /// included. Kept through death.
    Souls {
        /// Souls held.
        count: u32,
    },
    /// A feast of souls under way on its caster: he attacks and walks faster,
    /// and souls are taken from the enemies near him. Dota's basic dispel
    /// ends it.
    Feasting {
        /// The level it was cast at, counted from zero.
        level: u8,
        /// Souls it has taken so far.
        souls: u16,
    },
    /// What a feast left once it ended: the souls it took from enemies that
    /// have died since, held past the cap until it runs out.
    FeastLinger {
        /// Souls held past the cap.
        souls: u16,
    },
    /// What the clock has raised a neutral by: Dota's hidden `neutral_upgrade`
    /// modifier. Each interval adds what the neutral's `per_upgrade` says.
    NeutralUpgrade {
        /// Intervals it has been raised by.
        count: u32,
    },
    /// A unit that stands for a while, as Dota's `modifier_kill`: it is gone
    /// when this runs out.
    TimedLife,
    /// What makes a unit an illusion, as Dota's `modifier_illusion`: put on
    /// by what made it, for as long as the illusion lasts. The illusion is
    /// gone when it runs out, and only the illusion's own side is told of it.
    Illusion,
    /// A building's backdoor protection, worn while it holds: it takes
    /// [`rules::BACKDOOR_DAMAGE_PCT`] percent of the damage that reaches it
    /// and mends what enemy blows took.
    BackdoorProtection,
    /// Under its side's Glyph of Fortification (`modifier_fountain_glyph`):
    /// takes no damage. Put on the structures and lane creeps that stand
    /// when the Glyph is used, each with its own time.
    Fortified,
    /// Sight given to a side from where the wearer stands, a ground
    /// viewer of that side as long as it lasts: a Watcher lit by a hero of
    /// `side` (`modifier_lamp_on`), seeing `day` far by day and `night` by
    /// night for it.
    VisionGrant {
        /// The side it sees for.
        side: Team,
        /// How far it sees by day, in world units.
        day: i32,
        /// How far it sees by night, in world units.
        night: i32,
    },
    /// A Watcher put out (`modifier_lamp_off`): nobody may light it while
    /// it lasts.
    LampOff,
    /// An Outpost the other side may not capture yet
    /// (`modifier_watch_tower_invulnerable`), worn while every tier-two tower
    /// of its home side stands and that side holds it.
    CaptureGuard,
    /// A Shrine of Wisdom holding the experience of its last activation for
    /// whoever gathers it.
    ShrineReady,
    /// What a neutral's spell left, at the level it was cast at. `amount`
    /// is the spell's own count: the blows a Steal Weapon has counted, the
    /// magical damage a bubble has left to soak.
    Spell {
        ability: NeutralAbility,
        level: u8,
        amount: u16,
    },
}

impl ModifierKind {
    /// The number the wire names this kind of modifier with.
    pub const fn effect_id(self) -> EffectId {
        match self {
            ModifierKind::Mending { .. } => EffectId::MENDING,
            ModifierKind::Clarity { .. } => EffectId::CLARITY,
            ModifierKind::Fountain { .. } => EffectId::FOUNTAIN,
            ModifierKind::Stunned => EffectId::STUNNED,
            ModifierKind::Shielded => EffectId::SHIELDED,
            ModifierKind::Slowed { .. } => EffectId::SLOWED,
            ModifierKind::Hastened { .. } | ModifierKind::Haste { .. } => EffectId::HASTENED,
            ModifierKind::Burning { .. } => EffectId::BURNING,
            ModifierKind::Phased => EffectId::PHASED,
            ModifierKind::Flying => EffectId::FLYING,
            ModifierKind::FountainInvulnerable => EffectId::FOUNTAIN_INVULNERABLE,
            ModifierKind::FountainPassive => EffectId::FOUNTAIN_PASSIVE,
            ModifierKind::FountainDamage { .. } => EffectId::FOUNTAIN_DAMAGE,
            ModifierKind::ArmorBroken { .. } => EffectId::ARMOR_BROKEN,
            ModifierKind::MagicBroken { .. } => EffectId::MAGIC_BROKEN,
            ModifierKind::Guarded { .. } => EffectId::GUARDED,
            ModifierKind::Inspired { .. } => EffectId::INSPIRED,
            ModifierKind::Shadowraze { .. } => EffectId::SHADOWRAZE,
            ModifierKind::Rot { .. } | ModifierKind::Rotting { .. } => EffectId::ROT,
            ModifierKind::Feared => EffectId::FEARED,
            ModifierKind::BloodGrenade { .. } => EffectId::BLOOD_GRENADE,
            ModifierKind::Rooted => EffectId::ROOTED,
            ModifierKind::Disarmed => EffectId::DISARMED,
            ModifierKind::Broken => EffectId::BROKEN,
            ModifierKind::BackdoorProtection => EffectId::BACKDOOR_PROTECTION,
            ModifierKind::TimedLife => EffectId::TIMED_LIFE,
            ModifierKind::Illusion => EffectId::ILLUSION,
            ModifierKind::Fortified => EffectId::FORTIFIED,
            ModifierKind::NeutralUpgrade { .. } => EffectId::NEUTRAL_UPGRADE,
            ModifierKind::VisionGrant { .. } => EffectId::VISION_GRANT,
            ModifierKind::LampOff => EffectId::LAMP_OFF,
            ModifierKind::CaptureGuard => EffectId::CAPTURE_GUARD,
            ModifierKind::ShrineReady => EffectId::SHRINE_READY,
            ModifierKind::FleshHeap { .. } => EffectId::FLESH_HEAP,
            ModifierKind::Souls { .. } => EffectId::SOULS,
            ModifierKind::Feasting { .. } => EffectId::FEASTING,
            ModifierKind::FeastLinger { .. } => EffectId::FEAST_LINGER,
            ModifierKind::Spell { ability, .. } => match spell_effect(ability) {
                Some(effect) => effect,
                None => panic!("a spell without a modifier left one"),
            },
            ModifierKind::Dismembered { .. } => EffectId::DISMEMBERED,
            ModifierKind::MeatShield { .. } => EffectId::MEAT_SHIELD,
        }
    }

    /// Whether whatever wears it takes nobody on of its own accord, as Dota's
    /// `MODIFIER_PROPERTY_DISABLE_AUTOATTACK`: the fountain invulnerability's
    /// ("Players with the Fountain Invulnerablility buff will no longer
    /// auto-attack and break the buff when an enemy is within range", Dota's
    /// Spring Cleaning 2022).
    pub const fn disables_auto_attack(self) -> bool {
        matches!(self, ModifierKind::FountainInvulnerable)
    }

    /// The souls the necromastery holds, `count` of them.
    pub const fn souls(count: u32) -> ModifierKind {
        ModifierKind::Souls { count }
    }

    /// The flesh heap keeping `count` heroes.
    pub const fn flesh_heap(count: u32) -> ModifierKind {
        ModifierKind::FleshHeap { count }
    }

    /// The neutral upgrade, raised `count` intervals.
    pub const fn neutral_upgrade(count: u32) -> ModifierKind {
        ModifierKind::NeutralUpgrade { count }
    }

    /// The [`StatusFlags`] bits it sets on whatever wears it: what holds,
    /// slows, burns or frightens it and what keeps magic off it. A neutral's
    /// spell sets what its own modifier does ([`spell_statuses`]).
    pub fn statuses(self) -> u16 {
        match self {
            ModifierKind::Stunned => StatusFlags::STUNNED,
            ModifierKind::Shielded => StatusFlags::MAGIC_IMMUNE,
            ModifierKind::Slowed { .. } => StatusFlags::SLOWED,
            ModifierKind::Burning { .. }
            | ModifierKind::Dismembered { .. }
            | ModifierKind::Rot { .. } => StatusFlags::DOT,
            ModifierKind::Rotting { .. } => StatusFlags::SLOWED | StatusFlags::DOT,
            ModifierKind::Feared => StatusFlags::FEARED,
            ModifierKind::BloodGrenade { .. } => StatusFlags::SLOWED | StatusFlags::DOT,
            ModifierKind::Spell {
                ability,
                level,
                amount,
            } => spell_statuses(ability, level, amount),
            _ => 0,
        }
    }

    /// The count the wire shows beside it, for a kind that keeps one.
    pub const fn stacks(self) -> Option<u32> {
        match self {
            ModifierKind::Shadowraze { stacks } => Some(stacks as u32),
            ModifierKind::MeatShield { amount } => Some(amount as u32),
            ModifierKind::FleshHeap { count }
            | ModifierKind::FountainDamage { stacks: count }
            | ModifierKind::Souls { count }
            | ModifierKind::NeutralUpgrade { count } => Some(count),
            ModifierKind::Feasting { souls, .. } | ModifierKind::FeastLinger { souls } => {
                Some(souls as u32)
            }
            _ => None,
        }
    }

    /// Whether, of two instances of one aura on a unit, this one is the
    /// stronger than `other`. Two instances of one aura do not stack and the
    /// stronger applies; different auras stack (Liquipedia, *Aura*;
    /// *Buildings*: "Multiple instances of Tower Protection aura do not
    /// stack"). The stronger presence tears more armor; a tower's
    /// protection adds more armor, then more regeneration; a flagbearer's
    /// inspiration lends more magic resistance, then more regeneration; a
    /// fountain mends a larger share of health, then of mana; a neutral's
    /// spell is held at a higher level, which its values grow with. Of two
    /// equal ones neither is the stronger.
    pub fn outranks(self, other: ModifierKind) -> bool {
        match (self, other) {
            (ModifierKind::ArmorBroken { armor }, ModifierKind::ArmorBroken { armor: theirs }) => {
                armor > theirs
            }
            (
                ModifierKind::Guarded {
                    armor,
                    hp_per_second,
                },
                ModifierKind::Guarded {
                    armor: their_armor,
                    hp_per_second: their_regen,
                },
            ) => (armor, hp_per_second) > (their_armor, their_regen),
            (
                ModifierKind::Inspired {
                    hp_per_second,
                    magic_resist_pct,
                },
                ModifierKind::Inspired {
                    hp_per_second: their_regen,
                    magic_resist_pct: their_resist,
                },
            ) => (magic_resist_pct, hp_per_second) > (their_resist, their_regen),
            (
                ModifierKind::Fountain { hp_pct, mana_pct },
                ModifierKind::Fountain {
                    hp_pct: their_hp,
                    mana_pct: their_mana,
                },
            ) => (hp_pct, mana_pct) > (their_hp, their_mana),
            (
                ModifierKind::Spell { ability, level, .. },
                ModifierKind::Spell {
                    ability: theirs,
                    level: their_level,
                    ..
                },
            ) => ability == theirs && level > their_level,
            _ => false,
        }
    }

    /// What an aura of this kind hands out from a source that spawned after
    /// `steps` upgrade intervals. Only the flagbearer's inspiration grows.
    #[inline]
    pub fn grown(self, steps: u32) -> ModifierKind {
        match self {
            ModifierKind::Inspired {
                hp_per_second,
                magic_resist_pct,
            } => {
                let steps = steps.min(rules::FLAGBEARER_AURA_MAGIC_RESIST_CAP) as i32;
                ModifierKind::Inspired {
                    hp_per_second,
                    magic_resist_pct: magic_resist_pct
                        + rules::FLAGBEARER_AURA_MAGIC_RESIST_PCT * steps,
                }
            }
            other => other,
        }
    }
}

/// The effect a spell's own modifier shows, if the spell leaves one.
pub const fn spell_effect(ability: NeutralAbility) -> Option<EffectId> {
    match *ability.spell() {
        Spell::Aura(aura) => Some(aura.effect),
        Spell::Storm { effect, .. }
        | Spell::Strike(Strike::Lay { effect, .. })
        | Spell::Strike(Strike::Petrify { effect, .. })
        | Spell::Throe(Throe::Rouse { effect, .. })
        | Spell::Throe(Throe::Split { effect, .. }) => Some(effect),
        Spell::Strike(Strike::Disarm { counter, .. }) => Some(counter),
        Spell::Cast { act, .. } => match act {
            Act::Stomp {
                then: Then::Wear { effect, .. },
                ..
            }
            | Act::Enrage { effect, .. } => Some(effect),
            Act::Bubble { .. } => Some(WATER_BUBBLE_EFFECT),
            Act::Purge { .. } => Some(crate::PURGE_EFFECT),
            _ => None,
        },
        _ => None,
    }
}

/// The [`StatusFlags`] bits a neutral spell's own modifier sets at `level`:
/// SLOWED for a buff that takes movement speed (a purge's while its falling
/// `amount` holds), DOT for a strike that burns on its beat or a storm.
pub fn spell_statuses(ability: NeutralAbility, level: u8, amount: u16) -> u16 {
    let slows = match ability {
        NeutralAbility::Purge => amount > 0,
        _ => spell_buff(ability).is_some_and(|buff| buff.get(crate::Stat::MovePct, level) < 0),
    };
    let burns = match *ability.spell() {
        Spell::Strike(Strike::Lay { dps, .. }) => crate::at_level(&dps, level) > 0,
        Spell::Strike(Strike::Petrify { dps, .. }) => dps > 0,
        Spell::Storm { .. } => true,
        _ => false,
    };
    let mut bits = 0;
    if slows {
        bits |= StatusFlags::SLOWED;
    }
    if burns {
        bits |= StatusFlags::DOT;
    }
    bits
}

/// What a spell's own modifier does to the stats of whatever wears it, if
/// the spell's modifier does anything to them. A purge's slow is the
/// modifier's own `amount`, falling as it wears off.
pub const fn spell_buff(ability: NeutralAbility) -> Option<Buff> {
    match *ability.spell() {
        Spell::Aura(aura) => Some(aura.buff),
        Spell::Storm { buff, .. }
        | Spell::Strike(Strike::Lay { buff, .. })
        | Spell::Throe(Throe::Rouse { buff, .. }) => Some(buff),
        Spell::Throe(Throe::Split { slow_pct, .. }) => {
            Some(Buff::of(crate::Stat::MovePct, [-slow_pct; 4]))
        }
        Spell::Cast {
            act:
                Act::Stomp {
                    then: Then::Wear { buff, .. },
                    ..
                }
                | Act::Enrage { buff, .. },
            ..
        } => Some(buff),
        _ => None,
    }
}
