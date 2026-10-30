//! The records of a metadata file, and what makes them durable.
//!
//! A file is frames, `u32` little endian length then a postcard [`MetaRecord`], the first the
//! [`MetaHeader`]. The layouts here are listed in `registry/meta.txt`, which a test holds to
//! append-only: a released record, field or variant never changes or moves, and a fact the
//! records cannot tell yet is a new variant or a new [`stat`] id. A reader skips a frame it does
//! not know by its length, and ignores a stat id it does not know. The records carry primitives
//! only, never a wire type, whose layout moves with the wire.

use serde::{Deserialize, Serialize};

/// The first record of every file, frozen: no field is ever added to it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetaHeader {
    /// [`MetaHeader::MAGIC`].
    pub magic: [u8; 8],
    /// The version of this format the file was written in; raised only if a released layout had
    /// to break, which the append-only rule is there to avoid.
    pub schema: u16,
    /// The wire fingerprint of the build that wrote it: where it came from, never a gate.
    pub wire_fingerprint: u64,
    /// The version of the rules the match was played by.
    pub rules_version: u32,
    pub match_id: u64,
    pub map: u16,
    /// Ticks per second.
    pub tick_rate: u16,
    /// Ticks before the game clock reaches zero.
    pub pregame_ticks: u32,
    /// How many item ids the match played on, from the first, and the digest of those items:
    /// the catalogue its seats shared, which prices what the file's values were worked out with.
    pub catalogue_items: u16,
    pub catalogue_digest: u64,
}

impl MetaHeader {
    pub const MAGIC: [u8; 8] = *b"bota.met";
    /// The version this build writes.
    pub const SCHEMA: u16 = 1;
}

/// Who did a thing in the match.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// A seat's hero, by slot.
    Hero(u8),
    /// Anything else: a creep, a building.
    Other,
}

/// A fact of the match.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum MetaRecord {
    Header(MetaHeader),
    /// A seat, once at the start.
    Seat {
        slot: u8,
        /// 0 Radiant, 1 Dire.
        team: u8,
        /// The hero's id.
        hero: u16,
    },
    /// What a seat held and had earned at a tick: one for every seat at the start of each
    /// whole game minute, and at the end. `(stat id, value)`, see [`stat`].
    Sample {
        tick: u32,
        slot: u8,
        stats: Vec<(u16, i64)>,
    },
    /// A hero fell: a seat's, or one the match created for no seat.
    Kill {
        tick: u32,
        victim: Who,
        killer: Option<Who>,
        /// The gold the kill paid its killer's seat.
        gold: i32,
    },
    /// A seat bought an item.
    Purchase {
        tick: u32,
        slot: u8,
        item: u16,
    },
    /// A hero gained a level.
    LevelUp {
        tick: u32,
        slot: u8,
        level: u8,
    },
    /// A building fell.
    Structure {
        tick: u32,
        /// The wire's unit kind, by its index (`Tower`, `Barracks`, `Ancient`, …).
        kind: u8,
        /// The side that lost it: 0 Radiant, 1 Dire.
        team: u8,
    },
    /// What a seat's hero carried at the end: the inventory then the backpack, an item's id.
    Inventory {
        tick: u32,
        slot: u8,
        items: Vec<Option<u16>>,
    },
    /// The match was decided.
    End {
        tick: u32,
        /// 0 Radiant, 1 Dire; none for a draw.
        winner: Option<u8>,
    },
}

/// The ids of the values of a [`MetaRecord::Sample`]. Append-only: a retired id stays reserved.
pub mod stat {
    /// Gold in hand.
    pub const GOLD: u16 = 0;
    /// Gold earned over the match, spent or not; only in the sample at the end.
    pub const GOLD_EARNED: u16 = 1;
    /// Gold in hand and the value of everything carried and stashed.
    pub const NET_WORTH: u16 = 2;
    /// Experience earned.
    pub const XP: u16 = 3;
    pub const LEVEL: u16 = 4;
    pub const KILLS: u16 = 5;
    pub const DEATHS: u16 = 6;
    pub const ASSISTS: u16 = 7;
    pub const LAST_HITS: u16 = 8;
    pub const DENIES: u16 = 9;
    /// Damage dealt to enemy heroes.
    pub const HERO_DAMAGE: u16 = 10;
    /// Damage dealt to enemy buildings.
    pub const STRUCTURE_DAMAGE: u16 = 11;
    /// Health restored to heroes of the seat's side.
    pub const HEALING: u16 = 12;
}
