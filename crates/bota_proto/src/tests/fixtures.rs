//! Sample values for the tests.
//!
//! Every struct here is built by listing all of its fields, so a new field on
//! a wire type breaks compilation here.

use crate::*;

pub fn entity(idx: u32) -> EntityId {
    EntityId { idx, generation: 1 }
}

fn fixed(units: i32) -> Fixed {
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
        mute_left: 15,
        mode: None,
        mana_cost: 25,
        range: 400,
        aim: Some(Aim::Point),
        for_sale: false,
        owner: SlotId(0),
    }
}

pub fn loot_view() -> LootView {
    LootView {
        id: entity(21),
        pos: Vec2::from_ints(6000, 6200),
        item: ItemId(3),
        charges: Some(2),
    }
}

pub fn hero_unit() -> UnitView {
    UnitView {
        id: entity(7),
        kind: UnitKind::Hero,
        team: Team::Radiant,
        pos: Vec2::from_ints(4096, 3200),
        facing: Angle { brads: 12345 },
        hp: 640,
        max_hp: 700,
        mana: 280,
        max_mana: 350,
        move_speed: fixed(300),
        attack_damage: 54,
        attack_damage_min: 54,
        attack_damage_max: 54,
        hp_regen: Fixed::ZERO,
        mana_regen: Fixed::ZERO,
        town_portal: Some(item_view(8)),
        attack_range: fixed(550),
        attack_time: 1700,
        attack_point: 500,
        attack_speed: 100,
        attributes: Attributes::ZERO,
        primary: None,
        armor: fixed(3),
        magic_resist: Fixed { raw: 16384 },
        collision: fixed(27),
        bound: fixed(24),
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
        pos: Vec2::from_ints(2048, 2048),
        facing: Angle { brads: 0 },
        hp: 480,
        max_hp: 550,
        mana: 0,
        max_mana: 0,
        move_speed: fixed(325),
        attack_damage: 21,
        attack_damage_min: 21,
        attack_damage_max: 21,
        hp_regen: Fixed::ZERO,
        mana_regen: Fixed::ZERO,
        town_portal: None,
        attack_range: fixed(100),
        attack_time: 1000,
        attack_point: 466,
        attack_speed: 100,
        attributes: Attributes::ZERO,
        primary: None,
        armor: fixed(2),
        magic_resist: Fixed { raw: 0 },
        collision: fixed(36),
        bound: fixed(16),
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
        pos: Vec2::from_ints(3000, 3000),
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
            town_portal: Some(item_view(8)),
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
        aggro: (0..creeps).map(|i| aggro_view(100 + i)).collect(),
        order_delays: vec![OrderDelayView {
            slot: SlotId(0),
            ticks: 3,
        }],
        skipped_ticks: vec![SkippedTick {
            tick: 5398,
            events: all_events().into_iter().take(2).collect(),
        }],
        stock: vec![stock_view()],
        talents: vec![TalentView {
            slot: SlotId(0),
            learned: Talents::default().with(TalentSlot(1)),
            learnable: Talents::default().with(TalentSlot(2)).with(TalentSlot(3)),
        }],
        teleports: vec![TeleportView {
            slot: SlotId(1),
            team: Team::Dire,
            building: entity(40),
            destination: Vec2::from_ints(15000, 14000),
            ticks_left: 61,
            ticks_total: 90,
        }],
    }
}

/// A shop down to one of three, the next due in a minute.
pub fn stock_view() -> StockView {
    StockView {
        team: Team::Radiant,
        item: ItemId(53),
        left: 1,
        max: 3,
        restock_left: 1800,
    }
}

/// Creep `idx` swinging at the hero, drawn onto it a moment ago.
pub fn aggro_view(idx: u32) -> AggroView {
    AggroView {
        unit: entity(idx),
        target: Some(entity(7)),
        swing: Some(SwingView {
            target: entity(7),
            strike_in: 4,
        }),
        hold_left: 52,
        chase_left: 69,
        draw_cooldown: 72,
        returning: false,
    }
}

pub fn match_info() -> MatchInfo {
    MatchInfo {
        match_id: 0xdead_beef,
        map: MapId(1),
        tick_rate: 30,
        pregame_ticks: 900,
        trees: vec![Vec2::from_ints(5000, 4000), Vec2::from_ints(11384, 12384)],
        terrain_cells: 4,
        terrain_rle: vec![(12, 0x81), (4, 0xc0)],
        opaque_cells: vec![(1, 1), (2, 1)],
        mode: TickMode::Lockstep,
        fountains: [Vec2::from_ints(1760, 2278), Vec2::from_ints(16624, 16064)],
        shop_range: 1000,
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
        name: "fan".to_string(),
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

/// Every [`Order`] variant, with every [`Target`] shape it takes.
pub fn all_orders() -> Vec<Order> {
    let mut orders = vec![
        Order::Move {
            target: Target::None,
        },
        Order::Attack {
            target: Target::None,
        },
        Order::Move {
            target: Target::Pos(Vec2::from_ints(100, -100)),
        },
        Order::Attack {
            target: Target::Pos(Vec2::from_ints(0, 0)),
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
            target: Target::Pos(Vec2::from_ints(512, 512)),
        },
        Order::Cast {
            slot: AbilitySlot(1),
            target: Target::Unit(entity(8)),
        },
        Order::Learn {
            slot: AbilitySlot(2),
        },
        Order::LearnTalent {
            talent: TalentSlot(7),
        },
    ];
    orders.extend(item_orders());
    orders.extend(cheat_orders());
    orders
}

fn item_orders() -> Vec<Order> {
    vec![
        Order::Use {
            slot: ItemSlot(2),
            target: Target::Unit(entity(9)),
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
            target: Target::Pos(Vec2::from_ints(700, 900)),
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

fn cheat_orders() -> Vec<Order> {
    [
        Cheat::Gold { amount: 500 },
        Cheat::Levels { count: 3 },
        Cheat::Refresh,
        Cheat::Item { item: ItemId(46) },
        modifier_cheat(),
        Cheat::ClearModifiers {
            target: Target::None,
        },
    ]
    .into_iter()
    .map(|cheat| Order::Cheat { cheat })
    .collect()
}

/// A modifier cheat on `entity(7)` with every field off nominal.
pub fn modifier_cheat() -> Cheat {
    Cheat::ApplyModifier {
        target: Target::Unit(entity(7)),
        spec: ModifierSpec {
            magic_resist: 1_000,
            status_resist: 2_000,
            physical_damage: 11_000,
            magic_damage: 12_000,
            pure_damage: 13_000,
            cooldown_rate: 9_000,
            mana_cost_rate: 8_000,
            move_speed: 8_500,
            max_hp: 11_000,
            max_mana: 12_000,
            gold_income: 7_500,
        },
        ticks: 900,
    }
}

/// A view of nothing at all.
pub fn empty_world_view() -> WorldView {
    WorldView {
        tick: 0,
        viewer: None,
        units: Vec::new(),
        projectiles: Vec::new(),
        players: Vec::new(),
        felled_trees: Vec::new(),
        planted_trees: Vec::new(),
        loot: Vec::new(),
        aggro: Vec::new(),
        order_delays: Vec::new(),
        skipped_ticks: Vec::new(),
        stock: Vec::new(),
        talents: Vec::new(),
        teleports: Vec::new(),
    }
}

/// Every [`EventKind`] variant.
pub fn all_events() -> Vec<EventKind> {
    vec![
        EventKind::Damaged {
            source: Some(entity(7)),
            target: entity(100),
            amount: 54,
            kind: DamageKind::Physical,
        },
        EventKind::Damaged {
            source: None,
            target: entity(7),
            amount: 300,
            kind: DamageKind::Pure,
        },
        EventKind::Missed {
            source: Some(entity(7)),
            target: entity(100),
        },
        EventKind::Healed {
            source: None,
            target: entity(7),
            amount: 12,
            mana: 40,
        },
        EventKind::Died {
            unit: entity(100),
            killer: Some(entity(7)),
            denied: false,
            gold: 43,
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
            key: Some(SeatKey([7; 32])),
            wire_fingerprint: WIRE_FINGERPRINT,
        },
        ClientMsg::PickHero { hero: HeroId(0) },
        ClientMsg::SetReady(true),
        ClientMsg::Order {
            unit: None,
            seq: 12345,
            order: Order::Attack {
                target: Target::Unit(entity(7)),
            },
            seen: Some(5399),
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
        ServerMsg::WireMismatch {
            server_fingerprint: WIRE_FINGERPRINT,
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
        ServerMsg::WireMismatch { .. } => "WireMismatch",
    }
}
