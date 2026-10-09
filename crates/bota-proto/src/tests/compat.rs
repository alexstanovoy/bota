//! What a client built against an older wire still reads.

use serde::Deserialize;

use super::fixtures::*;
use crate::*;

/// [`WorldView`] as it stood before [`WorldView::aggro`].
#[derive(Deserialize, Debug, PartialEq)]
struct ViewBeforeAggro {
    tick: u32,
    viewer: Option<Team>,
    units: Vec<UnitView>,
    projectiles: Vec<ProjectileView>,
    players: Vec<PlayerView>,
    felled_trees: Vec<u32>,
    planted_trees: Vec<Vec2>,
    loot: Vec<LootView>,
}

/// [`ServerMsg`] as such a client declares it, down to the snapshot.
#[derive(Deserialize, Debug, PartialEq)]
enum MsgBeforeAggro {
    Welcome,
    LobbyState,
    MatchStart,
    Snapshot { view: ViewBeforeAggro },
}

#[test]
fn a_client_built_before_aggro_reads_a_snapshot_carrying_it() {
    let view = world_view(24);
    assert!(!view.aggro.is_empty());
    let frame = encode_frame_to_vec(&ServerMsg::Snapshot { view: view.clone() }).expect("encode");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    let old = reader
        .next_message::<MsgBeforeAggro>()
        .expect("decode")
        .expect("one whole frame was pushed");
    assert_eq!(reader.buffered(), 0, "the whole frame was consumed");
    assert_eq!(
        old,
        MsgBeforeAggro::Snapshot {
            view: ViewBeforeAggro {
                tick: view.tick,
                viewer: view.viewer,
                units: view.units,
                projectiles: view.projectiles,
                players: view.players,
                felled_trees: view.felled_trees,
                planted_trees: view.planted_trees,
                loot: view.loot,
            },
        }
    );
}
