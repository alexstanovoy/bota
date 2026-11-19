use bota_proto::{Cheat, ControlKind, Target, decode_payload, encode_frame_to_vec};
#[test]
fn generic_control_appends_after_stun_without_renumbering_either_tag() {
    for (control, tag) in [
        (ControlKind::Stun, 0),
        (ControlKind::Root, 1),
        (ControlKind::Disarm, 2),
        (ControlKind::Silence, 3),
    ] {
        let cheat = Cheat::Control {
            target: Target::None,
            control,
            ticks: 3,
        };
        let bytes = encode_frame_to_vec(&cheat).unwrap();
        assert_eq!(&bytes[4..], &[31, 0, tag, 3]);
        assert_eq!(decode_payload::<Cheat>(&bytes[4..]).unwrap(), cheat);
    }
    let bytes = encode_frame_to_vec(&Cheat::Stun {
        target: Target::None,
        ticks: 3,
    })
    .unwrap();
    assert_eq!(bytes[4], 30);
}
