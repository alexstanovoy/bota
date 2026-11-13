//! Every wire this build admits (`bota_core::ADMITTED_WIRES`) is one whose rules it plays
//! unchanged, on the evidence of the wire's corpus: the scripted matches of
//! `admitted/scenarios.rs` as a build of that wire recorded them (`tools/admitted.sh corpus`),
//! which this build plays again to the same records, snapshots and world fingerprints, and whose
//! replays this build's player plays to their end.

#[path = "admitted/scenarios.rs"]
mod scenarios;

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use bota_client_core::{ReplayPlayer, UnpackedReplay};
use bota_core::{ADMITTED_WIRES, AdmittedWire};
use bota_proto::{FrameReader, ItemId, ReplayRecord, ServerMsg, fingerprint_to_hex};

/// Where the corpora are kept, one directory a wire.
fn corpora() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/admitted")
}

/// What a corpus' `corpus.txt` says of one scripted match.
struct Counted {
    records: (usize, u64),
    world: u64,
    views: u64,
}

/// The corpus of `admitted`: what its build counted of each match, by name. An error says what is
/// missing or does not belong to the wire.
fn manifest(admitted: AdmittedWire) -> Result<Vec<(String, Counted)>, String> {
    let wire = fingerprint_to_hex(admitted.wire);
    let path = corpora().join(&wire).join("corpus.txt");
    let text = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "wire {wire} has no corpus ({}: {error}); write it with `tools/admitted.sh corpus \
COMMIT` from a commit of that wire",
            path.display()
        )
    })?;
    let mut counted = Vec::new();
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let hex = |word: &str| u64::from_str_radix(word, 16).map_err(|_| format!("{line:?}"));
        match words.as_slice() {
            ["wire", told] if *told != wire => {
                return Err(format!(
                    "the corpus of wire {wire} says it is of wire {told}"
                ));
            }
            ["items", told] if *told != admitted.catalogue.items.to_string() => {
                return Err(format!(
                    "the corpus of wire {wire} holds {told} items, the wire {}",
                    admitted.catalogue.items
                ));
            }
            [
                "scenario",
                name,
                "records",
                count,
                records,
                "world",
                world,
                "views",
                views,
            ] => {
                let count = count.parse().map_err(|_| format!("{line:?}"))?;
                counted.push((
                    (*name).to_owned(),
                    Counted {
                        records: (count, hex(records)?),
                        world: hex(world)?,
                        views: hex(views)?,
                    },
                ));
            }
            _ => {}
        }
    }
    Ok(counted)
}

/// The records of a replay file, its header first.
fn replay_records(path: &Path) -> Result<Vec<ReplayRecord>, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut unpacked = UnpackedReplay::new(file);
    let mut frames = FrameReader::new();
    let mut records = Vec::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let got = unpacked
            .read(&mut buffer)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if got == 0 {
            return Ok(records);
        }
        frames.push(&buffer[..got]);
        while let Some(record) = frames
            .next_message::<ReplayRecord>()
            .map_err(|error| format!("{}: {error}", path.display()))?
        {
            records.push(record);
        }
    }
}

/// How this build's play of the scripted match `name` departs from the corpus of `admitted`:
/// nothing when it records, shows and fingerprints the same.
fn departure(admitted: AdmittedWire, name: &str, counted: &Counted) -> Option<String> {
    let wire = fingerprint_to_hex(admitted.wire);
    let Some((_, play)) = scenarios::SCENARIOS
        .iter()
        .find(|(known, _)| *known == name)
    else {
        return Some(format!(
            "{wire}: the corpus has a match {name} no script plays"
        ));
    };
    let path = corpora().join(&wire).join(format!("{name}.brp"));
    let recorded = match replay_records(&path) {
        Ok(records) => records,
        Err(error) => return Some(format!("{wire}: {error}")),
    };
    let Some((ReplayRecord::Header(header), recorded)) = recorded.split_first() else {
        return Some(format!("{wire}: {name}.brp starts with no header"));
    };
    if header.wire_fingerprint != admitted.wire {
        return Some(format!(
            "{wire}: {name}.brp was recorded by wire {}",
            fingerprint_to_hex(header.wire_fingerprint)
        ));
    }
    let played = play(admitted.items());
    if let Some(at) = (0..recorded.len().max(played.records.len()))
        .find(|&at| recorded.get(at) != played.records.get(at))
    {
        return Some(format!(
            "{wire}: {name}: record {at} of {} departs: {}",
            recorded.len(),
            difference(recorded.get(at), played.records.get(at))
        ));
    }
    let mut folded = scenarios::FOLD_START;
    for record in &played.records {
        scenarios::fold(
            &mut folded,
            &bota_proto::encode_frame_to_vec(record).expect("a record encodes"),
        );
    }
    let ours = ((played.records.len(), folded), played.world, played.views);
    let theirs = (counted.records, counted.world, counted.views);
    (ours != theirs).then(|| {
        format!("{wire}: {name}: (records, world, views) {ours:x?}, the corpus counted {theirs:x?}")
    })
}

/// Where two records part: for snapshots of one tick the first unit told otherwise, and for
/// any two records the stretch of their debug forms around the first character that differs.
fn difference(corpus: Option<&ReplayRecord>, ours: Option<&ReplayRecord>) -> String {
    if let (
        Some(ReplayRecord::Msg(ServerMsg::Snapshot { view: theirs })),
        Some(ReplayRecord::Msg(ServerMsg::Snapshot { view: mine })),
    ) = (corpus, ours)
    {
        let unit = (0..theirs.units.len().max(mine.units.len()))
            .find(|&at| theirs.units.get(at) != mine.units.get(at));
        if let Some(at) = unit {
            let (theirs_told, mine_told) = around(&theirs.units.get(at), &mine.units.get(at));
            return format!(
                "the snapshot of tick {} tells unit {at} as …{theirs_told}… in the corpus, as \
…{mine_told}… here",
                theirs.tick
            );
        }
    }
    let (theirs_told, mine_told) = around(&corpus, &ours);
    format!("the corpus has …{theirs_told}…, this build …{mine_told}…")
}

/// The debug forms of two values around the first character where they differ.
fn around(theirs: &dyn std::fmt::Debug, mine: &dyn std::fmt::Debug) -> (String, String) {
    let (theirs, mine) = (format!("{theirs:?}"), format!("{mine:?}"));
    let parted = theirs
        .char_indices()
        .zip(mine.chars())
        .find(|((_, a), b)| a != b)
        .map_or(theirs.len().min(mine.len()), |((at, _), _)| at);
    let window = |told: &str| {
        let from = told.floor_char_boundary(parted.saturating_sub(300));
        let to = told.floor_char_boundary((parted + 300).min(told.len()));
        told[from..to].to_owned()
    };
    (window(&theirs), window(&mine))
}

/// Whether this build's replay player plays a replay of the corpus to its end.
fn plays_to_its_end(path: &Path) -> Result<(), String> {
    let mut player =
        ReplayPlayer::load(path).map_err(|error| format!("{}: {error}", path.display()))?;
    player.advance_ticks(1e9);
    for _ in 0..100_000 {
        player.poll(0.0);
        if let Some(error) = player.error() {
            return Err(format!("{}: {error}", path.display()));
        }
        if player.finished() {
            return Ok(());
        }
    }
    Err(format!("{} never finished playing", path.display()))
}

/// Every admitted wire has its corpus, and this build plays each of its matches to the records,
/// snapshots and world fingerprints the wire's build recorded, and its player plays the replays to
/// their end. A change of an old item's or hero's rules departs from them: the wire is then not
/// one this build may admit.
#[test]
fn every_admitted_wire_has_a_corpus_this_build_plays_alike() {
    let mut failures = Vec::new();
    for &admitted in ADMITTED_WIRES {
        let counted = match manifest(admitted) {
            Ok(counted) => counted,
            Err(error) => {
                failures.push(error);
                continue;
            }
        };
        let named: BTreeSet<&str> = counted.iter().map(|(name, _)| name.as_str()).collect();
        for (name, _) in scenarios::SCENARIOS {
            if !named.contains(name) {
                failures.push(format!(
                    "{}: the corpus lacks the match {name}",
                    fingerprint_to_hex(admitted.wire)
                ));
            }
        }
        for (name, counted) in &counted {
            failures.extend(departure(admitted, name, counted));
            let path = corpora()
                .join(fingerprint_to_hex(admitted.wire))
                .join(format!("{name}.brp"));
            failures.extend(plays_to_its_end(&path).err());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every corpus kept is of a wire this build admits: one of a wire it no longer admits goes with
/// its entry.
#[test]
fn every_corpus_is_of_an_admitted_wire() {
    let admitted: BTreeSet<String> = ADMITTED_WIRES
        .iter()
        .map(|admitted| fingerprint_to_hex(admitted.wire))
        .collect();
    let mut kept = BTreeSet::new();
    for entry in std::fs::read_dir(corpora()).expect("the corpora directory") {
        let entry = entry.expect("a directory entry");
        if entry.file_type().expect("its type").is_dir() {
            kept.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    assert_eq!(kept, admitted);
}

/// The scripted matches hold every item of an admitted catalogue but the retired as a fight
/// begins, and use every one of them that has a use: the corpus covers each item's rules.
#[test]
fn the_scripted_matches_hold_and_use_every_item_of_an_admitted_catalogue() {
    for &admitted in ADMITTED_WIRES {
        let items = admitted.items();
        let mut held = BTreeSet::new();
        let mut used = BTreeSet::new();
        for (_, play) in scenarios::SCENARIOS {
            let played = play(items);
            held.extend(played.held);
            used.extend(played.used);
        }
        let live: BTreeSet<ItemId> = (0..items.items())
            .map(ItemId)
            .filter(|&item| items.item(item).is_some())
            .collect();
        let usable: BTreeSet<ItemId> = live
            .iter()
            .copied()
            .filter(|&item| items.item(item).is_some_and(|def| def.aim.is_some()))
            .collect();
        assert_eq!(held, live, "held");
        assert_eq!(used, usable, "used");
    }
}
