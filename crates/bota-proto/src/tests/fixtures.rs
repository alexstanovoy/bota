//! Sample values for the tests.
//!
//! Every struct here is built by listing all of its fields. Adding a field to a
//! wire type breaks compilation right here, which is the point: it forces a
//! decision about what the new field carries before it can ship.

use crate::*;

pub fn entity(idx: u32) -> EntityId {
    EntityId { idx, generation: 1 }
}

pub fn fixed(units: i32) -> Fixed {
    Fixed { raw: units << 16 }
}

pub fn ability_view(slot: u16) -> AbilityView {
    AbilityView {
        id: AbilityId(slot),
        level: 2,
        max_level: 4,
        cooldown_left: 40,
        mana_cost: 90,
        range: 600,
        aim: Aim::Unit,
        passive: false,
        on: false,
        can_level: true,
    }
}

pub fn item_view(slot: u16) -> ItemView {
    ItemView {
        id: ItemId(slot),
        charges: Some(3),
        cooldown_left: 0,
        mode: None,
        mana_cost: 25,
        range: 400,
        aim: Some(Aim::Point),
        for_sale: false,
    }
}

pub fn loot_view() -> LootView {
    LootView {
        id: entity(21),
        pos: Vec2 {
            x: fixed(6000),
            y: fixed(6200),
        },
        item: ItemId(3),
        charges: Some(2),
    }
}

pub fn hero_unit() -> UnitView {
    UnitView {
        id: entity(7),
        kind: UnitKind::Hero,
        team: Team::Radiant,
        pos: Vec2 {
            x: fixed(4096),
            y: fixed(3200),
        },
        facing: Angle { brads: 12345 },
        hp: 640,
        max_hp: 700,
        mana: 280,
        max_mana: 350,
        move_speed: fixed(300),
        attack_damage: 54,
        attack_range: fixed(550),
        attack_interval: 51,
        attack_speed: 100,
        attributes: Attributes::ZERO,
        primary: None,
        armor: fixed(3),
        magic_resist: Fixed { raw: 16384 },
        radius: fixed(24),
        vision_radius: fixed(1800),
        true_sight_radius: fixed(0),
        statuses: StatusFlags {
            bits: StatusFlags::SLOWED | StatusFlags::DOT,
        },
        hero: Some(HeroId(0)),
        owner: Some(SlotId(0)),
        level: 6,
        abilities: (0..4).map(ability_view).collect(),
        items: (0..9)
            .map(|i| if i < 3 { Some(item_view(i)) } else { None })
            .collect(),
        effects: vec![
            EffectView {
                id: EffectId(0),
                ticks_left: Some(120),
                stacks: None,
            },
            EffectView {
                id: EffectId(11),
                ticks_left: None,
                stacks: Some(21),
            },
        ],
    }
}

pub fn creep_unit(idx: u32) -> UnitView {
    UnitView {
        id: entity(idx),
        kind: UnitKind::CreepMelee,
        team: Team::Dire,
        pos: Vec2 {
            x: fixed(2048),
            y: fixed(2048),
        },
        facing: Angle { brads: 0 },
        hp: 480,
        max_hp: 550,
        mana: 0,
        max_mana: 0,
        move_speed: fixed(325),
        attack_damage: 21,
        attack_range: fixed(100),
        attack_interval: 30,
        attack_speed: 100,
        attributes: Attributes::ZERO,
        primary: None,
        armor: fixed(2),
        magic_resist: Fixed { raw: 0 },
        radius: fixed(16),
        vision_radius: fixed(1100),
        true_sight_radius: fixed(0),
        statuses: StatusFlags::default(),
        hero: None,
        owner: None,
        level: 0,
        abilities: Vec::new(),
        items: Vec::new(),
        effects: Vec::new(),
    }
}

pub fn projectile() -> ProjectileView {
    ProjectileView {
        id: entity(99),
        pos: Vec2 {
            x: fixed(3000),
            y: fixed(3000),
        },
        facing: Angle { brads: 32768 },
        team: Team::Radiant,
        ability: Some(AbilityId(2)),
    }
}

pub fn player_view(slot: u8) -> PlayerView {
    PlayerView {
        slot: SlotId(slot),
        team: Team::Radiant,
        hero: HeroId(0),
        unit: Some(entity(7)),
        level: 6,
        xp: 2400,
        gold: Some(1200),
        stash: Some(
            (0..6)
                .map(|i| if i == 0 { Some(item_view(9)) } else { None })
                .collect(),
        ),
        kit: Some(Kit {
            abilities: vec![ability_view(0)],
            items: vec![Some(item_view(3)), None],
        }),
        kills: 3,
        deaths: 1,
        assists: 0,
        last_hits: 42,
        denies: 7,
        respawn_left: 0,
    }
}

/// A view holding one hero, `creeps` creeps, a projectile and two seats.
pub fn world_view(creeps: u32) -> WorldView {
    WorldView {
        tick: 5400,
        viewer: Some(Team::Radiant),
        units: std::iter::once(hero_unit())
            .chain((0..creeps).map(|i| creep_unit(100 + i)))
            .collect(),
        projectiles: vec![projectile()],
        players: (0..2).map(player_view).collect(),
        felled_trees: Vec::new(),
        planted_trees: Vec::new(),
        loot: vec![loot_view()],
    }
}

pub fn match_info() -> MatchInfo {
    MatchInfo {
        match_id: 0xdead_beef,
        map: MapId(1),
        tick_rate: 30,
        pregame_ticks: 900,
        trees: vec![
            Vec2 {
                x: fixed(5000),
                y: fixed(4000),
            },
            Vec2 {
                x: fixed(11384),
                y: fixed(12384),
            },
        ],
        terrain_cells: 4,
        terrain_rle: vec![(12, 0x81), (4, 0xc0)],
        opaque_cells: vec![(1, 1), (2, 1)],
        mode: TickMode::Lockstep,
        shop: vec![
            ShopEntry {
                id: ItemId(0),
                cost: 500,
                components: Vec::new(),
            },
            ShopEntry {
                id: ItemId(29),
                cost: 1400,
                components: vec![ItemId(0), ItemId(19), ItemId(13)],
            },
        ],
        picks: vec![
            Pick {
                slot: SlotId(0),
                team: Team::Radiant,
                hero: HeroId(0),
            },
            Pick {
                slot: SlotId(1),
                team: Team::Dire,
                hero: HeroId(0),
            },
        ],
    }
}

pub fn lobby_slot() -> LobbySlot {
    LobbySlot {
        slot: SlotId(0),
        team: Team::Radiant,
        name: "sylla-fan".to_string(),
        role: Some(Role::Bot),
        hero: Some(HeroId(0)),
        ready: true,
    }
}

pub fn match_stats() -> MatchStats {
    MatchStats {
        duration: 54_000,
        slots: vec![SlotStats {
            slot: SlotId(0),
            kills: 3,
            deaths: 1,
            assists: 0,
            last_hits: 42,
            denies: 7,
            net_worth: 5400,
            hero_damage: 3100,
            structure_damage: 900,
        }],
    }
}

/// Every [`Order`] variant.
pub fn all_orders() -> Vec<Order> {
    vec![
        Order::Move {
            target: Target::None,
        },
        Order::Attack {
            target: Target::None,
        },
        Order::Move {
            target: Target::Pos(Vec2 {
                x: fixed(100),
                y: fixed(-100),
            }),
        },
        Order::Attack {
            target: Target::Pos(Vec2 {
                x: fixed(0),
                y: fixed(0),
            }),
        },
        Order::Attack {
            target: Target::Unit(entity(7)),
        },
        Order::Move {
            target: Target::Unit(entity(6)),
        },
        Order::Cast {
            slot: AbilitySlot(3),
            target: Target::None,
        },
        Order::Cast {
            slot: AbilitySlot(0),
            target: Target::Pos(Vec2 {
                x: fixed(512),
                y: fixed(512),
            }),
        },
        Order::Cast {
            slot: AbilitySlot(1),
            target: Target::Unit(entity(8)),
        },
        Order::Use {
            slot: ItemSlot(2),
            target: Target::Unit(entity(9)),
        },
        Order::Learn {
            slot: AbilitySlot(2),
        },
        Order::Buy { item: ItemId(4) },
        Order::Sell { slot: ItemSlot(5) },
        Order::Swap {
            from: ItemSlot(0),
            to: ItemSlot(7),
        },
        Order::Put {
            slot: ItemSlot(1),
            target: Target::None,
        },
        Order::Put {
            slot: ItemSlot(0),
            target: Target::Pos(Vec2 {
                x: fixed(700),
                y: fixed(900),
            }),
        },
        Order::Put {
            slot: ItemSlot(6),
            target: Target::Unit(entity(10)),
        },
        Order::Take {
            target: Target::Unit(entity(21)),
        },
    ]
}

/// Every [`EventKind`] variant.
pub fn all_events() -> Vec<EventKind> {
    vec![
        EventKind::Damaged {
            source: Some(entity(7)),
            target: entity(100),
            amount: 54,
            kind: DamageKind::Physical,
            crit: true,
        },
        EventKind::Damaged {
            source: None,
            target: entity(7),
            amount: 300,
            kind: DamageKind::Pure,
            crit: false,
        },
        EventKind::Healed {
            source: None,
            target: entity(7),
            amount: 12,
        },
        EventKind::Died {
            unit: entity(100),
            killer: Some(entity(7)),
            denied: false,
        },
        EventKind::AbilityCast {
            caster: entity(7),
            ability: AbilityId(3),
        },
        EventKind::LevelUp {
            unit: entity(7),
            level: 7,
        },
        EventKind::ItemBought {
            slot: SlotId(0),
            item: ItemId(4),
        },
        EventKind::StructureDestroyed {
            unit: entity(200),
            team: Team::Dire,
        },
    ]
}

/// Every [`ClientMsg`] variant.
pub fn all_client_msgs() -> Vec<ClientMsg> {
    vec![
        ClientMsg::Hello {
            role: Role::Player,
            name: "player-one".to_string(),
        },
        ClientMsg::PickHero { hero: HeroId(0) },
        ClientMsg::SetReady(true),
        ClientMsg::Order {
            unit: None,
            seq: 12345,
            order: Order::Attack {
                target: Target::Unit(entity(7)),
            },
        },
        ClientMsg::Ack { tick: 5400 },
        ClientMsg::ViewAs {
            seat: Some(SlotId(1)),
        },
    ]
}

/// One accepted order as the wire carries it.
pub fn slot_order(slot: u8) -> SlotOrder {
    SlotOrder {
        slot: SlotId(slot),
        unit: Some(entity(30 + u32::from(slot))),
        order: Order::Attack {
            target: Target::Unit(entity(7)),
        },
    }
}

/// Every [`ServerMsg`] variant.
pub fn all_server_msgs() -> Vec<ServerMsg> {
    vec![
        ServerMsg::Welcome {
            player_id: PlayerId(3),
            slot: Some(SlotId(0)),
            tick_rate: 30,
            mode: TickMode::Realtime,
        },
        ServerMsg::LobbyState {
            slots: vec![lobby_slot()],
        },
        ServerMsg::MatchStart { info: match_info() },
        ServerMsg::Snapshot {
            view: world_view(4),
        },
        ServerMsg::Events {
            tick: 5400,
            events: all_events(),
        },
        ServerMsg::OrderRejected {
            seq: 12345,
            reason: RejectReason::UnknownTarget,
        },
        ServerMsg::MatchOver {
            winner: Team::Radiant,
            stats: match_stats(),
        },
        ServerMsg::ParticipantLeft {
            player_id: PlayerId(3),
            slot: Some(SlotId(0)),
        },
        ServerMsg::Orders {
            tick: 5400,
            orders: vec![slot_order(0), slot_order(1)],
        },
    ]
}

/// Every [`ReplayRecord`] variant.
pub fn all_replay_records() -> Vec<ReplayRecord> {
    vec![
        ReplayRecord::Msg(ServerMsg::Events {
            tick: 5400,
            events: all_events(),
        }),
        ReplayRecord::Orders {
            tick: 5400,
            orders: vec![
                SlotOrder {
                    slot: SlotId(0),
                    unit: None,
                    order: Order::Move {
                        target: Target::None,
                    },
                },
                slot_order(1),
            ],
        },
    ]
}

/// Names a variant, and fails to compile when one is added without being
/// covered above.
pub fn client_msg_name(msg: &ClientMsg) -> &'static str {
    match msg {
        ClientMsg::Hello { .. } => "Hello",
        ClientMsg::PickHero { .. } => "PickHero",
        ClientMsg::SetReady(_) => "SetReady",
        ClientMsg::Order { .. } => "Order",
        ClientMsg::Ack { .. } => "Ack",
        ClientMsg::ViewAs { .. } => "ViewAs",
    }
}

/// Names a variant, and fails to compile when one is added without being
/// covered above.
pub fn server_msg_name(msg: &ServerMsg) -> &'static str {
    match msg {
        ServerMsg::Welcome { .. } => "Welcome",
        ServerMsg::LobbyState { .. } => "LobbyState",
        ServerMsg::MatchStart { .. } => "MatchStart",
        ServerMsg::Snapshot { .. } => "Snapshot",
        ServerMsg::Events { .. } => "Events",
        ServerMsg::OrderRejected { .. } => "OrderRejected",
        ServerMsg::MatchOver { .. } => "MatchOver",
        ServerMsg::ParticipantLeft { .. } => "ParticipantLeft",
        ServerMsg::Orders { .. } => "Orders",
    }
}
