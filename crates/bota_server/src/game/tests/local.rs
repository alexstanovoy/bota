//! An in-process match tells each seat exactly what a server would.

use bota_core::{MAP2_ID, map_of, rules};
use bota_proto::{EntityId, EventKind, Order, OutgoingOrder, ServerMsg, Target, Team};

use crate::game::{MatchConfig, OrderDelay};
use crate::local::LocalMatch;

use super::support::{config, fiend_against_fiend};

pub(super) fn mid_config() -> MatchConfig {
    MatchConfig {
        starting_gold_lost: Vec::new(),
        match_id: 5,
        stream: 5,
        master_key: [9; 32],
        picks: fiend_against_fiend(),
        map: MAP2_ID,
        ack_timeout_ticks: 150,
        ..config()
    }
}

/// The non-hero deaths a seat is told the other side's hero struck last,
/// with the gold it is told each paid, across one step's messages.
fn enemy_kills_told(stream: &[ServerMsg]) -> Vec<i32> {
    let mut told = Vec::new();
    let mut heroes: Vec<(EntityId, Team)> = Vec::new();
    let mut viewer = None;
    for msg in stream {
        match msg {
            ServerMsg::Snapshot { view } => {
                viewer = view.viewer;
                heroes = view
                    .players
                    .iter()
                    .filter_map(|player| Some((player.unit?, player.team)))
                    .collect();
            }
            ServerMsg::Events { events, .. } => {
                for kind in events {
                    let EventKind::Died {
                        unit,
                        killer: Some(killer),
                        gold,
                        ..
                    } = kind
                    else {
                        continue;
                    };
                    let team_of = |id| heroes.iter().find(|(hero, _)| *hero == id).map(|h| h.1);
                    let by_enemy = team_of(*killer).is_some_and(|team| Some(team) != viewer);
                    if by_enemy && team_of(*unit).is_none() {
                        told.push(*gold);
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
    for _ in 0..rules::FIRST_WAVE_TICK + 4000 {
        for stream in &streams {
            for gold in enemy_kills_told(stream) {
                assert_eq!(gold, 0, "a side was told the other's creep gold");
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
