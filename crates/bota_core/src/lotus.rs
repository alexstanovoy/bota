//! The Lotus Pools at Dota 7.41f: two invulnerable buildings that grow Healing
//! Lotuses for any hero to pluck by standing in them.
//!
//! Sources, tagged where each number is used: `[ENT:n]` the entity blocks of
//! the map's `maps/dota/entities/default_ents.vents` (7.41f), shifted by half
//! the map like every landmark; `[U]` and `[A]` the lines of the game's
//! `scripts/npc/npc_units.txt` and `npc_abilities.txt` (`npc_dota_lotus_pool`,
//! `ability_lotus_pool`, `ability_pluck_lotus`), `[I]` those of `items.txt`
//! (`item_famango`, `item_great_famango`, `item_greater_famango` and their
//! recipes); `[BIN]` the server's convar names (`dota_lotus_respawn_time`,
//! `dota_lotuses_max`); `[PN v]` Valve's patch notes of version `v`; `[LIQ
//! page]` Liquipedia's *Lotus Pool* and *Healing Lotus* (read 2026-10-30).

use bota_proto::{ItemId, LotusTier, Vec2};

use crate::rules::{self, Schedule, TICKS_PER_SECOND};
use crate::{BARE_UNIT, Cadence, UnitDef};
use bota_proto::UnitKind;

/// A pool's health [U]; it is invulnerable all the same [LIQ Lotus Pool].
pub const POOL_HP: i32 = 450;
/// How far from a pool's centre a hero must stand to pluck: `radius` 350
/// [A, PN 7.38].
pub const POOL_RADIUS: i32 = 350;
/// How many lotuses a pool holds at most [LIQ Lotus Pool: "Max Lotuses: 6",
/// BIN `dota_lotuses_max`].
pub const POOL_MAX_LOTUSES: u8 = 6;
/// Ticks between two lotuses a pool grows, on the game clock counted from the horn: 3 minutes
/// [PN 7.33; LIQ Lotus Pool: "Replenish Interval: 180", BIN
/// `dota_lotus_respawn_time`], at the exact minute mark [PN 7.35]. A pool
/// starts empty. A lotus that would grow while the pool is full does not.
pub const POOL_GROWTH: Cadence = Cadence {
    first: 3 * 60 * TICKS_PER_SECOND,
    period: 180 * TICKS_PER_SECOND,
};
/// The tick pools start growing Great Healing Lotuses: 35:00, "after Tier 4
/// neutral items are available" [PN 7.38; LIQ Lotus Pool]. The lotuses
/// lying in a pool then are rounded up and combined into greater ones.
pub const GREAT_AFTER_HORN: u32 = 35 * 60 * TICKS_PER_SECOND;
/// The tick pools start growing Greater Healing Lotuses: 60:00, with Tier 5
/// [PN 7.38; LIQ Lotus Pool].
pub const GREATER_AFTER_HORN: u32 = 60 * 60 * TICKS_PER_SECOND;

/// Ticks a hero plucks the first lotus of a row: `first_lotus_pickup_time` 1.5
/// [A].
pub const FIRST_PLUCK_TICKS: u32 = 45;
/// Ticks each lotus after the first of a row takes, by its place: each takes
/// 30% less than the one before (`pickup_time_reduction_pct` 70 [A, PN 7.38]),
/// down to `min_lotus_pickup_time` 0.3 s [A]: 1.5, 1.05, 0.74, 0.51, 0.36, 0.3
/// seconds [LIQ Lotus Pool].
pub const PLUCK_TICKS: [u32; 6] = [FIRST_PLUCK_TICKS, 32, 22, 15, 11, 9];

/// The Dota map's two Lotus Pools, shifted like every landmark.
pub const DOTA_LOTUS_POOLS: [Vec2; 2] = [
    // "npc_dota_lotus_pool_dire" (-7548.28, 4209.10) [ENT:194168].
    Vec2::from_ints(1668, 13425),
    // "npc_dota_lotus_pool_radiant" (7503.98, -4404.69) [ENT:194273].
    Vec2::from_ints(16720, 4811),
];

/// A Lotus Pool: an invulnerable building of the neutral team on the filler
/// hull that sees 500 round it by day and by night [U].
pub const LOTUS_POOL: UnitDef = UnitDef {
    kind: UnitKind::LotusPool,
    max_hp: POOL_HP,
    day_vision: 500,
    night_vision: 500,
    collision: rules::FILLER_COLLISION,
    bound: rules::FILLER_BOUND,
    invulnerable: true,
    ..BARE_UNIT
};

/// Ticks from `clock` to the next lotus a pool grows.
pub const fn next_growth_in(clock: u32, schedule: Schedule) -> u32 {
    let first = schedule.horn + POOL_GROWTH.first;
    if clock < first {
        first - clock
    } else {
        POOL_GROWTH.period - (clock - first) % POOL_GROWTH.period
    }
}

/// Which lotus a pool grows at a tick of the game clock.
pub const fn grown_at(clock: u32, schedule: Schedule) -> LotusTier {
    if clock >= schedule.horn + GREATER_AFTER_HORN {
        LotusTier::Greater
    } else if clock >= schedule.horn + GREAT_AFTER_HORN {
        LotusTier::Great
    } else {
        LotusTier::Healing
    }
}

/// The item a lotus of a tier is.
pub const fn lotus_item(tier: LotusTier) -> ItemId {
    match tier {
        LotusTier::Healing => ItemId::HEALING_LOTUS,
        LotusTier::Great => ItemId::GREAT_HEALING_LOTUS,
        LotusTier::Greater => ItemId::GREATER_HEALING_LOTUS,
    }
}

/// Lotuses of a lower tier rounded up into the tier a pool grows from now
/// on: three healing lotuses make a great one and two great ones a greater
/// one, and one left over still makes one [PN 7.38; LIQ Lotus Pool: "All
/// existing lower-tier Lotuses get rounded up and combined into a higher
/// tier"].
pub const fn rounded_up(held: u8, from: LotusTier, to: LotusTier) -> u8 {
    let mut count = held;
    let mut tier = from as u8;
    while tier < to as u8 {
        let per = if tier == LotusTier::Healing as u8 {
            3
        } else {
            2
        };
        count = count.div_ceil(per);
        tier += 1;
    }
    count
}

/// Health and mana a lotus mends each at once on use: 125, 400 and 900 [I
/// `replenish_amount`].
pub const fn lotus_replenish(tier: LotusTier) -> i32 {
    match tier {
        LotusTier::Healing => 125,
        LotusTier::Great => 400,
        LotusTier::Greater => 900,
    }
}

/// How many lotuses of a tier combine into one of the next: three healing,
/// two great [I `item_recipe_*_famango`, PN 7.33].
pub const fn lotuses_to_combine(tier: LotusTier) -> u8 {
    match tier {
        LotusTier::Healing => 3,
        LotusTier::Great => 2,
        LotusTier::Greater => u8::MAX,
    }
}

/// How far a hero eats a lotus at an ally: `AbilityCastRange` 400 [I].
pub const LOTUS_RANGE: i32 = 400;
/// Ticks between two lotuses eaten, whatever their tier, by one hero:
/// `AbilityCooldown` 5, the shared cooldown `famango` [I].
pub const LOTUS_COOLDOWN: u32 = 5 * TICKS_PER_SECOND;
/// How near a hero must stand to take a lotus lying on the ground: 275
/// [LIQ Healing Lotus: "a total 275 pick-up range, compared to the 150 range
/// of all other items"].
pub const LOTUS_TAKE_RANGE: i32 = 275;

const _: () = {
    assert!(PLUCK_TICKS[PLUCK_TICKS.len() - 1] > 0);
    assert!(GREAT_AFTER_HORN < GREATER_AFTER_HORN);
    assert!(DOTA_LOTUS_POOLS.len() <= bota_proto::MAX_LOTUS_POOLS);
};
