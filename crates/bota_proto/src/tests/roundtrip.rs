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
fn attack_launch_appends_event_tag_twenty_six_and_maps_only_disclosed_handles() {
    let mut event = EventKind::AttackLaunched { attacker: entity(7), target: Some(entity(9)) };
    assert_eq!(postcard::to_allocvec(&event).expect("encode")[0], 26);
    event.visit_ids(&mut |id| id.idx += 100);
    assert_eq!(event, EventKind::AttackLaunched { attacker: entity(107), target: Some(entity(109)) });
    assert_eq!(roundtrip(&event), event);
    let concealed = EventKind::AttackLaunched { attacker: entity(7), target: None };
    assert_eq!(roundtrip(&concealed), concealed);
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

#[test]
fn secret_shop_sequences_are_bounded_before_decoding() {
    for (shops, corners) in [(MAX_SECRET_SHOPS + 1, 3), (1, MAX_SHOP_ZONE_CORNERS + 1)] {
        let mut info = match_info();
        info.secret_shops = vec![
            ShopZoneView {
                pos: Vec2::ZERO,
                corners: vec![Vec2::ZERO; corners]
            };
            shops
        ];
        let bytes = encode_frame_to_vec(&ServerMsg::MatchStart { info })
            .expect("encode invalid server data");
        let mut reader = FrameReader::new();
        reader.push(&bytes);
        assert!(
            matches!(
                reader.next_message::<ServerMsg>(),
                Err(CodecError::Malformed)
            ),
            "shops={shops} corners={corners}"
        );
    }
}

#[test]
fn death_gold_awards_are_bounded_before_decoding() {
    let award = GoldAward {
        recipient: SlotId(0),
        team: Team::Radiant,
        kind: GoldAwardKind::FlagbearerBonus,
        amount: 49,
    };
    let death = |count| EventKind::Died {
        credited_hero: None,
        unit: entity(100),
        killer: Some(entity(7)),
        denied: false,
        gold: 0,
        bounty: Some(86),
        assists: 0,
        awards: Some(Box::new(vec![award; count])),
    };
    let maximum = death(MAX_DEATH_GOLD_AWARDS);
    assert_eq!(roundtrip(&maximum), maximum);
    let bytes =
        encode_frame_to_vec(&death(MAX_DEATH_GOLD_AWARDS + 1)).expect("encode invalid award list");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    assert!(matches!(
        reader.next_message::<EventKind>(),
        Err(CodecError::Malformed)
    ));
}

#[test]
fn assist_gold_appends_tag_two_without_changing_existing_award_tags() {
    assert_eq!(
        postcard::to_allocvec(&GoldAwardKind::KillBounty).expect("kill tag"),
        [0]
    );
    assert_eq!(
        postcard::to_allocvec(&GoldAwardKind::FlagbearerBonus).expect("flag tag"),
        [1]
    );
    let kind: GoldAwardKind = postcard::from_bytes(&[2]).expect("append-only assist gold tag");
    let award = GoldAward {
        recipient: SlotId(2),
        team: Team::Radiant,
        kind,
        amount: 77,
    };
    assert_eq!(roundtrip(&award), award);
}

#[test]
fn death_gold_awards_hold_each_component_for_every_seat() {
    let mut awards = Vec::new();
    for seat in 0..MAX_SEATS {
        for kind in [
            GoldAwardKind::KillBounty,
            GoldAwardKind::AssistGold,
            GoldAwardKind::FlagbearerBonus,
        ] {
            awards.push(GoldAward {
                recipient: SlotId(seat as u8),
                team: Team::Radiant,
                kind,
                amount: 66,
            });
        }
    }
    let death = EventKind::Died {
        credited_hero: None,
        unit: entity(100),
        killer: None,
        denied: false,
        gold: 0,
        bounty: Some(133),
        assists: 0,
        awards: Some(Box::new(awards)),
    };
    assert_eq!(roundtrip(&death), death);
    let mut oversized = death;
    if let EventKind::Died {
        awards: Some(awards),
        ..
    } = &mut oversized
    {
        awards.push(awards[0]);
    }
    let bytes = encode_frame_to_vec(&oversized).expect("encode excess awards");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    assert!(
        matches!(
            reader.next_message::<EventKind>(),
            Err(CodecError::Malformed)
        ),
        "the expanded list remains bounded"
    );
}

#[test]
fn credited_hero_is_appended_after_legacy_death_fields_and_remapped() {
    // Died remains variant 3; its original fields 0..6 retain their byte order.
    let original = (
        3u32,
        entity(100),
        Some(entity(7)),
        false,
        0i32,
        Some(133i32),
        0u16,
        None::<Box<Vec<GoldAward>>>,
    );
    let mut payload = postcard::to_allocvec(&original).expect("legacy death prefix");
    payload.extend(postcard::to_allocvec(&Some(entity(8))).expect("new optional hero"));
    let (mut death, rest) =
        postcard::take_from_bytes::<EventKind>(&payload).expect("decode extended death");
    assert!(
        rest.is_empty(),
        "the new hero is field 7, immediately after the old fields"
    );
    assert!(matches!(death, EventKind::Died { killer: Some(killer), .. } if killer == entity(7)));
    let mut visited = Vec::new();
    death.visit_ids(&mut |id| visited.push(*id));
    assert_eq!(visited, [entity(100), entity(7), entity(8)]);
    assert_eq!(postcard::to_allocvec(&death).unwrap(), payload);
}

#[test]
fn death_credit_event_batches_keep_the_decoded_memory_limit() {
    assert!(
        MAX_EVENTS_PER_BATCH * size_of::<EventKind>() + 3 * MAX_PAYLOAD_LEN <= MAX_DECODED_BYTES
    );
    // The minimum-size event makes both the exact-cap and over-cap payloads
    // fit the wire budget. Reading an extra entry must fail, never truncate.
    #[derive(serde::Deserialize, serde::Serialize, Debug, PartialEq)]
    struct Batch(
        #[serde(deserialize_with = "crate::bounded::seq::<_, _, { MAX_EVENTS_PER_BATCH }>")]
        Vec<EventKind>,
    );
    let mut batch = Batch(vec![
        EventKind::TreeGrew { at: Vec2::ZERO };
        MAX_EVENTS_PER_BATCH
    ]);
    assert_eq!(roundtrip(&batch), batch);
    batch.0.push(EventKind::TreeGrew { at: Vec2::ZERO });
    let bytes = postcard::to_allocvec(&batch).unwrap();
    assert!(bytes.len() <= MAX_PAYLOAD_LEN);
    assert_eq!(
        postcard::from_bytes::<Batch>(&bytes).unwrap_err(),
        postcard::Error::SerdeDeCustom
    );
}

#[test]
fn a_snapshot_lair_list_is_bounded_before_it_is_folded() {
    let mut view = empty_world_view();
    let lair = LairView {
        lair: 0,
        fell_at: None,
        alive_known_by: SeenBy {
            radiant: false,
            dire: false,
        },
    };
    view.lairs = vec![lair; limits::MAX_LAIRS];
    assert_eq!(roundtrip(&view), view);
    view.lairs.push(lair);
    let frame = encode_frame_to_vec(&view).expect("small encoded frame");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    assert_eq!(
        reader.next_message::<WorldView>(),
        Err(CodecError::Malformed)
    );
}
