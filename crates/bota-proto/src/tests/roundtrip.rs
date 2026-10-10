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
    assert_eq!(decoded.unit, None, "a dead hero must not gain a unit");
}

#[test]
fn empty_collections_survive() {
    let view = empty_world_view();
    assert_eq!(roundtrip(&view), view);
}

/// Each value survives and encodes as the one-byte tag of its position in
/// `values`, which lists an enum's variants in declaration order.
fn assert_tagged_in_order<T>(values: &[T])
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + core::fmt::Debug,
{
    for (index, value) in values.iter().enumerate() {
        assert_eq!(&roundtrip(value), value);
        let frame = encode_frame_to_vec(value).expect("encode");
        assert_eq!(frame[LEN_PREFIX..], [index as u8], "{value:?}");
    }
}

#[test]
fn fieldless_enums_are_tagged_in_declaration_order() {
    assert_tagged_in_order(&[Team::Radiant, Team::Dire, Team::Neutral]);
    assert_tagged_in_order(&[Aim::Own, Aim::Point, Aim::Unit, Aim::Tree, Aim::Building]);
    assert_tagged_in_order(&[
        Attribute::Strength,
        Attribute::Agility,
        Attribute::Intelligence,
    ]);
    assert_tagged_in_order(&[DamageKind::Physical, DamageKind::Magical, DamageKind::Pure]);
    assert_tagged_in_order(&[Role::Player, Role::Bot, Role::Spectator]);
    assert_tagged_in_order(&[TickMode::Realtime, TickMode::Lockstep]);
    assert_tagged_in_order(&[
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
    ]);
}

#[test]
fn reject_reasons_are_tagged_in_declaration_order() {
    assert_tagged_in_order(&[
        RejectReason::HeroDead,
        RejectReason::UnknownTarget,
        RejectReason::WrongTargetKind,
        RejectReason::OnCooldown,
        RejectReason::NotEnoughMana,
        RejectReason::NotEnoughGold,
        RejectReason::EmptySlot,
        RejectReason::NotCastable,
        RejectReason::NotLearned,
        RejectReason::NoCharges,
        RejectReason::NotReady,
        RejectReason::NotYourUnit,
        RejectReason::UnknownItem,
        RejectReason::NotAtShop,
        RejectReason::InventoryFull,
        RejectReason::Disabled,
        RejectReason::NotYourItem,
        RejectReason::ClosedGround,
        RejectReason::NotInBag,
        RejectReason::NoCheats,
        RejectReason::BadCheat,
        RejectReason::OutOfStock,
        RejectReason::CannotLearn,
    ]);
}
