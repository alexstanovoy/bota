use super::*;

fn frames(bytes: &[u8]) -> Vec<MetaRecord> {
    let mut reader = bota_proto::FrameReader::new();
    reader.push(bytes);
    std::iter::from_fn(|| reader.next_message().expect("valid metadata frame")).collect()
}

#[test]
fn permanent_departures_survive_the_normal_budget_in_a_match_without_a_result() {
    let mut recorder = Recorder {
        started: true,
        ..Recorder::default()
    };
    let sample = MetaRecord::Sample {
        tick: 1,
        slot: 0,
        stats: vec![(0, 1); 60],
    };
    while recorder.out.push(&sample, false) {}
    for slot in 0..bota_proto::limits::MAX_SEATS as u8 {
        recorder.push(&ReplayRecord::Leave {
            tick: 12,
            slot: SlotId(slot),
        });
        recorder.push(&ReplayRecord::Leave {
            tick: 99,
            slot: SlotId(slot),
        });
    }
    let bytes = recorder.finish();
    assert!(bytes.len() <= crate::limits::MAX_META_BYTES + crate::limits::END_RESERVE_BYTES);
    let leaves: Vec<_> = frames(&bytes)
        .into_iter()
        .filter_map(|record| match record {
            MetaRecord::Leave { slot, tick } => Some((slot, tick)),
            _ => None,
        })
        .collect();
    assert_eq!(
        leaves,
        (0..bota_proto::limits::MAX_SEATS as u8)
            .map(|slot| (slot, 12))
            .collect::<Vec<_>>()
    );
}

#[test]
fn simultaneous_same_kind_building_falls_have_distinct_location_companions() {
    let mut recorder = Recorder {
        started: true,
        ..Recorder::default()
    };
    let unit = |idx| EntityId { idx, generation: 1 };
    for idx in [1, 2] {
        recorder
            .structures
            .insert(unit(idx), (UnitKind::Tower, Team::Dire));
    }
    let fall = |idx, lane, tier| EventKind::StructureDestroyed {
        unit: unit(idx),
        team: Team::Dire,
        kind: UnitKind::Tower,
        lane: Some(lane),
        tier: Some(tier),
        ranged: None,
    };
    recorder.events(
        900,
        &[
            fall(1, bota_core::rules::LANE_MID, 1),
            fall(999, bota_core::rules::LANE_BOT, 4), // Unknown to the first view; no legacy record.
            fall(2, bota_core::rules::LANE_TOP, 2),
        ],
    );
    let records = frames(&recorder.finish());
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record, MetaRecord::Structure { .. }))
            .count(),
        2
    );
    let locations: Vec<_> = records
        .into_iter()
        .filter_map(|record| match record {
            MetaRecord::StructureLocation {
                index, lane, tier, ..
            } => Some((index, lane, tier)),
            _ => None,
        })
        .collect();
    assert_eq!(
        locations,
        [
            (0, Some(bota_core::rules::LANE_MID), Some(1)),
            (1, Some(bota_core::rules::LANE_TOP), Some(2))
        ]
    );
}
