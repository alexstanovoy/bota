//! Landings retain their missile identity through framing and side-stream translation.
use crate::*;

fn id(idx: u32) -> EntityId {
    EntityId { idx, generation: 1 }
}

fn landing(missile: Option<EntityId>) -> EventKind {
    EventKind::Landed {
        inflictor: Inflictor::Item(ItemId::BLOOD_GRENADE),
        caster: Some(id(7)),
        at: Vec2::from_ints(1500, 1000),
        missile,
    }
}

#[test]
fn a_landing_roundtrips_its_optional_missile_identity() {
    for missile in [None, Some(id(9))] {
        let expected = landing(missile);
        let bytes = encode_frame_to_vec(&expected).expect("a landing encodes");
        let mut reader = FrameReader::new();
        reader.push(&bytes);
        assert_eq!(
            reader
                .next_message::<EventKind>()
                .expect("the landing decodes"),
            Some(expected)
        );
    }
}

#[test]
fn a_landings_caster_and_missile_are_both_translated() {
    let mut event = landing(Some(id(9)));
    event.visit_ids(&mut |id| id.idx += 100);
    let mut expected = landing(Some(id(109)));
    if let EventKind::Landed { caster, .. } = &mut expected {
        *caster = Some(id(107));
    }
    assert_eq!(event, expected);
}
