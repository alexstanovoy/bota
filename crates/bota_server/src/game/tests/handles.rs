//! What a side does out of the other's sight changes nothing in the names the
//! other side is told.

use bota_core::rules;
use bota_proto::{AbilitySlot, Order, OutgoingOrder, ServerMsg, Target, Team, encode_frame_to_vec};

use super::local::mid_config;
use crate::game::{Fnv, OrderDelay};
use crate::local::LocalMatch;

/// Ticks played: past the first creeps' clash, whose missiles and deaths
/// come and go in the radiant seat's sight.
const TICKS: u32 = rules::FIRST_WAVE_TICK + 1500;

/// Ticks between the dire hero's casts.
const CAST_EVERY: u32 = 20;

/// Per tick, the digests of what the radiant seat was sent and of the
/// radiant side's view in the simulation's own handles.
struct Seen {
    told: Vec<u64>,
    raw: Vec<u64>,
}

/// A two-fiend match where the radiant hero idles at home and the dire hero
/// learns its near raze; `busy`, it also casts it at the ground of its own
/// base, out of the radiant side's sight, whenever it can before the first
/// wave. Each raze's mark takes a slot and gives it back with its generation
/// bumped. Later, with units dying around it, a mark would also reorder which
/// freed slots the next units take, and the simulation walks its units in slot
/// order: what the other side sees would then change, not only what it is
/// called.
fn play(busy: bool) -> Seen {
    let (mut local, mut streams) =
        LocalMatch::new(&mid_config(), &[OrderDelay::NONE]).expect("a match");
    let mut seen = Seen {
        told: Vec::new(),
        raw: Vec::new(),
    };
    let near = AbilitySlot(0);
    while local.tick() < TICKS {
        let mut told = Fnv::new();
        for msg in &streams[0] {
            encode_frame_to_vec(msg)
                .expect("a message fits a frame")
                .into_iter()
                .for_each(|byte| told.u8(byte));
        }
        seen.told.push(told.done());
        let raw = ServerMsg::Snapshot {
            view: local.world().view(Team::Radiant),
        };
        let mut digest = Fnv::new();
        encode_frame_to_vec(&raw)
            .expect("a view fits a frame")
            .into_iter()
            .for_each(|byte| digest.u8(byte));
        seen.raw.push(digest.done());
        let tick = local.tick();
        let order = match tick {
            1 => Some(Order::Learn { slot: near }),
            _ if busy && tick < rules::FIRST_WAVE_TICK && tick.is_multiple_of(CAST_EVERY) => {
                Some(Order::Cast {
                    slot: near,
                    target: Target::None,
                })
            }
            _ => None,
        };
        let dire = order.map(|order| OutgoingOrder {
            seq: tick,
            unit: None,
            order,
            seen: tick,
        });
        streams = local.step(&[None, dire]).expect("a step");
    }
    seen
}

#[test]
fn what_a_side_does_out_of_sight_changes_no_name_it_tells_the_other() {
    let quiet = play(false);
    let busy = play(true);
    assert_ne!(
        quiet.raw, busy.raw,
        "the hidden razes shift the simulation's own handles in the radiant view"
    );
    let differs = quiet.told.iter().zip(&busy.told).position(|(a, b)| a != b);
    assert_eq!(
        differs, None,
        "the radiant seat was told otherwise from this tick"
    );
}
