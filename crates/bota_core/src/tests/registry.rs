//! Every table row is its line of the protocol's id registry: the same id,
//! the same key, the same liveness, and no line without a row.

use bota_proto::{IdSpace, registered};

use crate::{ABILITIES, EFFECTS, HEROES, ITEMS, NEUTRALS};

/// The rows of a table as `(id, key, retired)` are the registry's lines of
/// `space`, both ways.
fn rows_are_the_registry(space: IdSpace, rows: impl Iterator<Item = (u16, &'static str, bool)>) {
    let mut rows: Vec<_> = rows.collect();
    rows.sort_unstable();
    let mut lines: Vec<_> = registered(space)
        .map(|line| (line.id, line.name, line.retired))
        .collect();
    lines.sort_unstable();
    assert_eq!(
        rows, lines,
        "{space:?}: the table and registry/ids.txt differ"
    );
}

#[test]
fn every_row_is_its_registry_line() {
    rows_are_the_registry(
        IdSpace::Hero,
        HEROES.iter().map(|row| (row.id.0, row.key, false)),
    );
    rows_are_the_registry(
        IdSpace::Ability,
        ABILITIES.iter().map(|row| (row.id.0, row.key, row.retired)),
    );
    rows_are_the_registry(
        IdSpace::Item,
        ITEMS.iter().map(|row| (row.id.0, row.key, row.retired)),
    );
    rows_are_the_registry(
        IdSpace::Effect,
        EFFECTS.iter().map(|row| (row.id.0, row.key, row.retired)),
    );
    rows_are_the_registry(
        IdSpace::Neutral,
        NEUTRALS
            .iter()
            .map(|row| (row.kind.id().0, row.unit, false)),
    );
}
