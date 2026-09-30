//! A place at the match, and what belongs to a player rather than to a body.

use bota_proto::{HeroId, SlotId, Team};

use bota_proto::ItemId;

use crate::game::{AbilityBook, Entity, Inventory, Stacks};

/// What a fallen hero leaves behind until it stands up again.
#[derive(Clone, Debug)]
pub struct Kept {
    /// What it had learned.
    pub book: AbilityBook,
    /// What it was carrying.
    pub bag: Inventory,
    /// What it had gathered.
    pub stacks: Stacks,
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
            stash: Inventory::empty(stash_slots),
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
        }
    }
}
