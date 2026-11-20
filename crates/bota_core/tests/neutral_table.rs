//! A digest of the neutral kind and ability definitions for the current patch.

use bota_core::{NEUTRAL_ABILITIES, NEUTRALS};

/// FNV-1a of the dump, 64 bits.
fn digest(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The dump: each kind (and the raw regeneration of its health and mana), then each ability.
/// `offered`, a field the generated table added, is left out so that the digest stays comparable with the hand-written table's.
fn dump() -> String {
    let mut out = String::new();
    for neutral in NEUTRALS.iter() {
        out.push_str(&format!("{neutral:#?}\n"));
        out.push_str(&format!(
            "RAW {:?} {} {}\n",
            neutral.kind, neutral.def.hp_regen.raw, neutral.def.mana_regen.raw
        ));
    }
    for ability in NEUTRAL_ABILITIES.iter() {
        out.push_str(&format!("{ability:#?}\n"));
    }
    out.replace("    offered: true,\n", "")
}

/// The digest of the neutral definitions, including retained abilities.
const PINNED: u64 = 0xff5c_be04_832e_c1e8;

#[test]
fn the_neutral_definitions_keep_their_reviewed_values() {
    assert_eq!(
        digest(&dump()),
        PINNED,
        "a neutral definition changed: verify the intended changes before updating the digest"
    );
}

#[test]
fn replaced_neutral_spells_keep_their_old_wire_identity() {
    use bota_core::{NeutralAbility, NeutralKind, Spell, Strike};
    use bota_proto::AbilityId;
    let old = NeutralAbility::of(AbilityId(19)).expect("the original Steal Weapon");
    assert!(matches!(
        old.spell(),
        Spell::Strike(Strike::Disarm { hits: 3, .. })
    ));
    assert!(!NEUTRALS.iter().any(|unit| unit.abilities.contains(&old)));
    for id in [AbilityId(53), AbilityId(55)] {
        let old = NeutralAbility::of(id).expect("the original spell");
        assert!(!NEUTRALS.iter().any(|unit| unit.abilities.contains(&old)));
    }
    assert_ne!(
        NeutralKind::KoboldTunneler.neutral().abilities[0].id(),
        AbilityId(19)
    );
}
