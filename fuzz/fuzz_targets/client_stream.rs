//! A client's picture of a match fed what a hostile server may send: the
//! first byte picks a seat or a spectator, the rest is server messages. No
//! stream panics or makes the client do unbounded work; what the session
//! answers is read the way a front end and a bot read it.
#![no_main]

use bota_client_core::{ClientSession, Seat, Seated, SessionOptions};
use bota_proto::{HeroId, ItemId, PlayerId, ServerMsg, SlotId, TickMode, decode_payload};
use libfuzzer_sys::fuzz_target;

fn read_back(session: &ClientSession) {
    let _ = session.own_hero();
    let _ = session.own_courier();
    let _ = session.order_delay();
    for track in session.entities() {
        let _ = session.talents_of(track.id);
        let _ = session.known(track.id);
    }
    for slot in 0..2 {
        let _ = session.body_of(SlotId(slot));
    }
    let statics = session.statics();
    if let Some(entry) = statics.shop().first() {
        let held: Vec<ItemId> = statics.shop().iter().take(6).map(|e| e.id).collect();
        let _ = bota_client_core::missing_for(entry.id, &held, None);
    }
    if let Some(view) = session.current() {
        for unit in &view.units {
            let _ = statics.ground_tier(unit.pos);
            let _ = session.observes_locally(unit.pos);
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let mut seat = match mode {
        0..128 => Seat::watching(SessionOptions::default()),
        _ => Seat::new(
            Seated {
                player: PlayerId(1),
                slot: SlotId(mode & 15),
                tick_rate: 30,
                mode: TickMode::Lockstep,
                hero: (mode & 16 != 0).then_some(HeroId::SHADOW_FIEND),
            },
            SessionOptions::default(),
        ),
    };
    for frame in bota_fuzz::frames(rest) {
        let Ok(msg) = decode_payload::<ServerMsg>(frame) else {
            continue;
        };
        if seat.ingest(msg, &mut ()).is_err() {
            return;
        }
        if let Some(session) = seat.session() {
            read_back(session);
        }
    }
});
