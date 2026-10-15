//! An in-process match tells each seat exactly what a server would.

use bota_core::{MAP2_ID, map_of, rules};
use bota_proto::{
    EntityId, EventKind, HeroId, MapId, Order, Pick, ServerMsg, SlotId, Target, Team, TickMode,
    UnitKind,
};

use crate::game::{MatchConfig, OrderDelay};
use crate::local::{LocalMatch, LocalOrder};

fn mid_config() -> MatchConfig {
    let pick = |slot, team| Pick {
        slot: SlotId(slot),
        team,
        hero: HeroId(1),
    };
    MatchConfig {
        match_id: 5,
        master_key: [9; 32],
        picks: vec![pick(0, Team::Radiant), pick(1, Team::Dire)],
        map: MapId(MAP2_ID.0),
        tick_rate: 30,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 150,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
    }
}

/// The gold each seat was told for one death, when it was told of it.
fn told_gold(streams: &[Vec<ServerMsg>], unit: EntityId) -> [Option<i32>; 2] {
    let gold = |stream: &Vec<ServerMsg>| {
        stream.iter().find_map(|msg| match msg {
            ServerMsg::Events { events, .. } => events.iter().find_map(|kind| match kind {
                EventKind::Died {
                    unit: dead, gold, ..
                } if *dead == unit => Some(*gold),
                _ => None,
            }),
            _ => None,
        })
    };
    [gold(&streams[0]), gold(&streams[1])]
}

#[test]
fn a_non_hero_kills_gold_is_hidden_from_the_other_side() {
    let cfg = mid_config();
    let (mut local, mut streams) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    let map = map_of(cfg.map);
    let mut kinds: Vec<(EntityId, UnitKind)> = Vec::new();
    let mut hidden = 0;
    for _ in 0..rules::FIRST_WAVE_TICK + 4000 {
        for stream in &streams {
            for msg in stream {
                if let ServerMsg::Snapshot { view } = msg {
                    kinds.extend(view.units.iter().map(|u| (u.id, u.kind)));
                }
            }
        }
        for msg in streams.iter().flatten() {
            let ServerMsg::Events { events, .. } = msg else {
                continue;
            };
            for kind in events {
                let EventKind::Died {
                    unit,
                    killer: Some(killer),
                    ..
                } = kind
                else {
                    continue;
                };
                let kind_of = |id| kinds.iter().rev().find(|(i, _)| *i == id).map(|k| k.1);
                if kind_of(*unit) == Some(UnitKind::Hero)
                    || kind_of(*killer) != Some(UnitKind::Hero)
                {
                    continue;
                }
                if let [Some(a), Some(b)] = told_gold(&streams, *unit) {
                    assert!(a == 0 || b == 0, "both sides were told {a} and {b}");
                    hidden += usize::from(a != b);
                }
            }
        }
        let tick = local.tick();
        let order = |slot: usize| LocalOrder {
            seq: tick,
            unit: None,
            order: Order::Attack {
                target: Target::Pos(map.fountains[1 - slot]),
            },
            seen: tick,
        };
        streams = local
            .step(&[Some(order(0)), Some(order(1))])
            .expect("a step");
        if streams[0]
            .iter()
            .any(|m| matches!(m, ServerMsg::MatchOver { .. }))
        {
            break;
        }
    }
    assert!(hidden > 0, "no creep kill was seen by both sides");
}
