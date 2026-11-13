//! Runes and the Bottle at Dota 7.41f: where and when they appear, what each
//! rune is worth, and the Bottle that keeps one.
//!
//! Sources, tagged where each number is used: `[ENT:n]` the rune spawner
//! entities of the map's `maps/dota/entities/default_ents.vents` (7.41f),
//! shifted by half the map like every landmark; `[BIN]` a server convar
//! default of the 6966 build; `[LOC key]` the game's English tooltip;
//! `[I:n]` the lines of `scripts/npc/items.txt` (client 6966); `[SRV]` the behaviour
//! of the installed server (build 6975), where no data or note says; `[PN v]`
//! Valve's patch notes of version `v`; `[LIQ page]` Liquipedia's article
//! (read 2026-10-30), which is also where the Archive of its Bottle page
//! and its Bottle changelog are quoted. The install is newer than 7.41f; no
//! patch note after 7.41 touches a rune or the Bottle, and where a source
//! disagrees with the install's, the line says which was taken.

use bota_proto::{
    Angle, Fixed, ItemId, MAX_RUNES_AT_SPOT, MAX_SNAPSHOT_RUNES, RuneKind, RuneSet, Team, Vec2,
};

use crate::ITEMS;
use crate::rules::{self, Schedule, TICKS_PER_SECOND};

/// The kinds of rune the simulation spawns and plays: what a match may ask
/// for. The Wisdom Rune is no kind: 7.38 replaced it with the Shrines of
/// Wisdom [PN 7.38], which the objectives play.
pub const SIMULATED_RUNES: RuneSet = RuneSet::ALL;

/// The power runes: the seven kinds that appear at the river, one at a time.
pub const POWER_RUNES: RuneSet = RuneSet::ALL
    .without(RuneSet::of(RuneKind::Water))
    .without(RuneSet::of(RuneKind::Bounty));

/// The power runes the river's rotation draws from: all of [`POWER_RUNES`]
/// but the Illusion Rune, held out until its illusions are done for the
/// clients. The map still has spots for it, so a match may still ask for it.
pub const POWER_ROTATION: RuneSet = POWER_RUNES.without(RuneSet::of(RuneKind::Illusion));

/// Where a rune spot lies on the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuneSpotKind {
    /// A river spot: water and power runes appear here, and the first pair of
    /// Bounty Runes [LIQ Runes].
    River,
    /// A bounty spot in the jungle of a side.
    Jungle(Team),
}

/// One rune spot of a map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuneSpot {
    /// Where runes lie.
    pub at: Vec2,
    /// What appears here.
    pub kind: RuneSpotKind,
}

/// The Dota map's rune spots, the river first.
pub const DOTA_RUNE_SPOTS: [RuneSpot; 4] = [
    // `dota_item_rune_spawner_powerup`, runeposition "top":
    // (-1640, 1112) [ENT:216178].
    RuneSpot {
        at: Vec2::from_ints(7576, 10328),
        kind: RuneSpotKind::River,
    },
    // runeposition "bot": (1180, -1216) [ENT:216753].
    RuneSpot {
        at: Vec2::from_ints(10396, 8000),
        kind: RuneSpotKind::River,
    },
    // `dota_item_rune_spawner_bounty`, runeteam 3 (the Dire primary jungle):
    // (-996.48, 4431.43) [ENT:215856].
    RuneSpot {
        at: Vec2::from_ints(8220, 13647),
        kind: RuneSpotKind::Jungle(Team::Dire),
    },
    // runeteam 2 (the Radiant primary jungle): (595.0, -4659.99) [ENT:217418].
    RuneSpot {
        at: Vec2::from_ints(9811, 4556),
        kind: RuneSpotKind::Jungle(Team::Radiant),
    },
];

/// A schedule that fires once every `period` ticks of the game clock from
/// `first` ticks past the horn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cadence {
    /// Ticks from the horn to the first firing.
    pub first: u32,
    /// Ticks between two firings; not zero.
    pub period: u32,
}

impl Cadence {
    /// Which firing falls on `clock` of a match with schedule `schedule`,
    /// counted from zero, if one does.
    pub const fn firing_at(self, clock: u32, schedule: Schedule) -> Option<u32> {
        let first = schedule.horn + self.first;
        if clock < first || !(clock - first).is_multiple_of(self.period) {
            return None;
        }
        Some((clock - first) / self.period)
    }

    /// The next firing strictly after `clock`: its zero-based index and
    /// game-clock tick, if both are representable.
    const fn next_after(self, clock: u32, schedule: Schedule) -> Option<(u32, u32)> {
        assert!(self.period > 0, "a cadence needs a positive period");
        let Some(first) = schedule.horn.checked_add(self.first) else {
            return None;
        };
        let index = if clock < first {
            0
        } else {
            let Some(index) = ((clock - first) / self.period).checked_add(1) else {
                return None;
            };
            index
        };
        let Some(offset) = index.checked_mul(self.period) else {
            return None;
        };
        let Some(tick) = first.checked_add(offset) else {
            return None;
        };
        Some((index, tick))
    }
}

/// The first set of Bounty Runes appears with the horn, at all four spots, the
/// river's pair and the jungles': owner-confirmed current Dota ("Runes appear
/// at all 4 spots at 0:00", 2026-10-30). Liquipedia's *Runes* names only the
/// river pair [LIQ Runes: "The first pair ... spawn at the river Power Rune
/// spots"]; the owner's word on the current game settles it. Ticks past the
/// horn.
pub const OPENING_BOUNTY_AFTER_HORN: u32 = 0;

/// Bounty Runes in the jungles: `dota_bounty_rune_spawn_time` 240 [BIN],
/// 4:00 since 7.38 [PN 7.38].
pub const BOUNTY_RUNES: Cadence = Cadence {
    first: 4 * 60 * TICKS_PER_SECOND,
    period: 240 * TICKS_PER_SECOND,
};

/// Water Runes: `dota_water_rune_spawn_time_initial` 120 [BIN]; they appear
/// only at 2:00 and 4:00, at both river spots [LIQ Runes: "Water Runes spawn
/// only at the following in-game intervals, they are no longer created
/// after"].
pub const WATER_RUNES: Cadence = Cadence {
    first: 2 * 60 * TICKS_PER_SECOND,
    period: 120 * TICKS_PER_SECOND,
};
/// How many times the Water Runes appear.
pub const WATER_RUNE_SPAWNS: u32 = 2;

/// The next scheduled expiry of a rune from this spawner, as an absolute
/// game-clock tick. River spawners replace their opening bounty, water and
/// power runes; jungle spawners retain untaken runes. `opening` is whether the
/// mode plays its first events ([`MapDef::opening_runes`](crate::MapDef)).
pub const fn rune_expires_at(
    spot: RuneSpotKind,
    clock: u32,
    schedule: Schedule,
    opening: bool,
) -> Option<u32> {
    if !matches!(spot, RuneSpotKind::River) {
        return None;
    }
    let water = match WATER_RUNES.next_after(clock, schedule) {
        // The mode that skips the first event never plays the second's predecessor: the
        // next water event is the one after it.
        Some((0, tick)) if !opening => tick.checked_add(WATER_RUNES.period),
        Some((index, tick)) if index < WATER_RUNE_SPAWNS => Some(tick),
        _ => None,
    };
    let power = match POWER_RUNES_CADENCE.next_after(clock, schedule) {
        Some((_, tick)) => Some(tick),
        None => None,
    };
    match (water, power) {
        (Some(water), Some(power)) => Some(if water < power { water } else { power }),
        (water, None) => water,
        (None, power) => power,
    }
}

/// Power Runes: `dota_powerup_rune_spawn_time_initial` 360 and
/// `dota_powerup_rune_spawn_time` 120 [BIN]; [LIQ Runes] "1st Spawn Interval:
/// 360, Subsequent Spawn Interval: 120".
pub const POWER_RUNES_CADENCE: Cadence = Cadence {
    first: 6 * 60 * TICKS_PER_SECOND,
    period: 120 * TICKS_PER_SECOND,
};

/// What appears on one tick of the clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunesDue {
    /// The first set of Bounty Runes, at every spot.
    pub opening_bounty: bool,
    /// A Bounty Rune at each jungle spot.
    pub jungle_bounty: bool,
    /// A Water Rune at each river spot.
    pub water: bool,
    /// One Power Rune at a river spot drawn at random.
    pub power: bool,
}

impl RunesDue {
    /// Whether the river's runes change this tick: its opening bounties,
    /// water and power runes go at the next river spawn [LIQ Runes: "If the rune is not
    /// activated before the next spawn time, they disappear"].
    pub const fn river_turns(self) -> bool {
        self.water || self.power
    }

    /// Whether anything appears.
    pub const fn any(self) -> bool {
        self.opening_bounty || self.jungle_bounty || self.water || self.power
    }
}

/// What the schedule puts out on `clock` of a match with schedule `schedule`. A mode that does
/// not play its opening (`opening` false: Dota's 1v1 Solo Mid, whose first event of each cycle
/// is skipped) has no horn set of Bounty Runes and no first Water event, the one at 2:00, so
/// its water comes at 4:00 alone [SRV: the 1v1 mode skips the first bounty event and the first
/// water event; the install's text says "First rune does not spawn"].
pub const fn runes_due(clock: u32, schedule: Schedule, opening: bool) -> RunesDue {
    RunesDue {
        opening_bounty: opening && clock == schedule.horn + OPENING_BOUNTY_AFTER_HORN,
        jungle_bounty: BOUNTY_RUNES.firing_at(clock, schedule).is_some(),
        water: match WATER_RUNES.firing_at(clock, schedule) {
            Some(n) => n < WATER_RUNE_SPAWNS && (opening || n > 0),
            None => false,
        },
        power: POWER_RUNES_CADENCE.firing_at(clock, schedule).is_some(),
    }
}

/// How near a hero must stand to take a rune it was ordered to take: 150 units,
/// measured on the ground between origins [SRV: a hero takes the rune on
/// arriving within 150 units of ground distance, with no facing test;
/// `Activate Distance` 150 in LIQ Runes]. The pickup is instant. Nothing
/// takes a rune but that order: the server has no pickup by proximity [SRV].
pub const RUNE_PICKUP_RANGE: i32 = 150;

/// How near another rune a new one may stand: it is put at its spawner's origin
/// unless a rune lies nearer than this to the origin [SRV: a spot with a
/// rune closer than 70 units, whatever the kinds, is rejected].
pub const RUNE_SPACING: i32 = 70;
/// How much farther from the spawner's origin each further ring of spots lies
/// [SRV: 90 units].
pub const RUNE_RING_STEP: i32 = 90;
/// The turn from one spot of a ring to the next: 45 degrees [SRV].
pub const RUNE_RING_TURN: Angle = Angle { brads: 8192 };
/// How many spots a ring holds: eight turns of 45 degrees, and a ninth that the
/// server's single-precision arithmetic reaches just short of the full turn
/// (it repeats the first) [SRV].
pub const RUNE_RING_SPOTS: u32 = 9;
/// How many spots the server tries before it gives up and takes the last
/// [SRV: 100].
pub const RUNE_PLACEMENT_ATTEMPTS: u32 = 100;

/// The `attempt`th spot a new rune is tried at around a spawner's `origin`
/// (counted from zero), the rings turned by `phase`: the origin itself, then
/// the rings of [`RUNE_RING_SPOTS`] spots at [`RUNE_RING_STEP`], twice that,
/// and so on, each turning counterclockwise from `phase` [SRV: the phase is
/// drawn at random for each placement; the first spot is the origin].
pub fn rune_spot(origin: Vec2, phase: Angle, attempt: u32) -> Vec2 {
    if attempt == 0 {
        return origin;
    }
    let past = attempt - 1;
    let ring = past / RUNE_RING_SPOTS + 1;
    let turns = past % RUNE_RING_SPOTS;
    // Eight turns make the full circle, which a u16 wraps to zero: widen first.
    let turned = (u32::from(phase.brads) + turns * u32::from(RUNE_RING_TURN.brads)) & 0xffff;
    crate::point_ahead(
        origin,
        Angle {
            brads: turned as u16,
        },
        Fixed::from_int((ring as i32) * RUNE_RING_STEP),
    )
}

/// Gold a Bounty Rune pays every hero of the side that takes it, fixed when
/// the rune appears [PN 7.40]: `40 + 6` for every whole four minutes since
/// the horn [PN 7.38: "Gold provided ... rescaled from 36 + (9 per 5
/// minutes) to 40 + (6 per 4 minutes)"]. Liquipedia's `40 + 6 * floor(t / 5)`
/// is the formula before 7.38.
pub const fn bounty_rune_gold(created: u32, schedule: Schedule) -> i32 {
    let since = created.saturating_sub(schedule.horn);
    let intervals = since / (240 * TICKS_PER_SECOND);
    40 + 6 * intervals as i32
}

/// Health a Water Rune mends at once [LOC DOTA_Tooltip_rune_water_description].
pub const WATER_RUNE_HP: i32 = 40;
/// Mana a Water Rune mends at once.
pub const WATER_RUNE_MANA: i32 = 80;

/// How long a Haste Rune holds: always 22 s [PN 7.40].
pub const HASTE_RUNE_TICKS: u32 = 22 * TICKS_PER_SECOND;
/// The speed a Haste Rune sets: "Haste Speed: 550", which nothing slows
/// the unit below [LIQ Runes].
pub const HASTE_RUNE_SPEED: i32 = 550;
/// How long an Amplify Damage Rune holds [LIQ Runes].
pub const AMPLIFY_RUNE_TICKS: u32 = 45 * TICKS_PER_SECOND;
/// Percent of the main attack damage it adds [LOC
/// DOTA_Tooltip_rune_doubledamage_description].
pub const AMPLIFY_DAMAGE_PCT: i32 = 80;
/// Spell amplification it adds, in basis points.
pub const AMPLIFY_SPELL_AMP_BP: i32 = 1_500;
/// How long an Arcane Rune holds [LIQ Runes; the modifier's tooltip there
/// says 30 % for cooldowns, the install's says 25 %, which is taken].
pub const ARCANE_RUNE_TICKS: u32 = 50 * TICKS_PER_SECOND;
/// Basis points it takes off every cooldown set [LOC
/// DOTA_Tooltip_rune_arcane_description].
pub const ARCANE_COOLDOWN_CUT_BP: i32 = 2_500;
/// Basis points it takes off every mana cost paid.
pub const ARCANE_MANA_CUT_BP: i32 = 3_000;
/// How long a Regeneration Rune holds [LIQ Runes].
pub const REGENERATION_RUNE_TICKS: u32 = 30 * TICKS_PER_SECOND;
/// Percent of the maximum health and mana it mends a second.
pub const REGENERATION_PCT: i32 = 6;
/// The same while damped by a player's blow [PN 7.35].
pub const REGENERATION_DAMPED_PCT: i32 = 1;
/// How long a blow from a hero damps it [LIQ Runes: "Downtime: 3"].
pub const REGENERATION_DAMP_TICKS: u32 = 3 * TICKS_PER_SECOND;
/// How long an Invisibility Rune holds [LIQ Runes].
pub const INVISIBILITY_RUNE_TICKS: u32 = 45 * TICKS_PER_SECOND;
/// How long the unit fades before it hides: "Fade Time: 2" [LIQ Runes].
pub const INVISIBILITY_FADE_TICKS: u32 = 2 * TICKS_PER_SECOND;
/// How long a Shield Rune holds [LOC DOTA_Tooltip_rune_shield_description].
pub const SHIELD_RUNE_TICKS: u32 = 75 * TICKS_PER_SECOND;
/// Percent of the maximum health its barrier takes.
pub const SHIELD_BARRIER_PCT: i32 = 50;
/// How many illusions an Illusion Rune makes [LOC
/// DOTA_Tooltip_rune_illusion_description].
pub const ILLUSION_RUNE_COUNT: usize = 2;
/// How long they stand.
pub const ILLUSION_RUNE_TICKS: u32 = 75 * TICKS_PER_SECOND;
/// The damage they deal, in basis points of the hero's: 35 %.
pub const ILLUSION_DEALT_BP: i32 = 3_500;
/// The damage a melee illusion takes: 200 %.
pub const ILLUSION_MELEE_TAKEN_BP: i32 = 20_000;
/// The damage a ranged illusion takes: 300 %.
pub const ILLUSION_RANGED_TAKEN_BP: i32 = 30_000;

/// How long a rune that holds for a time holds, in ticks. Zero for a rune
/// that takes effect at once or lives as units (the illusions).
pub const fn rune_ticks(kind: RuneKind) -> u32 {
    match kind {
        RuneKind::Water | RuneKind::Bounty | RuneKind::Illusion => 0,
        RuneKind::Haste => HASTE_RUNE_TICKS,
        RuneKind::AmplifyDamage => AMPLIFY_RUNE_TICKS,
        RuneKind::Arcane => ARCANE_RUNE_TICKS,
        RuneKind::Regeneration => REGENERATION_RUNE_TICKS,
        RuneKind::Invisibility => INVISIBILITY_RUNE_TICKS,
        RuneKind::Shield => SHIELD_RUNE_TICKS,
    }
}

/// What the power runes have spawned in the cycle they are in. Never repeats
/// within a cycle, and the first rune of a cycle is never the last one of
/// the cycle before [PN 7.33; LIQ Runes].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerCycle {
    /// The kinds spawned in this cycle.
    pub spawned: RuneSet,
    /// The last kind spawned.
    pub last: Option<RuneKind>,
}

impl PowerCycle {
    /// Draws the next power rune among the `enabled` kinds, `draw(n)` giving a
    /// number below `n`. Answers nothing when no power rune is enabled.
    pub fn draw(&mut self, enabled: RuneSet, mut draw: impl FnMut(u32) -> u32) -> Option<RuneKind> {
        let power = enabled.and(POWER_ROTATION);
        if power.is_empty() {
            return None;
        }
        let mut open = power.without(self.spawned);
        if open.is_empty() {
            self.spawned = RuneSet::NONE;
            open = power;
        }
        if self.spawned.is_empty()
            && let Some(last) = self.last
            && open.without(RuneSet::of(last)).iter().next().is_some()
        {
            open = open.without(RuneSet::of(last));
        }
        let count = open.iter().count() as u32;
        let kind = open.iter().nth(draw(count) as usize)?;
        self.spawned = self.spawned.with(kind);
        self.last = Some(kind);
        Some(kind)
    }
}

// The Bottle (`item_bottle`) [I:2073-2118]: what it costs, holds and mends is its catalog row's.

/// How long a Bottle keeps a rune before it takes effect by itself:
/// `rune_expire_time` 90 [I:2073-2118] [LIQ Bottle/Changelogs: 7.30e].
pub const BOTTLE_RUNE_TICKS: u32 = 90 * TICKS_PER_SECOND;
/// Charges a stored Bounty or Water Rune refills once it takes effect, when
/// the Bottle was not full as it was stored; a full one is refilled whole
/// [LOC DOTA_Tooltip_ability_item_bottle_Note1]. A stored power rune refills
/// the Bottle whole [LOC DOTA_Tooltip_ability_item_bottle_Description:
/// "Using a stored rune fully refills the Bottle"].
pub const BOTTLE_LESSER_RUNE_CHARGES: u8 = 2;
/// Ticks between two refills at the fountain: Dota starts a one-second timer
/// at each [LIQ Bottle/Changelogs: 7.38, "the wielder must now be physically
/// within the fountain area"]. The area is the fountain's own, as far as its
/// Rejuvenation Aura reaches.
pub const BOTTLE_REFILL_PERIOD_TICKS: u32 = TICKS_PER_SECOND;

const _: () = {
    assert!(BOTTLE_LESSER_RUNE_CHARGES <= ITEMS[ItemId::BOTTLE.0 as usize].charges);
    assert!(INVISIBILITY_FADE_TICKS < INVISIBILITY_RUNE_TICKS);
    assert!(REGENERATION_DAMPED_PCT < REGENERATION_PCT);
    assert!(DOTA_RUNE_SPOTS.len() * MAX_RUNES_AT_SPOT <= MAX_SNAPSHOT_RUNES);
    assert!(SIMULATED_RUNES.bits == RuneSet::ALL.bits);
    assert!(rules::SOLO_MID_PREGAME_TICKS > 0 && rules::DOTA_PREGAME_TICKS > 0);
};
