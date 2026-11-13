//! Writes the corpus of this build's wire: the replays of the scripted matches of
//! `tests/admitted/scenarios.rs` on every item this build holds, and `corpus.txt`, what this build
//! counted of them. A later build that lists this build's wire among `bota_core::ADMITTED_WIRES`
//! plays the same matches and must count the same (`tests/admitted.rs`).
//!
//! ```text
//! cargo run --release -p bota_server --example admitted_corpus -- DIR COMMIT
//! ```
//!
//! `tools/admitted.sh corpus COMMIT` builds a commit with this program and runs it; see
//! `DEVELOPMENT.md`. The corpus is refused (nothing written, exit 1) unless the matches held every
//! item of the catalogue, carried it into a blow on a creep, and used every one that has a use to
//! effect (a channelled one to its end).

#[path = "../tests/admitted/scenarios.rs"]
mod scenarios;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use bota_core::Catalogue;
use bota_proto::ItemId;
use bota_server::replay::ReplayWriter;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(dir), Some(commit), None) = (args.next(), args.next(), args.next()) else {
        eprintln!("usage: admitted_corpus DIR COMMIT");
        std::process::exit(2);
    };
    let dir = Path::new(&dir);
    let catalogue = Catalogue::FULL;
    let mut manifest = String::new();
    let _ = writeln!(
        manifest,
        "wire {}\nitems {}\ncommit {commit}",
        bota_proto::fingerprint_to_hex(bota_core::WIRE_FINGERPRINT),
        catalogue.items()
    );
    let mut held = BTreeSet::new();
    let mut used = BTreeSet::new();
    let mut struck = BTreeSet::new();
    let mut channelled = BTreeSet::new();
    let mut replays = Vec::new();
    for (name, play) in scenarios::SCENARIOS {
        let played = play(catalogue);
        let mut records = scenarios::FOLD_START;
        for record in &played.records {
            let frame = bota_proto::encode_frame_to_vec(record).expect("a record encodes");
            scenarios::fold(&mut records, &frame);
        }
        let _ = writeln!(
            manifest,
            "scenario {name} records {} {records:016x} world {:016x} views {:016x}",
            played.records.len(),
            played.world,
            played.views
        );
        for (item, reason) in &played.refused {
            eprintln!(
                "admitted_corpus: {name}: a use of item {} was refused: {reason:?}",
                item.0
            );
        }
        held.extend(played.held.iter().copied());
        struck.extend(played.struck_creeps.iter().copied());
        used.extend(played.used.iter().copied());
        channelled.extend(played.channelled.iter().copied());
        replays.push((name, played.records));
    }
    let missing = missing(catalogue, [&held, &struck, &used, &channelled]);
    if !missing.is_empty() {
        eprintln!("admitted_corpus: the matches leave out {missing}; nothing written");
        std::process::exit(1);
    }
    for (name, records) in replays {
        let path = dir.join(format!("{name}.brp"));
        let mut writer = ReplayWriter::create(&path, None).unwrap_or_else(|error| {
            eprintln!("admitted_corpus: {error}");
            std::process::exit(1);
        });
        for record in &records {
            writer.record(record);
        }
        if writer.finish() {
            eprintln!("admitted_corpus: {} was not written whole", path.display());
            std::process::exit(1);
        }
    }
    let path = dir.join("corpus.txt");
    if let Err(error) = std::fs::write(&path, &manifest) {
        eprintln!("admitted_corpus: {}: {error}", path.display());
        std::process::exit(1);
    }
    print!("{manifest}");
}

/// The items of `catalogue` the matches never held, never carried into a blow on a creep, of a use
/// that never took effect, of a channel that never ran to its end.
fn missing(
    catalogue: Catalogue,
    [held, struck, used, channelled]: [&BTreeSet<ItemId>; 4],
) -> String {
    let mut missing = String::new();
    for item in (0..catalogue.items()).map(ItemId) {
        let Some(def) = catalogue.item(item) else {
            continue;
        };
        let channel = def.behavior.has(bota_core::Behavior::CHANNELLED);
        let gaps = [
            ("held", !held.contains(&item)),
            ("struck", !struck.contains(&item)),
            ("used", def.aim.is_some() && !used.contains(&item)),
            (
                "channelled",
                def.aim.is_some() && channel && !channelled.contains(&item),
            ),
        ];
        for (gap, _) in gaps.into_iter().filter(|(_, missing)| *missing) {
            let _ = write!(missing, " {gap} {}", def.key);
        }
    }
    missing
}
