//! A place at the match, and what belongs to a player rather than to a body.

use bota_proto::{HeroId, ItemId, SlotId, Team, Vec2};

use crate::engine::state_hash;
use crate::game::{AbilityBook, Entity, Inventory, Modifiers};

/// What a fallen hero leaves behind until it stands up again.
#[derive(Clone, Debug)]
pub struct Kept {
    /// What it had learned.
    pub book: AbilityBook,
    /// What it was carrying.
    pub bag: Inventory,
    /// What it wore that lasts through death: what it had gathered.
    pub modifiers: Modifiers,
    /// The tick it fell on: what was bought since may be sold back while it
    /// is gone, as in Dota.
    pub fell: u32,
}

/// A Town Portal Scroll being read by a seat's hero, and the sight it
/// leaves where it lands for a moment after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Teleport {
    /// The allied building it carries to.
    pub building: Entity,
    /// Where the hero lands.
    pub landing: Vec2,
    /// The tick the channel began.
    pub started: u32,
    /// The tick it carries, unless broken off first.
    pub lands: u32,
    /// Whether the channel is still under way. Once it is not, all that is
    /// left is the sight at `landing` until `sight_until`.
    pub live: bool,
    /// The last tick its side sees around `landing`: while it is live,
    /// [`bota_core::rules::SCROLL_SIGHT_LINGER_TICKS`] past `lands`; once
    /// it ended, that long past the end.
    pub sight_until: u32,
    /// Radius of that sight: the scroll's.
    pub sight_radius: i32,
}

/// What a side's scoreboard shows of a seat's hero: as it stood the last
/// time that side saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sighting {
    pub level: u8,
    pub xp: i32,
    /// The tick it was seen on; none before it ever was.
    pub at: Option<u32>,
}

impl Sighting {
    /// A hero as every match starts it, not yet seen.
    pub const START: Sighting = Sighting {
        level: 1,
        xp: 0,
        at: None,
    };
}

/// One player's place at the match. Everything here outlives the body.
#[derive(Clone, Debug)]
pub struct Seat {
    /// Which place this is.
    pub slot: SlotId,
    /// Which side it plays.
    pub team: Team,
    /// Which hero it picked.
    pub hero: HeroId,
    /// The body it drives, while one stands.
    pub unit: Option<Entity>,
    /// Gold in hand.
    pub gold: i32,
    /// Gold earned over the match, spent or not, less what deaths cost.
    pub net_worth: i32,
    /// Experience gathered.
    pub xp: i32,
    /// Level reached.
    pub level: u8,
    /// Ticks before the body comes back.
    pub respawn_left: u32,
    /// What waits in the stash at the fountain.
    pub stash: Inventory,
    /// What the body left behind, while it is gone.
    pub kept: Option<Kept>,
    /// The courier it owns, while one stands.
    pub courier: Option<Entity>,
    /// Ticks before the courier comes back. Zero while one stands.
    pub courier_left: u32,
    /// What the courier carried when it fell, until the next one stands.
    pub courier_kept: Option<Inventory>,
    /// Ticks owed on a kind of item, whichever stack of it is held next.
    pub item_clocks: Vec<(ItemId, u32)>,
    /// Enemy heroes brought down.
    pub kills: u16,
    /// Enemy heroes brought down since its own body last fell. Prices its
    /// head.
    pub streak: u16,
    /// Times its own body was brought down.
    pub deaths: u16,
    /// Kills it helped with.
    pub assists: u16,
    /// Enemy units it landed the last blow on.
    pub last_hits: u16,
    /// Its own units it brought down.
    pub denies: u16,
    /// Damage its hero has dealt to enemy heroes, after armor and resistance.
    pub hero_damage: i32,
    /// Damage its hero has dealt to enemy structures, after armor and
    /// resistance.
    pub structure_damage: i32,
    /// The scroll its hero reads, or read a moment ago.
    pub teleport: Option<Teleport>,
    /// Its hero as each side last saw it, Radiant's then Dire's: the level
    /// and experience the other side's scoreboard shows.
    pub sighted: [Sighting; 2],
    /// A seat a cheat created for its hero alone: driven by no player, and
    /// no death of it counts against its side.
    pub created: bool,
    /// Its hero takes no damage (a cheat), dead and back again.
    pub invulnerable: bool,
}

impl Seat {
    /// A place at the start of a match.
    pub fn new(slot: SlotId, team: Team, hero: HeroId, gold: i32, stash_slots: usize) -> Seat {
        Seat {
            slot,
            team,
            hero,
            unit: None,
            gold,
            net_worth: gold,
            xp: 0,
            level: 1,
            respawn_left: 0,
            stash: Inventory::stash(stash_slots),
            kept: None,
            courier: None,
            courier_left: 0,
            courier_kept: None,
            item_clocks: Vec::new(),
            kills: 0,
            streak: 0,
            deaths: 0,
            assists: 0,
            last_hits: 0,
            denies: 0,
            hero_damage: 0,
            structure_damage: 0,
            teleport: None,
            sighted: [Sighting::START; 2],
            created: false,
            invulnerable: false,
        }
    }
}

state_hash!(Kept {
    book,
    bag,
    modifiers,
    fell
});
state_hash!(Teleport {
    building,
    landing,
    started,
    lands,
    live,
    sight_until,
    sight_radius
});
state_hash!(Sighting { level, xp, at });
state_hash!(Seat {
    slot,
    team,
    hero,
    unit,
    gold,
    net_worth,
    xp,
    level,
    respawn_left,
    stash,
    kept,
    courier,
    courier_left,
    courier_kept,
    item_clocks,
    kills,
    streak,
    deaths,
    assists,
    last_hits,
    denies,
    hero_damage,
    structure_damage,
    teleport,
    sighted,
    created,
    invulnerable
});
