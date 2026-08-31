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
    let decoded = reader
        .next_message::<T>()
        .expect("decode")
        .expect("one whole frame was pushed");
    assert_eq!(reader.buffered(), 0, "frame was not fully consumed");
    decoded
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
fn world_view_survives() {
    let view = world_view(24);
    assert_eq!(roundtrip(&view), view);
}

#[test]
fn negative_and_extreme_fixed_survive() {
    let values = [
        Fixed { raw: 0 },
        Fixed { raw: 1 },
        Fixed { raw: -1 },
        Fixed { raw: i32::MIN },
        Fixed { raw: i32::MAX },
        fixed(-8192),
        fixed(8191),
    ];
    for v in values {
        assert_eq!(roundtrip(&v), v, "{v:?}");
    }
}

#[test]
fn angle_wraps_are_preserved() {
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
    assert_eq!(decoded.unit, None, "a dead hero must not gain a unit");
}

#[test]
fn empty_collections_survive() {
    let view = WorldView {
        tick: 0,
        viewer: None,
        units: Vec::new(),
        projectiles: Vec::new(),
        players: Vec::new(),
        felled_trees: Vec::new(),
        planted_trees: Vec::new(),
        loot: Vec::new(),
    };
    assert_eq!(roundtrip(&view), view);
}

#[test]
fn every_reject_reason_survives() {
    let reasons = [
        RejectReason::NotYourSlot,
        RejectReason::HeroDead,
        RejectReason::UnknownTarget,
        RejectReason::WrongTargetKind,
        RejectReason::OnCooldown,
        RejectReason::NotEnoughMana,
        RejectReason::NotEnoughGold,
        RejectReason::EmptySlot,
        RejectReason::UnknownItem,
        RejectReason::CannotLevelUp,
        RejectReason::NotAtShop,
        RejectReason::InventoryFull,
        RejectReason::Disabled,
        RejectReason::NotPlaying,
        RejectReason::NotYourItem,
        RejectReason::ClosedGround,
        RejectReason::NotInBag,
    ];
    for reason in reasons {
        assert_eq!(roundtrip(&reason), reason, "{reason:?}");
    }
}

#[test]
fn every_unit_kind_survives() {
    let kinds = [
        UnitKind::Hero,
        UnitKind::CreepMelee,
        UnitKind::CreepFlagbearer,
        UnitKind::CreepRanged,
        UnitKind::CreepSiege,
        UnitKind::CreepNeutral,
        UnitKind::Tower,
        UnitKind::Ancient,
        UnitKind::Barracks,
        UnitKind::Fountain,
        UnitKind::Ward,
        UnitKind::Courier,
    ];
    for kind in kinds {
        assert_eq!(roundtrip(&kind), kind, "{kind:?}");
    }
}
