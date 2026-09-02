//! Budgets for how much a snapshot costs on the wire.
//!
//! Full views are sent every tick with no delta encoding, so the size of a
//! snapshot is the bandwidth of the game. These bounds are loose enough to
//! ignore ordinary changes and tight enough to catch a field that costs far
//! more than it looks.

use super::fixtures::*;
use crate::*;

fn encoded_len<T: serde::Serialize>(value: &T) -> usize {
    encode_frame_to_vec(value).expect("encode").len()
}

/// Units in a 1v1 match: two heroes, two creep waves, towers, fountains.
const UNITS_1V1: u32 = 25;

/// Ticks per second the server runs at.
const TICK_RATE: usize = 30;

#[test]
fn a_hero_stays_under_171_bytes() {
    let len = encoded_len(&hero_unit());
    assert!(len <= 171, "hero unit grew to {len} bytes");
}

#[test]
fn a_creep_stays_under_80_bytes() {
    let len = encoded_len(&creep_unit(1));
    assert!(len <= 80, "creep unit grew to {len} bytes");
}

#[test]
fn a_1v1_snapshot_fits_in_a_tcp_burst() {
    let msg = ServerMsg::Snapshot {
        view: world_view(UNITS_1V1 - 1),
    };
    let len = encoded_len(&msg);
    let per_second = len * TICK_RATE;

    assert!(
        len <= 4096,
        "snapshot of {UNITS_1V1} units grew to {len} bytes, \
         which is {} KB/s per viewer at {TICK_RATE} Hz",
        per_second / 1024,
    );
}

#[test]
fn an_order_stays_small() {
    for order in all_orders() {
        let msg = ClientMsg::Order {
            seq: 12345,
            unit: None,
            order,
        };
        let len = encoded_len(&msg);
        assert!(len <= 32, "order encoded to {len} bytes: {msg:?}");
    }
}

#[test]
fn an_empty_view_costs_almost_nothing() {
    let msg = ServerMsg::Snapshot {
        view: WorldView {
            tick: 0,
            viewer: None,
            units: Vec::new(),
            projectiles: Vec::new(),
            players: Vec::new(),
            felled_trees: Vec::new(),
            planted_trees: Vec::new(),
            loot: Vec::new(),
        },
    };
    let len = encoded_len(&msg);
    assert!(len <= 16, "an empty snapshot cost {len} bytes");
}
