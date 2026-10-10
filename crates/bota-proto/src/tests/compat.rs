//! What a client built against an older wire still reads.

use serde::Deserialize;

use super::fixtures::*;
use crate::*;

/// [`WorldView`] as it stood before [`WorldView::aggro`].
#[derive(Deserialize, Debug, PartialEq)]
struct ViewBeforeAggro {
    tick: u32,
    viewer: Option<Team>,
    units: Vec<UnitView>,
    projectiles: Vec<ProjectileView>,
    players: Vec<PlayerView>,
    felled_trees: Vec<u32>,
    planted_trees: Vec<Vec2>,
    loot: Vec<LootView>,
}

/// [`ServerMsg`] as such a client declares it, down to the snapshot.
#[derive(Deserialize, Debug, PartialEq)]
enum MsgBeforeAggro {
    Welcome,
    LobbyState,
    MatchStart,
    Snapshot { view: ViewBeforeAggro },
}

#[test]
fn a_client_built_before_aggro_reads_a_snapshot_carrying_it() {
    let view = world_view(24);
    assert!(!view.aggro.is_empty());
    let frame = encode_frame_to_vec(&ServerMsg::Snapshot { view: view.clone() }).expect("encode");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    let old = reader
        .next_message::<MsgBeforeAggro>()
        .expect("decode")
        .expect("one whole frame was pushed");
    assert_eq!(reader.buffered(), 0, "the whole frame was consumed");
    assert_eq!(
        old,
        MsgBeforeAggro::Snapshot {
            view: ViewBeforeAggro {
                tick: view.tick,
                viewer: view.viewer,
                units: view.units,
                projectiles: view.projectiles,
                players: view.players,
                felled_trees: view.felled_trees,
                planted_trees: view.planted_trees,
                loot: view.loot,
            },
        }
    );
}

/// [`WorldView`] as it stood before [`WorldView::order_delays`].
#[derive(Deserialize, Debug, PartialEq)]
struct ViewBeforeOrderDelays {
    tick: u32,
    viewer: Option<Team>,
    units: Vec<UnitView>,
    projectiles: Vec<ProjectileView>,
    players: Vec<PlayerView>,
    felled_trees: Vec<u32>,
    planted_trees: Vec<Vec2>,
    loot: Vec<LootView>,
    aggro: Vec<AggroView>,
}

/// [`ServerMsg`] as such a client declares it, down to the snapshot.
#[derive(Deserialize, Debug, PartialEq)]
enum MsgBeforeOrderDelays {
    Welcome,
    LobbyState,
    MatchStart,
    Snapshot { view: ViewBeforeOrderDelays },
}

#[test]
fn a_client_built_before_order_delays_reads_a_snapshot_carrying_them() {
    let view = world_view(24);
    assert!(!view.order_delays.is_empty());
    let frame = encode_frame_to_vec(&ServerMsg::Snapshot { view: view.clone() }).expect("encode");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    let old = reader
        .next_message::<MsgBeforeOrderDelays>()
        .expect("decode")
        .expect("one whole frame was pushed");
    assert_eq!(reader.buffered(), 0, "the whole frame was consumed");
    assert_eq!(
        old,
        MsgBeforeOrderDelays::Snapshot {
            view: ViewBeforeOrderDelays {
                tick: view.tick,
                viewer: view.viewer,
                units: view.units,
                projectiles: view.projectiles,
                players: view.players,
                felled_trees: view.felled_trees,
                planted_trees: view.planted_trees,
                loot: view.loot,
                aggro: view.aggro,
            },
        }
    );
}

/// [`ClientMsg`] as a server built before [`ClientMsg::Order::seen`]
/// declares it, down to the order.
#[derive(Deserialize, Debug, PartialEq)]
enum ClientMsgBeforeSeen {
    Hello,
    PickHero,
    SetReady,
    Order {
        seq: u32,
        unit: Option<EntityId>,
        order: Order,
    },
}

#[test]
fn a_server_built_before_seen_reads_an_order_carrying_it() {
    let msg = ClientMsg::Order {
        seq: 9,
        unit: Some(entity(7)),
        order: Order::Move {
            target: Target::None,
        },
        seen: Some(5399),
    };
    let frame = encode_frame_to_vec(&msg).expect("encode");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    let old = reader
        .next_message::<ClientMsgBeforeSeen>()
        .expect("decode")
        .expect("one whole frame was pushed");
    assert_eq!(reader.buffered(), 0, "the whole frame was consumed");
    assert_eq!(
        old,
        ClientMsgBeforeSeen::Order {
            seq: 9,
            unit: Some(entity(7)),
            order: Order::Move {
                target: Target::None,
            },
        }
    );
}

/// [`WorldView`] as it stood before [`WorldView::skipped_ticks`].
#[derive(Deserialize, Debug, PartialEq)]
struct ViewBeforeSkippedTicks {
    tick: u32,
    viewer: Option<Team>,
    units: Vec<UnitView>,
    projectiles: Vec<ProjectileView>,
    players: Vec<PlayerView>,
    felled_trees: Vec<u32>,
    planted_trees: Vec<Vec2>,
    loot: Vec<LootView>,
    aggro: Vec<AggroView>,
    order_delays: Vec<OrderDelayView>,
}

/// [`ServerMsg`] as such a client declares it, down to the snapshot.
#[derive(Deserialize, Debug, PartialEq)]
enum MsgBeforeSkippedTicks {
    Welcome,
    LobbyState,
    MatchStart,
    Snapshot { view: ViewBeforeSkippedTicks },
}

#[test]
fn a_client_built_before_skipped_ticks_reads_a_snapshot_carrying_them() {
    let view = world_view(24);
    assert!(!view.skipped_ticks.is_empty());
    let frame = encode_frame_to_vec(&ServerMsg::Snapshot { view: view.clone() }).expect("encode");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    let old = reader
        .next_message::<MsgBeforeSkippedTicks>()
        .expect("decode")
        .expect("one whole frame was pushed");
    assert_eq!(reader.buffered(), 0, "the whole frame was consumed");
    assert_eq!(
        old,
        MsgBeforeSkippedTicks::Snapshot {
            view: ViewBeforeSkippedTicks {
                tick: view.tick,
                viewer: view.viewer,
                units: view.units,
                projectiles: view.projectiles,
                players: view.players,
                felled_trees: view.felled_trees,
                planted_trees: view.planted_trees,
                loot: view.loot,
                aggro: view.aggro,
                order_delays: view.order_delays,
            },
        }
    );
}

/// [`WorldView`] as it stood before [`WorldView::stock`].
#[derive(Deserialize, Debug, PartialEq)]
struct ViewBeforeStock {
    tick: u32,
    viewer: Option<Team>,
    units: Vec<UnitView>,
    projectiles: Vec<ProjectileView>,
    players: Vec<PlayerView>,
    felled_trees: Vec<u32>,
    planted_trees: Vec<Vec2>,
    loot: Vec<LootView>,
    aggro: Vec<AggroView>,
    order_delays: Vec<OrderDelayView>,
    skipped_ticks: Vec<SkippedTick>,
}

/// [`ServerMsg`] as a client built before the stock declares it.
#[derive(Deserialize, Debug, PartialEq)]
enum MsgBeforeStock {
    Welcome,
    LobbyState,
    MatchStart,
    Snapshot { view: Box<ViewBeforeStock> },
}

#[test]
fn a_client_built_before_stock_reads_a_snapshot_carrying_it() {
    let view = world_view(3);
    assert!(!view.stock.is_empty());
    let frame = encode_frame_to_vec(&ServerMsg::Snapshot { view: view.clone() }).expect("encode");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    let old = reader
        .next_message::<MsgBeforeStock>()
        .expect("decode")
        .expect("one whole frame was pushed");
    assert_eq!(reader.buffered(), 0, "the whole frame was consumed");
    let MsgBeforeStock::Snapshot { view: old } = old else {
        panic!("a snapshot decodes as a snapshot");
    };
    assert_eq!(old.skipped_ticks, view.skipped_ticks);
    assert_eq!(old.order_delays, view.order_delays);
    assert_eq!(old.units, view.units);
}

/// [`WorldView`] as it stood before [`WorldView::talents`].
#[derive(Deserialize, Debug, PartialEq)]
struct ViewBeforeTalents {
    tick: u32,
    viewer: Option<Team>,
    units: Vec<UnitView>,
    projectiles: Vec<ProjectileView>,
    players: Vec<PlayerView>,
    felled_trees: Vec<u32>,
    planted_trees: Vec<Vec2>,
    loot: Vec<LootView>,
    aggro: Vec<AggroView>,
    order_delays: Vec<OrderDelayView>,
    skipped_ticks: Vec<SkippedTick>,
    stock: Vec<StockView>,
}

/// [`ServerMsg`] as a client built before the talents declares it.
#[derive(Deserialize, Debug, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "mirrors the old wire shape, which kept the view inline"
)]
enum MsgBeforeTalents {
    Welcome,
    LobbyState,
    MatchStart,
    Snapshot { view: Box<ViewBeforeTalents> },
}

#[test]
fn a_client_built_before_talents_reads_a_snapshot_carrying_them() {
    let view = world_view(24);
    assert!(!view.talents.is_empty());
    let frame = encode_frame_to_vec(&ServerMsg::Snapshot { view: view.clone() }).expect("encode");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    let old = reader
        .next_message::<MsgBeforeTalents>()
        .expect("decode")
        .expect("one whole frame was pushed");
    assert_eq!(reader.buffered(), 0, "the whole frame was consumed");
    assert_eq!(
        old,
        MsgBeforeTalents::Snapshot {
            view: Box::new(ViewBeforeTalents {
                tick: view.tick,
                viewer: view.viewer,
                units: view.units,
                projectiles: view.projectiles,
                players: view.players,
                felled_trees: view.felled_trees,
                planted_trees: view.planted_trees,
                loot: view.loot,
                aggro: view.aggro,
                order_delays: view.order_delays,
                skipped_ticks: view.skipped_ticks,
                stock: view.stock,
            }),
        }
    );
}
