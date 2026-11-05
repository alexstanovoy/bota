//! A seat's LeaveGame that reaches the server behind its last readiness, both still queued when
//! the lobby closes, is heard in the lobby: the match is cancelled, not forfeited by a seat that
//! never played.

use std::sync::Arc;
use std::sync::atomic::Ordering::SeqCst;

use super::*;
use crate::game_loop::{Hold, TestHooks, run_hooked};

#[test]
fn a_leave_queued_behind_the_last_readiness_cancels_the_match_instead_of_forfeiting_it() {
    let hold = Arc::new(Hold::default());
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (done, ended) = mpsc::channel();
    let hooks = TestHooks {
        hold: Some(Arc::clone(&hold)),
        ..Default::default()
    };
    thread::spawn(move || {
        let _ = done.send(run_hooked(listener, realtime(11), hooks));
    });
    let (_ready, _) = join_as_bot(addr, "ready");
    let mut leaver = TestClient::connect(addr);
    leaver.send(&ClientMsg::hello(
        Role::Bot,
        "leaves",
        None,
        bota_core::WIRE_FINGERPRINT,
    ));
    leaver.recv_until(|msg| matches!(msg, ServerMsg::Welcome { .. }).then_some(()));
    leaver.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    // The lobby stops reading; the last readiness and the leave behind it are queued, the
    // probe's echo saying the server read both.
    hold.held.store(true, SeqCst);
    let started = Instant::now();
    while !hold.holding.load(SeqCst) {
        assert!(started.elapsed() < HANG_GUARD, "the lobby never held");
        thread::sleep(Duration::from_millis(1));
    }
    leaver.send(&ClientMsg::SetReady(true));
    leaver.send(&ClientMsg::LeaveGame);
    leaver.send(&ClientMsg::Probe { id: 7 });
    leaver.recv_until(|msg| matches!(msg, ServerMsg::Echo { id: 7 }).then_some(()));
    hold.held.store(false, SeqCst);
    let end = ended
        .recv_timeout(HANG_GUARD)
        .expect("the server ended")
        .expect("it ran");
    assert_eq!(end, MatchEnd::Abandoned, "cancelled, not forfeited");
}
