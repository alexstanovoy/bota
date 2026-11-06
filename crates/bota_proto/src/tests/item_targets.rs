//! Unit-targeted item uses retain their subject in full streams and through id translation.

use crate::*;

fn handle(idx: u32) -> EntityId {
    EntityId { idx, generation: 1 }
}

fn item_use(target: Option<EntityId>) -> EventKind {
    EventKind::ItemUsed {
        caster: Some(handle(7)),
        item: ItemId::HEALING_SALVE,
        at: Some(Vec2::from_ints(1500, 1000)),
        target,
    }
}

#[test]
fn an_item_use_roundtrips_its_optional_unit_target() {
    for target in [None, Some(handle(9))] {
        let expected = item_use(target);
        let bytes = encode_frame_to_vec(&expected).expect("an item use encodes");
        let mut reader = FrameReader::new();
        reader.push(&bytes);
        assert_eq!(
            reader
                .next_message::<EventKind>()
                .expect("the item use decodes"),
            Some(expected)
        );
    }
}

#[test]
fn an_item_uses_caster_and_target_are_both_translated() {
    let mut event = item_use(Some(handle(9)));
    event.visit_ids(&mut |id| id.idx += 100);
    assert_eq!(
        event,
        EventKind::ItemUsed {
            caster: Some(handle(107)),
            item: ItemId::HEALING_SALVE,
            at: Some(Vec2::from_ints(1500, 1000)),
            target: Some(handle(109)),
        }
    );
}
