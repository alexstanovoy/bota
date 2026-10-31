//! An in-process match tells each seat exactly what a server would.

use bota_core::{MAP2_ID, map_of, rules};
use bota_proto::{
    EntityId, EventKind, Order, OutgoingOrder, ReplayRecord, ServerMsg, Target, Team,
};

use crate::game::{MatchConfig, OrderDelay};
use crate::local::LocalMatch;

use super::support::{config, fiend_against_fiend};

pub(super) fn mid_config() -> MatchConfig {
    MatchConfig {
        starting_gold_lost: Vec::new(),
        economy_events: Vec::new(),
        match_id: 5,
        stream: 5,
        master_key: [9; 32],
        picks: fiend_against_fiend(),
        map: MAP2_ID,
        ack_timeout_ticks: 150,
        ..config()
    }
}

/// What a seat has been told of the match so far: its side and every hero
/// it has seen, a fallen one included.
#[derive(Default)]
struct Seen {
    viewer: Option<Team>,
    heroes: Vec<(EntityId, Team)>,
}

/// The non-hero deaths a seat is told the other side's hero struck last,
/// with the gold it is told each paid, across one step's messages.
fn enemy_kills_told(stream: &[ServerMsg], seen: &mut Seen) -> Vec<Option<i32>> {
    let mut told = Vec::new();
    for msg in stream {
        match msg {
            ServerMsg::Snapshot { view } => {
                seen.viewer = view.viewer;
                for player in &view.players {
                    if let Some(unit) = player.unit
                        && !seen.heroes.iter().any(|(hero, _)| *hero == unit)
                    {
                        seen.heroes.push((unit, player.team));
                    }
                }
            }
            ServerMsg::Events { events, .. } => {
                for kind in events {
                    let EventKind::Died {
                        unit,
                        killer: Some(killer),
                        bounty,
                        ..
                    } = kind
                    else {
                        continue;
                    };
                    let team_of = |id| {
                        seen.heroes
                            .iter()
                            .find(|(hero, _)| *hero == id)
                            .map(|h| h.1)
                    };
                    let by_enemy = team_of(*killer).is_some_and(|team| Some(team) != seen.viewer);
                    if by_enemy && team_of(*unit).is_none() {
                        told.push(*bounty);
                    }
                }
            }
            _ => {}
        }
    }
    told
}

#[test]
fn a_non_hero_kills_gold_is_hidden_from_the_other_side() {
    let cfg = mid_config();
    let (mut local, mut streams) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    let map = map_of(cfg.map);
    let mut hidden = 0;
    let mut ordered = false;
    let mut seen = [Seen::default(), Seen::default()];
    for _ in 0..rules::FIRST_WAVE_TICK + 4000 {
        for (stream, seen) in streams.iter().zip(&mut seen) {
            for gold in enemy_kills_told(stream, seen) {
                assert_eq!(gold, None, "a side was told the other's creep gold");
                hidden += 1;
            }
        }
        let tick = local.tick();
        let order = |slot: usize| OutgoingOrder {
            seq: tick,
            unit: None,
            order: Order::Attack {
                target: Target::Pos(map.fountains[1 - slot]),
            },
            seen: tick,
            queue: false,
        };
        // Told once: the attack-move runs on, and a fresh order would break each swing off.
        let first = !std::mem::replace(&mut ordered, true);
        streams = local
            .step(&[
                first.then(|| order(0)).as_slice(),
                first.then(|| order(1)).as_slice(),
            ])
            .expect("a step");
        if streams[0]
            .iter()
            .any(|m| matches!(m, ServerMsg::MatchOver { .. }))
        {
            break;
        }
    }
    assert!(hidden > 0, "no creep kill by the other hero was seen");
}

/// What `examples/local_match.rs --replay` writes: a viewer reads the records as a server's replay.
#[test]
fn a_recorded_match_keeps_the_records_of_a_servers_replay() {
    let cfg = mid_config();
    let (mut local, _) = LocalMatch::new_recorded(&cfg, &[OrderDelay::NONE]).expect("a match");
    let attack = OutgoingOrder {
        seq: 1,
        unit: None,
        order: Order::Attack {
            target: Target::Pos(map_of(cfg.map).fountains[1]),
        },
        seen: 1,
        queue: false,
    };
    local.step(&[&[attack], &[]]).expect("a step");
    local.step(&[&[], &[]]).expect("a step");
    let records = local.take_records();
    let ticks: Vec<u32> = records
        .iter()
        .filter_map(|record| match record {
            ReplayRecord::Orders { tick, .. } => Some(*tick),
            _ => None,
        })
        .collect();
    assert_eq!(
        ticks,
        [1, 2, 3],
        "one Orders record per tick, from the first"
    );
    let snapshots = records
        .iter()
        .filter(|record| matches!(record, ReplayRecord::Msg(ServerMsg::Snapshot { view }) if view.viewer.is_none()))
        .count();
    assert_eq!(snapshots, 3, "the whole map's snapshot every tick");
    assert!(
        records.iter().any(|record| matches!(
            record,
            ReplayRecord::Orders { tick: 2, orders } if orders.len() == 1
        )),
        "the order is kept on the tick it was applied"
    );
    assert!(
        local.take_records().is_empty(),
        "the records are handed out once"
    );
}
