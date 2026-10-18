//! A match fed every order hostile seats may give, through the server's own
//! path: validation on receipt, the delay line, validation again on release,
//! the tick, and each side's view encoded as it is sent. The input is replay
//! order records, so a recorded match seeds it; the first byte sets the map,
//! cheats and the order delay.
#![no_main]

use bota_proto::{
    HeroId, MapId, Pick, ReplayRecord, ServerMsg, SlotId, Team, TickMode, decode_payload,
    encode_frame_to_vec,
};
use bota_server::game::{DelayLine, MatchConfig, OrderDelay, World};
use std::sync::{Mutex, OnceLock};

use libfuzzer_sys::fuzz_target;

/// Ticks one record may move the match on: an input stays quick.
const MAX_TICKS_PER_RECORD: u32 = 8;
/// Ticks one input may run at most.
const MAX_TICKS: u32 = 300;

fn config(knobs: u8) -> MatchConfig {
    let pick = |slot: u8, team, hero| Pick {
        slot: SlotId(slot),
        team,
        hero,
    };
    let second = if knobs & 4 == 0 {
        HeroId::SHADOW_FIEND
    } else {
        HeroId::PUDGE
    };
    MatchConfig {
        match_id: 1,
        master_key: [7; 32],
        stream: 0,
        picks: vec![
            pick(0, Team::Radiant, HeroId::SHADOW_FIEND),
            pick(1, Team::Dire, second),
        ],
        map: MapId(if knobs & 1 == 0 { 2 } else { 0 }),
        tick_rate: 30,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 1,
        cheats: knobs & 2 != 0,
        spawn_modifiers: Vec::new(),
        starting_gold_lost: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
    }
}

/// The match at tick zero for the knobs that shape it, built once: building
/// takes far longer than an input runs.
fn fresh(cfg: &MatchConfig, knobs: u8) -> World {
    static WORLDS: OnceLock<Mutex<Vec<Option<World>>>> = OnceLock::new();
    let worlds = WORLDS.get_or_init(|| Mutex::new(vec![None; 8]));
    let mut worlds = worlds.lock().expect("one fuzzing thread");
    worlds[usize::from(knobs & 7)]
        .get_or_insert_with(|| World::for_match(cfg, cfg.rng()))
        .clone()
}

fn publish(world: &World, line: &DelayLine) {
    for team in [Team::Radiant, Team::Dire] {
        let mut view = world.view(team);
        view.order_delays = line.views(Some(team));
        encode_frame_to_vec(&ServerMsg::Snapshot { view }).expect("a view encodes");
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&knobs, rest)) = data.split_first() else {
        return;
    };
    let cfg = config(knobs & 7);
    let mut world = fresh(&cfg, knobs);
    let delay = OrderDelay {
        base: u32::from(knobs >> 6),
        jitter: u32::from((knobs >> 3) & 1),
    };
    let Ok(mut line) = DelayLine::new(&cfg, &[delay]) else {
        return;
    };
    let mut seq = 0u32;
    let mut refused = Vec::new();
    for (at, frame) in bota_fuzz::frames(rest).enumerate() {
        let Ok(ReplayRecord::Orders { tick, orders }) = decode_payload::<ReplayRecord>(frame)
        else {
            continue;
        };
        for given in orders {
            seq = seq.wrapping_add(1);
            let seen = Some(tick.min(world.tick));
            let _ = line.give(&world, given.slot, seq, given.unit, given.order, seen);
        }
        let ticks = tick
            .saturating_sub(world.tick)
            .clamp(1, MAX_TICKS_PER_RECORD);
        for _ in 0..ticks {
            let taken = line.release(&world, &mut refused);
            refused.clear();
            let _ = world.advance(&taken);
            if world.victor().is_some() || world.tick >= MAX_TICKS {
                publish(&world, &line);
                return;
            }
        }
        if at % 4 == 0 {
            publish(&world, &line);
        }
    }
    publish(&world, &line);
});
