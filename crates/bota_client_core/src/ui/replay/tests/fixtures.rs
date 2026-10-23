use std::cell::Cell;
use std::io::{self, Cursor, Read};
use std::rc::Rc;

use bota_proto::{
    HeroId, MapId, MatchInfo, Pick, PlayerView, ReplayHeader, ReplayRecord, ServerMsg, SlotId,
    Team, TickMode, Vec2, WorldView, encode_frame_to_vec,
};

pub fn match_start(tick_rate: u16) -> ReplayRecord {
    ReplayRecord::Msg(ServerMsg::MatchStart {
        info: MatchInfo {
            match_id: 1,
            map: MapId(0),
            tick_rate,
            pregame_ticks: 900,
            trees: Vec::new(),
            terrain_cells: 1,
            terrain_rle: vec![(1, 0x80)],
            opaque_cells: Vec::new(),
            mode: TickMode::Lockstep,
            picks: vec![Pick {
                slot: SlotId(0),
                team: Team::Radiant,
                hero: HeroId::PUDGE,
            }],
            shop: Vec::new(),
            fountains: [Vec2::ZERO, Vec2::ZERO],
            shop_range: 0,
        },
    })
}

pub fn snapshot(tick: u32) -> ReplayRecord {
    ReplayRecord::Msg(ServerMsg::Snapshot {
        view: WorldView {
            tick,
            viewer: None,
            units: Vec::new(),
            players: vec![PlayerView {
                slot: SlotId(0),
                team: Team::Radiant,
                hero: HeroId::PUDGE,
                unit: None,
                level: 1,
                xp: 0,
                gold: None,
                stash: None,
                kit: None,
                kills: 0,
                deaths: 0,
                assists: 0,
                last_hits: 0,
                denies: 0,
                respawn_left: 0,
            }],
            projectiles: Vec::new(),
            loot: Vec::new(),
            aggro: Vec::new(),
            order_delays: Vec::new(),
            skipped_ticks: Vec::new(),
            stock: Vec::new(),
            talents: Vec::new(),
            teleports: Vec::new(),
            glyphs: Vec::new(),
            felled_trees: Vec::new(),
            planted_trees: Vec::new(),
        },
    })
}

/// The records framed as they are, with no header in front.
pub fn frames(records: &[ReplayRecord]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| encode_frame_to_vec(record).unwrap())
        .collect()
}

/// This build's header, framed: what a replay file starts with.
pub fn header() -> Vec<u8> {
    frames(&[ReplayRecord::Header(ReplayHeader::CURRENT)])
}

/// A replay file of the records: this build's header, then the records.
pub fn wire(records: &[ReplayRecord]) -> Vec<u8> {
    let mut bytes = header();
    bytes.extend(frames(records));
    bytes
}

pub struct CountedRead {
    pub source: Cursor<Vec<u8>>,
    pub bytes: Rc<Cell<usize>>,
    pub calls: Rc<Cell<usize>>,
    pub limit: usize,
    pub fragment: usize,
}

impl Read for CountedRead {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        self.calls.set(self.calls.get() + 1);
        let left = self.limit.saturating_sub(self.bytes.get());
        if left == 0 {
            return Err(io::Error::other("read crossed the permitted replay prefix"));
        }
        let count = target.len().min(left).min(self.fragment);
        let count = self.source.read(&mut target[..count])?;
        self.bytes.set(self.bytes.get() + count);
        Ok(count)
    }
}

pub fn counted(
    source: Vec<u8>,
    fragment: usize,
) -> (CountedRead, Rc<Cell<usize>>, Rc<Cell<usize>>) {
    let bytes = Rc::new(Cell::new(0));
    let calls = Rc::new(Cell::new(0));
    let reader = CountedRead {
        source: Cursor::new(source),
        bytes: bytes.clone(),
        calls: calls.clone(),
        limit: usize::MAX,
        fragment,
    };
    (reader, bytes, calls)
}

pub fn message(record: ReplayRecord) -> ServerMsg {
    match record {
        ReplayRecord::Msg(message) => message,
        ReplayRecord::Orders { tick, orders } => ServerMsg::Orders { tick, orders },
        ReplayRecord::Header(_) => panic!("a header is no message"),
    }
}
