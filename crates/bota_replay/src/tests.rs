//! The durable format's contracts: its layout only grows, a later build's file reads, and the
//! trailer carries the file whole.

use std::collections::HashSet;
use std::io::Cursor;

use serde_reflection::{
    ContainerFormat, Format, Named, Registry, Tracer, TracerConfig, VariantFormat,
};

use crate::meta::{MetaHeader, MetaRecord, Writer, decode, stat};
use crate::trailer;

/// The layout as committed: one line a struct field or enum variant, with its position.
const META: &str = include_str!("../registry/meta.txt");

#[test]
fn the_meta_layout_only_grows() {
    let mut tracer = Tracer::new(TracerConfig::default());
    tracer.trace_simple_type::<MetaRecord>().expect("traces");
    tracer.trace_simple_type::<MetaHeader>().expect("traces");
    tracer
        .trace_simple_type::<crate::meta::Who>()
        .expect("traces");
    let live = layout_lines(&tracer.registry().expect("whole"));
    let live_set: HashSet<&str> = live.iter().map(String::as_str).collect();
    let committed: Vec<&str> = META
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let changed: Vec<&str> = committed
        .iter()
        .copied()
        .filter(|line| !live_set.contains(line))
        .collect();
    assert!(
        changed.is_empty(),
        "these positions changed or disappeared; keep them and retire them instead:\n{}",
        changed.join("\n")
    );
    let unregistered: Vec<&str> = live
        .iter()
        .map(String::as_str)
        .filter(|line| !committed.contains(line))
        .collect();
    assert!(
        unregistered.is_empty(),
        "append these to crates/bota_replay/registry/meta.txt:\n{}",
        unregistered.join("\n")
    );
}

/// The registry as lines, each naming one position on the wire.
fn layout_lines(registry: &Registry) -> Vec<String> {
    let mut lines = Vec::new();
    for (name, container) in registry {
        match container {
            ContainerFormat::UnitStruct => {
                lines.push(format!("unit {name}"));
            }
            ContainerFormat::NewTypeStruct(inner) => {
                lines.push(format!("newtype {name} {}", render(inner)));
            }
            ContainerFormat::TupleStruct(items) => tuple_lines(&mut lines, name, items),
            ContainerFormat::Struct(fields) => field_lines(&mut lines, name, fields),
            ContainerFormat::Enum(variants) => {
                for (index, variant) in variants {
                    variant_lines(&mut lines, name, *index, variant);
                }
            }
        }
    }
    lines
}

fn variant_lines(lines: &mut Vec<String>, owner: &str, index: u32, variant: &Named<VariantFormat>) {
    let head = format!("variant {owner} {index} {}", variant.name);
    match &variant.value {
        VariantFormat::Unit => lines.push(head),
        VariantFormat::NewType(inner) => lines.push(format!("{head}({})", render(inner))),
        VariantFormat::Tuple(items) => {
            lines.push(format!("{head}()"));
            tuple_lines(lines, &format!("{owner}::{}", variant.name), items);
        }
        VariantFormat::Struct(fields) => {
            lines.push(format!("{head} {{}}"));
            field_lines(lines, &format!("{owner}::{}", variant.name), fields);
        }
        VariantFormat::Variable(_) => panic!("{owner}::{} traced incompletely", variant.name),
    }
}

fn tuple_lines(lines: &mut Vec<String>, owner: &str, items: &[Format]) {
    for (index, item) in items.iter().enumerate() {
        lines.push(format!("element {owner} {index} {}", render(item)));
    }
}

fn field_lines(lines: &mut Vec<String>, owner: &str, fields: &[Named<Format>]) {
    for (index, field) in fields.iter().enumerate() {
        lines.push(format!(
            "field {owner} {index} {}: {}",
            field.name,
            render(&field.value)
        ));
    }
}

/// A field's type as Rust spells it, primitives in lower case.
fn render(format: &Format) -> String {
    match format {
        Format::TypeName(name) => name.clone(),
        Format::Unit => "()".to_string(),
        Format::Bool => "bool".to_string(),
        Format::I8 => "i8".to_string(),
        Format::I16 => "i16".to_string(),
        Format::I32 => "i32".to_string(),
        Format::I64 => "i64".to_string(),
        Format::I128 => "i128".to_string(),
        Format::U8 => "u8".to_string(),
        Format::U16 => "u16".to_string(),
        Format::U32 => "u32".to_string(),
        Format::U64 => "u64".to_string(),
        Format::U128 => "u128".to_string(),
        Format::F32 => "f32".to_string(),
        Format::F64 => "f64".to_string(),
        Format::Char => "char".to_string(),
        Format::Str => "String".to_string(),
        Format::Bytes => "bytes".to_string(),
        Format::Option(inner) => format!("Option<{}>", render(inner)),
        Format::Seq(inner) => format!("Vec<{}>", render(inner)),
        Format::Map { key, value } => format!("Map<{}, {}>", render(key), render(value)),
        Format::Tuple(items) => {
            let items: Vec<String> = items.iter().map(render).collect();
            format!("({})", items.join(", "))
        }
        Format::TupleArray { content, size } => format!("[{}; {size}]", render(content)),
        Format::Variable(_) => panic!("a type traced incompletely"),
    }
}

/// The stat ids are part of the format: a retired one stays reserved.
#[test]
fn the_stat_ids_are_the_released_ones() {
    use stat::*;
    assert_eq!(
        [
            GOLD,
            GOLD_EARNED,
            NET_WORTH,
            XP,
            LEVEL,
            KILLS,
            DEATHS,
            ASSISTS,
            LAST_HITS,
            DENIES,
            HERO_DAMAGE,
            STRUCTURE_DAMAGE,
            HEALING
        ],
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    );
}

fn header() -> MetaHeader {
    MetaHeader {
        magic: MetaHeader::MAGIC,
        schema: MetaHeader::SCHEMA,
        wire_fingerprint: 7,
        rules_version: 70,
        match_id: 9,
        map: 2,
        tick_rate: 30,
        pregame_ticks: 900,
        catalogue_items: 100,
        catalogue_digest: 5,
    }
}

fn file(records: &[MetaRecord]) -> Vec<u8> {
    let mut out = Writer::default();
    for record in records {
        assert!(out.push(record, false));
    }
    out.into_bytes()
}

#[test]
fn a_file_reads_back_and_a_file_of_a_later_build_reads_with_what_it_knows() {
    let known = [
        MetaRecord::Header(header()),
        MetaRecord::Seat {
            slot: 0,
            team: 0,
            hero: 3,
        },
        MetaRecord::Sample {
            tick: 900,
            slot: 0,
            stats: vec![(stat::GOLD, 600), (9_999, 5)],
        },
        MetaRecord::End {
            tick: 1800,
            winner: Some(0),
        },
    ];
    let meta = decode(&file(&known)).expect("a file");
    assert_eq!(meta.header, header());
    assert_eq!(meta.records, known[1..]);
    assert!(meta.complete);
    assert_eq!(meta.skipped, 0);

    // A later build's record, a frame whose variant this build has no number for, sits among
    // the known: it is skipped by its length, the rest read.
    let mut later = file(&known[..3]);
    let unknown = [200u8, 1, 2, 3];
    later.extend((unknown.len() as u32).to_le_bytes());
    later.extend(unknown);
    later.extend(file(&known[3..]));
    let meta = decode(&later).expect("a file of a later build");
    assert_eq!(meta.records, known[1..]);
    assert_eq!(meta.skipped, 1);
}

#[test]
fn a_file_cut_short_gives_what_came_before_and_what_is_no_file_is_refused() {
    let whole = file(&[
        MetaRecord::Header(header()),
        MetaRecord::Seat {
            slot: 0,
            team: 0,
            hero: 3,
        },
        MetaRecord::LevelUp {
            tick: 5,
            slot: 0,
            level: 2,
        },
    ]);
    let meta = decode(&whole[..whole.len() - 3]).expect("what came before");
    assert_eq!(meta.records.len(), 1);
    assert!(!meta.complete);
    assert_eq!(decode(b"not a file"), Err(crate::meta::MetaError::NotMeta));
    assert_eq!(decode(&[]), Err(crate::meta::MetaError::NotMeta));
    let long = vec![0u8; crate::limits::MAX_META_BYTES + crate::limits::END_RESERVE_BYTES + 1];
    assert_eq!(decode(&long), Err(crate::meta::MetaError::TooLong));
}

#[test]
fn the_writer_stops_at_its_bound_and_keeps_room_to_close() {
    let mut out = Writer::default();
    let record = MetaRecord::Sample {
        tick: 1,
        slot: 0,
        stats: vec![(0, 1); 60],
    };
    while out.push(&record, false) {}
    assert!(out.cut());
    assert!(out.bytes().len() <= crate::limits::MAX_META_BYTES);
    assert!(out.push(
        &MetaRecord::End {
            tick: 2,
            winner: None
        },
        true
    ));
}

#[test]
fn the_trailer_carries_the_file_whole_and_a_replay_without_one_gives_none() {
    let meta = file(&[MetaRecord::Header(header())]);
    let frame = trailer::frame(&meta).expect("a trailer");
    let replay = [b"a stream".to_vec(), frame].concat();
    let found = trailer::find(&mut Cursor::new(replay.clone())).expect("reads");
    assert_eq!(found.as_deref(), Some(&meta[..]), "byte for byte");
    // It is one skippable frame.
    let at = replay.len() - meta.len() - 12 - 8;
    assert_eq!(replay[at..at + 4], 0x184D_2A50u32.to_le_bytes());
    assert_eq!(
        trailer::find(&mut Cursor::new(
            b"a stream with no trailer at all".to_vec()
        ))
        .expect("reads"),
        None
    );
    let mut damaged = replay;
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    assert_eq!(
        trailer::find(&mut Cursor::new(damaged)).expect("reads"),
        None
    );
}
