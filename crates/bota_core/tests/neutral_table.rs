//! The neutral kind and ability definitions for the current patch, against their reviewed copy.
//!
//! The copy is `neutral_table.txt` beside this file: one definition after another as text, so a change shows in review
//! as the rows it changed, and two changes to different neutrals merge as text. After a reviewed change, re-record it:
//! `BOTA_RECORD_NEUTRAL_TABLE=1 cargo test --profile test-release -p bota_core --test neutral_table`.

use bota_core::{NEUTRAL_ABILITIES, NEUTRALS};

const REVIEWED: &str = include_str!("neutral_table.txt");

/// The dump: each kind (and the raw regeneration of its health and mana), then each ability.
/// `offered`, a field the generated table added, is left out so that the copy stays comparable with the hand-written table's.
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

/// The first line where `now` leaves `reviewed`, with a few lines of each around it.
fn first_difference(reviewed: &str, now: &str) -> String {
    let (old, new): (Vec<&str>, Vec<&str>) = (reviewed.lines().collect(), now.lines().collect());
    let at = old
        .iter()
        .zip(&new)
        .position(|(a, b)| a != b)
        .unwrap_or(old.len().min(new.len()));
    let around = |lines: &[&str]| lines[at.saturating_sub(3)..(at + 4).min(lines.len())].join("\n");
    format!(
        "line {}:\n--- reviewed\n{}\n+++ now\n{}",
        at + 1,
        around(&old),
        around(&new)
    )
}

#[test]
fn the_neutral_definitions_keep_their_reviewed_values() {
    let now = dump();
    if std::env::var_os("BOTA_RECORD_NEUTRAL_TABLE").is_some() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/neutral_table.txt");
        std::fs::write(path, &now).expect("the reviewed copy is writable");
        return;
    }
    assert!(
        now == REVIEWED,
        "a neutral definition changed: verify the intended changes, then re-record tests/neutral_table.txt \
         (BOTA_RECORD_NEUTRAL_TABLE=1, see the top of this file); {}",
        first_difference(REVIEWED, &now)
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
