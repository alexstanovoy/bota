//! Every message survives a trip through the wire format unchanged.

use super::fixtures::*;
use crate::*;

fn roundtrip<T>(value: &T) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let frame = encode_frame_to_vec(value).expect("encode");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    reader
        .next_message::<T>()
        .expect("decode")
        .expect("one whole frame was pushed")
}

#[test]
fn client_messages_survive() {
    for msg in all_client_msgs() {
        assert_eq!(roundtrip(&msg), msg, "{}", client_msg_name(&msg));
    }
}

#[test]
fn server_messages_survive() {
    for msg in all_server_msgs() {
        assert_eq!(roundtrip(&msg), msg, "{}", server_msg_name(&msg));
    }
}

#[test]
fn orders_survive() {
    for order in all_orders() {
        assert_eq!(roundtrip(&order), order, "{order:?}");
    }
}

#[test]
fn events_survive() {
    for event in all_events() {
        assert_eq!(roundtrip(&event), event, "{event:?}");
    }
}

#[test]
fn replay_records_survive() {
    for record in all_replay_records() {
        assert_eq!(roundtrip(&record), record, "{record:?}");
    }
}

#[test]
fn extreme_scalars_survive() {
    for raw in [0, 1, -1, i32::MIN, i32::MAX] {
        let value = Fixed { raw };
        assert_eq!(roundtrip(&value), value, "{value:?}");
    }
    for brads in [0u16, 1, 16384, 32768, 65535] {
        let angle = Angle { brads };
        assert_eq!(roundtrip(&angle), angle);
    }
}

#[test]
fn absent_optionals_stay_absent() {
    let mut view = player_view(0);
    view.gold = None;
    view.unit = None;
    let decoded = roundtrip(&view);
    assert_eq!(decoded.gold, None, "enemy gold must not appear");
    assert_eq!(
        decoded.unit, None,
        "a hero kept from a side must not appear"
    );
}

#[test]
fn empty_collections_survive() {
    let view = empty_world_view();
    assert_eq!(roundtrip(&view), view);
}
