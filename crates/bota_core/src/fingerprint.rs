//! The number a build's wire is known by: its registries and its rules.

use bota_proto::{REGISTRY_DIGEST, fnv1a};

/// One source file of this crate: its path under `src/` and its text.
macro_rules! source {
    ($path:literal) => {
        ($path, include_str!($path))
    };
}

/// Every source file of this crate but its tests (`src/tests/`), in path
/// order. A test fails when a file is missing here.
pub(crate) const SOURCES: [(&str, &str); COUNT] = [
    source!("ability.rs"),
    source!("ability_neutral.rs"),
    source!("aggro.rs"),
    source!("attack_class.rs"),
    source!("aura.rs"),
    source!("camp.rs"),
    source!("cells.rs"),
    source!("chance.rs"),
    source!("damage.rs"),
    source!("economy.rs"),
    source!("effect.rs"),
    source!("fingerprint.rs"),
    source!("hero.rs"),
    source!("heroes/mod.rs"),
    source!("heroes/pudge/formulas.rs"),
    source!("heroes/pudge/kit.rs"),
    source!("heroes/pudge/mod.rs"),
    source!("heroes/pudge/numbers.rs"),
    source!("heroes/shadow_fiend/formulas.rs"),
    source!("heroes/shadow_fiend/kit.rs"),
    source!("heroes/shadow_fiend/mod.rs"),
    source!("heroes/shadow_fiend/numbers.rs"),
    source!("item.rs"),
    source!("item_table.rs"),
    source!("lib.rs"),
    source!("map.rs"),
    source!("modifier.rs"),
    source!("movement.rs"),
    source!("place.rs"),
    source!("progress.rs"),
    source!("protect.rs"),
    source!("regen.rs"),
    source!("roster.rs"),
    source!("rules.rs"),
    source!("sight.rs"),
    source!("standing.rs"),
    source!("talent.rs"),
    source!("terrain.rs"),
    source!("text.rs"),
    source!("time_of_day.rs"),
    source!("unit.rs"),
    source!("unit_kind.rs"),
    source!("unit_neutral.rs"),
    source!("wave.rs"),
];

const COUNT: usize = 44;

/// FNV-1a over every source file of this crate, each path then its text.
///
/// This crate is the rules a client knows: the tables and the integer
/// functions it checks its orders against and predicts with. A server
/// whose rules moved (a balance change, a new patch's numbers) would admit
/// an older client that then fails in mid-match, so any change to them, a
/// comment's too, makes another digest: builds from different sources are
/// never taken for one.
pub const RULES_DIGEST: u64 = digest(&SOURCES);

/// The wire fingerprint: the [`REGISTRY_DIGEST`] and the [`RULES_DIGEST`]
/// together. A client's `Hello` carries it, a replay's header names it, and
/// a server refuses a client of another one with `ServerMsg::WireMismatch`
/// (see the `bota_proto` contract).
pub const WIRE_FINGERPRINT: u64 =
    fnv1a(&[&REGISTRY_DIGEST.to_le_bytes(), &RULES_DIGEST.to_le_bytes()]);

const fn digest(sources: &[(&str, &str); COUNT]) -> u64 {
    let mut parts: [&[u8]; 2 * COUNT] = [&[]; 2 * COUNT];
    let mut at = 0;
    while at < COUNT {
        parts[2 * at] = sources[at].0.as_bytes();
        parts[2 * at + 1] = sources[at].1.as_bytes();
        at += 1;
    }
    fnv1a(&parts)
}
