//! Views built by hand, for tests that need a tick to read.
//!
//! Every field is listed. A field added to a view breaks these, which is the
//! point: what the bot should make of it is a decision, not a default.

use bota_proto::{
    AbilityId, AbilityView, Aim, Angle, Attribute, Attributes, EffectId, EffectView, EntityId,
    Fixed, HeroId, ItemId, ItemView, MapId, MatchInfo, Pick, PlayerView, ShopEntry, SlotId,
    StatusFlags, Team, TickMode, UnitKind, UnitView, Vec2, WorldView,
};

/// A handle with a given index, at the first generation.
pub fn id(idx: u32) -> EntityId {
    EntityId { idx, generation: 1 }
}

/// A hero standing somewhere, with nothing on it and nothing in its bag.
pub fn hero(idx: u32, team: Team, pos: Vec2) -> UnitView {
    UnitView {
        id: id(idx),
        kind: UnitKind::Hero,
        team,
        pos,
        facing: Angle { brads: 0 },
        hp: 600,
        max_hp: 600,
        mana: 300,
        max_mana: 300,
        move_speed: Fixed::from_int(300),
        attack_damage: 50,
        attack_range: Fixed::from_int(500),
        attack_interval: 51,
        attack_speed: 100,
        armor: Fixed::from_int(2),
        magic_resist: Fixed::from_ratio(1, 4),
        radius: Fixed::from_int(24),
        vision_radius: Fixed::from_int(1800),
        true_sight_radius: Fixed::ZERO,
        statuses: StatusFlags { bits: 0 },
        attributes: Attributes::all(20),
        primary: Some(Attribute::Agility),
        hero: Some(HeroId(2)),
        owner: Some(SlotId(0)),
        level: 1,
        abilities: Vec::new(),
        items: vec![None; 9],
        effects: Vec::new(),
    }
}

/// A melee lane creep standing somewhere.
pub fn creep(idx: u32, team: Team, pos: Vec2, hp: i32) -> UnitView {
    UnitView {
        id: id(idx),
        kind: UnitKind::CreepMelee,
        team,
        pos,
        facing: Angle { brads: 0 },
        hp,
        max_hp: 550,
        mana: 0,
        max_mana: 0,
        move_speed: Fixed::from_int(325),
        attack_damage: 21,
        attack_range: Fixed::from_int(100),
        attack_interval: 30,
        attack_speed: 100,
        armor: Fixed::from_int(2),
        magic_resist: Fixed::ZERO,
        radius: Fixed::from_int(16),
        vision_radius: Fixed::from_int(750),
        true_sight_radius: Fixed::ZERO,
        statuses: StatusFlags { bits: 0 },
        attributes: Attributes::ZERO,
        primary: None,
        hero: None,
        owner: None,
        level: 0,
        abilities: Vec::new(),
        items: Vec::new(),
        effects: Vec::new(),
    }
}

/// A building of one kind, standing somewhere.
pub fn works(idx: u32, kind: UnitKind, team: Team, pos: Vec2) -> UnitView {
    UnitView {
        id: id(idx),
        kind,
        team,
        pos,
        facing: Angle { brads: 0 },
        hp: 1800,
        max_hp: 1800,
        mana: 0,
        max_mana: 0,
        move_speed: Fixed::ZERO,
        attack_damage: 110,
        attack_range: Fixed::from_int(700),
        attack_interval: 29,
        attack_speed: 100,
        armor: Fixed::from_int(12),
        magic_resist: Fixed::ZERO,
        radius: Fixed::from_int(40),
        vision_radius: Fixed::from_int(1900),
        true_sight_radius: Fixed::from_int(700),
        statuses: StatusFlags { bits: 0 },
        attributes: Attributes::ZERO,
        primary: None,
        hero: None,
        owner: None,
        level: 0,
        abilities: Vec::new(),
        items: Vec::new(),
        effects: Vec::new(),
    }
}

/// One ability slot at a level, ready to cast.
pub fn ability(id: AbilityId, level: u8, range: i32) -> AbilityView {
    AbilityView {
        id,
        level,
        max_level: 4,
        cooldown_left: 0,
        mana_cost: 75,
        range,
        aim: Aim::Own,
        passive: false,
        on: false,
        can_level: false,
    }
}

/// One item slot, with charges left and nothing waiting.
pub fn item(id: ItemId) -> ItemView {
    ItemView {
        id,
        charges: Some(1),
        cooldown_left: 0,
        mute_left: 0,
        mode: None,
        mana_cost: 0,
        range: 0,
        aim: Some(Aim::Unit),
        for_sale: false,
    }
}

/// A gathering of souls showing on a hero.
pub fn souls(many: u32) -> EffectView {
    EffectView {
        id: EffectId(11),
        ticks_left: None,
        stacks: Some(many),
    }
}

/// The scoreboard row of one seat, with a hero standing.
pub fn seat(slot: SlotId, team: Team, hero: HeroId, unit: Option<EntityId>) -> PlayerView {
    PlayerView {
        slot,
        team,
        hero,
        unit,
        level: 1,
        xp: 0,
        gold: Some(600),
        stash: Some(vec![None; 6]),
        kit: None,
        kills: 0,
        deaths: 0,
        assists: 0,
        last_hits: 0,
        denies: 0,
        respawn_left: 0,
    }
}

/// A snapshot of a tick holding the units and seats given.
pub fn tick(at: u32, units: Vec<UnitView>, players: Vec<PlayerView>) -> WorldView {
    WorldView {
        tick: at,
        viewer: Some(Team::Radiant),
        units,
        projectiles: Vec::new(),
        players,
        felled_trees: Vec::new(),
        planted_trees: Vec::new(),
        loot: Vec::new(),
    }
}

/// The terms of a match selling the shop rows given.
pub fn started(shop: Vec<ShopEntry>, trees: Vec<Vec2>) -> MatchInfo {
    MatchInfo {
        match_id: 1,
        map: MapId(0),
        tick_rate: 30,
        pregame_ticks: 900,
        trees,
        terrain_cells: 0,
        terrain_rle: Vec::new(),
        opaque_cells: Vec::new(),
        mode: TickMode::Lockstep,
        picks: vec![Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero: HeroId(2),
        }],
        shop,
    }
}

/// One row of the shop.
pub fn sold(id: ItemId, cost: i32, components: Vec<ItemId>) -> ShopEntry {
    ShopEntry {
        id,
        cost,
        components,
    }
}

/// The two fountains of the Dota map, which every lane is laid out from.
pub fn fountains() -> Vec<UnitView> {
    vec![
        works(
            900,
            UnitKind::Fountain,
            Team::Radiant,
            Vec2::from_ints(1760, 2278),
        ),
        works(
            901,
            UnitKind::Fountain,
            Team::Dire,
            Vec2::from_ints(16624, 16064),
        ),
    ]
}
