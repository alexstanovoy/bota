//! Ticks built by hand, with every field named.
//!
//! Listing all of them is the guard on the shape of what the model is shown: a
//! new field in `UnitView` breaks this and asks what should be done about it,
//! rather than quietly arriving as a nought.

use bota_proto::{
    AbilityId, AbilityView, Aim, Angle, Attributes, EntityId, Fixed, HeroId, ItemId, ItemView,
    PlayerView, SlotId, StatusFlags, Team, UnitKind, UnitView, Vec2, WorldView,
};

/// Where each fountain stands on the map both sides play.
pub const RADIANT_HOME: (i32, i32) = (1760, 2278);
/// The other one.
pub const DIRE_HOME: (i32, i32) = (16624, 16064);

/// A handle by its number alone.
pub fn id(idx: u32) -> EntityId {
    EntityId { idx, generation: 1 }
}

/// One unit standing somewhere, with everything named.
pub fn unit(idx: u32, kind: UnitKind, team: Team, at: (i32, i32), hp: i32) -> UnitView {
    UnitView {
        id: id(idx),
        kind,
        team,
        pos: Vec2::from_ints(at.0, at.1),
        facing: Angle::default(),
        hp,
        max_hp: 600,
        mana: 300,
        max_mana: 300,
        move_speed: Fixed::from_int(300),
        attack_damage: 50,
        attack_range: Fixed::from_int(600),
        attack_interval: 51,
        attack_speed: 100,
        attributes: Attributes::ZERO,
        primary: None,
        armor: Fixed::from_int(2),
        magic_resist: Fixed::from_ratio(25, 100),
        radius: Fixed::from_int(24),
        vision_radius: Fixed::from_int(1800),
        true_sight_radius: Fixed::ZERO,
        statuses: StatusFlags::default(),
        hero: (kind == UnitKind::Hero).then_some(HeroId(0)),
        owner: (kind == UnitKind::Hero).then_some(SlotId(0)),
        level: 1,
        abilities: Vec::new(),
        items: vec![None; 9],
        effects: Vec::new(),
    }
}

/// A hero with four abilities learned and something to carry.
pub fn hero(idx: u32, team: Team, at: (i32, i32), slot: SlotId) -> UnitView {
    let mut body = unit(idx, UnitKind::Hero, team, at, 600);
    body.owner = Some(slot);
    body.level = 4;
    body.abilities = (0..4)
        .map(|which| AbilityView {
            id: AbilityId(which),
            level: 1,
            max_level: 4,
            range: 0,
            aim: Aim::Own,
            passive: false,
            on: false,
            can_level: false,
            cooldown_left: 0,
            mana_cost: 50,
        })
        .collect();
    body.items[0] = Some(ItemView {
        id: ItemId(7),
        charges: Some(3),
        cooldown_left: 0,
        mana_cost: 0,
        range: 0,
        aim: None,
        mode: None,
        for_sale: false,
    });
    body
}

/// This seat's courier, with the errands it carries.
pub fn courier(idx: u32, team: Team, at: (i32, i32), slot: SlotId) -> UnitView {
    let mut bird = unit(idx, UnitKind::Courier, team, at, 250);
    bird.owner = Some(slot);
    bird.hero = None;
    bird.items = vec![None; 6];
    bird.abilities = [8u16, 10, 11, 9]
        .into_iter()
        .map(|which| AbilityView {
            id: AbilityId(which),
            level: 1,
            max_level: 4,
            range: 0,
            aim: Aim::Own,
            passive: false,
            on: false,
            can_level: false,
            cooldown_left: 0,
            mana_cost: 0,
        })
        .collect();
    bird
}

/// A building of a side, which both sides always see.
pub fn building(idx: u32, kind: UnitKind, team: Team, at: (i32, i32)) -> UnitView {
    let mut built = unit(idx, kind, team, at, 2000);
    built.max_hp = 2000;
    built.attack_damage = if kind == UnitKind::Tower { 110 } else { 0 };
    built.attack_range = Fixed::from_int(if kind == UnitKind::Tower { 700 } else { 0 });
    built.radius = Fixed::from_int(144);
    built.move_speed = Fixed::ZERO;
    built
}

/// A seat holding one body.
pub fn player(slot: SlotId, team: Team, unit: Option<EntityId>, gold: i32) -> PlayerView {
    PlayerView {
        slot,
        team,
        hero: HeroId(0),
        unit,
        level: 4,
        xp: 0,
        gold: Some(gold),
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

/// A tick with a hero, a courier, both fountains, a tower each and the creeps
/// given.
pub fn a_tick(units: Vec<UnitView>, gold: i32) -> WorldView {
    a_tick_at((7000, 7200), units, gold)
}

/// A tick where this seat owns the items named and nothing else.
///
/// The bag first and the stash for whatever will not fit, since a hero has nine
/// slots and a build order is longer than that before its parts come together.
pub fn a_tick_holding(items: &[u16], gold: i32) -> WorldView {
    let held = |item: &u16| {
        Some(ItemView {
            id: ItemId(*item),
            charges: Some(1),
            cooldown_left: 0,
            mana_cost: 0,
            range: 0,
            aim: None,
            mode: None,
            for_sale: false,
        })
    };
    let (bag, spilled) = items.split_at(items.len().min(crate::BAG_SLOTS));
    let mut view = a_tick(Vec::new(), gold);
    for body in &mut view.units {
        if body.kind != UnitKind::Hero || body.owner != Some(SlotId(0)) {
            continue;
        }
        for slot in &mut body.items {
            *slot = None;
        }
        for (at, item) in bag.iter().enumerate() {
            body.items[at] = held(item);
        }
    }
    for player in &mut view.players {
        if player.slot == SlotId(0) {
            let mut stash = vec![None; 6];
            for (at, item) in spilled.iter().enumerate() {
                stash[at] = held(item);
            }
            player.stash = Some(stash);
        }
    }
    view
}

/// The same, with the goods placed by hand: some worn in the working slots,
/// some riding in the backpack, some waiting in the stash.
pub fn a_tick_wearing(worn: &[u16], backpack: &[u16], stash: &[u16], gold: i32) -> WorldView {
    let held = |item: &u16| {
        Some(ItemView {
            id: ItemId(*item),
            charges: Some(1),
            cooldown_left: 0,
            mana_cost: 0,
            range: 0,
            aim: None,
            mode: None,
            for_sale: false,
        })
    };
    let mut view = a_tick(Vec::new(), gold);
    for body in &mut view.units {
        if body.kind != UnitKind::Hero || body.owner != Some(SlotId(0)) {
            continue;
        }
        for slot in &mut body.items {
            *slot = None;
        }
        for (at, item) in worn.iter().enumerate().take(crate::WORN_SLOTS) {
            body.items[at] = held(item);
        }
        for (at, item) in backpack.iter().enumerate() {
            body.items[crate::WORN_SLOTS + at] = held(item);
        }
    }
    for player in &mut view.players {
        if player.slot == SlotId(0) {
            let mut waiting = vec![None; 6];
            for (at, item) in stash.iter().enumerate() {
                waiting[at] = held(item);
            }
            player.stash = Some(waiting);
        }
    }
    view
}

/// The same, with the hero standing where it is told to.
pub fn a_tick_at(me: (i32, i32), mut units: Vec<UnitView>, gold: i32) -> WorldView {
    let me = hero(0, Team::Radiant, me, SlotId(0));
    let mine = me.id;
    units.push(me);
    units.push(courier(1, Team::Radiant, RADIANT_HOME, SlotId(0)));
    units.push(building(
        90,
        UnitKind::Fountain,
        Team::Radiant,
        RADIANT_HOME,
    ));
    units.push(building(91, UnitKind::Fountain, Team::Dire, DIRE_HOME));
    units.push(building(92, UnitKind::Tower, Team::Radiant, (6000, 6200)));
    units.push(building(93, UnitKind::Tower, Team::Dire, (9740, 9868)));
    WorldView {
        tick: 3000,
        viewer: Some(Team::Radiant),
        units,
        projectiles: Vec::new(),
        players: vec![
            player(SlotId(0), Team::Radiant, Some(mine), gold),
            player(SlotId(1), Team::Dire, Some(id(50)), 0),
        ],
        felled_trees: Vec::new(),
        planted_trees: Vec::new(),
        loot: Vec::new(),
    }
}

/// A tick with a wave of each side and the other hero in sight.
pub fn a_busy_tick() -> WorldView {
    let mut units = Vec::new();
    for at in 0..5 {
        units.push(unit(
            10 + at,
            UnitKind::CreepMelee,
            Team::Dire,
            (7300 + 60 * at as i32, 7300),
            120,
        ));
    }
    for at in 0..3 {
        units.push(unit(
            20 + at,
            UnitKind::CreepMelee,
            Team::Radiant,
            (6800 - 60 * at as i32, 7100),
            200,
        ));
    }
    let mut theirs = hero(50, Team::Dire, (7600, 7600), SlotId(1));
    theirs.owner = Some(SlotId(1));
    units.push(theirs);
    a_tick(units, 600)
}
