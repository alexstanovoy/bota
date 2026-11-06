#![cfg(all(feature = "native", not(target_arch = "wasm32")))]
mod support;
use std::collections::VecDeque;
use bota_client_core::{ClientSession, MatchStatic, SessionOptions, Transport, IssuedOrder, Seated};
use bota_proto::{ClientMsg, ServerMsg, OutgoingOrder, Team, SlotId, HeroId, PlayerId, TickMode, Order, Target, Vec2};
use bota_lua::{BotSeat, Keys, Limits, ModuleBank};
use bota_lua::native::{NativeConfig, NativeTeam, play_team};

struct LocalTransport {
    local: bota_server::local::LocalMatch, queue: VecDeque<ServerMsg>, opponent: ClientSession,
    pending: Vec<OutgoingOrder>,
}
impl Transport for LocalTransport {
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> { Ok(self.queue.pop_front()) }
    fn send(&mut self, message: &ClientMsg) -> std::io::Result<()> {
        match *message {
            ClientMsg::Order { seq, unit, order, seen: Some(seen), queue } => self.pending.push(OutgoingOrder { seq, unit, order, seen, queue }),
            ClientMsg::Ack { .. } => {
                if self.queue.iter().any(|message| matches!(message, ServerMsg::MatchOver { .. })) { return Ok(()); }
                let order = self.opponent.issue(IssuedOrder::hero(Order::Attack { target: Target::Pos(Vec2::from_ints(9216,9216)) })).expect("opponent");
                let streams = self.local.step(&[&self.pending, &[order]]).expect("step");
                self.pending.clear();
                for (index, stream) in streams.into_iter().enumerate() {
                    if index == 0 { self.queue.extend(stream); } else { support::fold(&mut self.opponent, stream); }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
#[test]
fn process_driver_folds_real_streams_and_sends_orders_and_acknowledgements_to_match_end() {
    let (local, mut streams) = support::opening(&[Team::Radiant, Team::Dire]);
    let ServerMsg::MatchStart { info } = &streams[1][0] else { panic!("start") };
    let mut opponent = ClientSession::seat(MatchStatic::new(info).expect("statics"), SlotId(1), Some(HeroId::SHADOW_FIEND), SessionOptions::default()).expect("session");
    support::fold(&mut opponent, streams[1].iter().skip(1).cloned());
    let transport = LocalTransport { local, queue: std::mem::take(&mut streams[0]).into(), opponent, pending: Vec::new() };
    let seated = Seated { player: PlayerId(1), slot: SlotId(0), tick_rate: 30, mode: TickMode::Lockstep, hero: Some(HeroId::SHADOW_FIEND) };
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    modules.insert("bots/bot_nevermore", include_bytes!("scripts/duel.lua")).expect("source");
    let mut config = NativeConfig::new(Team::Radiant, vec![BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Keys { hash:[17,19], random:23 });
    config.think_timeout = std::time::Duration::from_secs(10);
    let mut worker = NativeTeam::new(config, modules).expect("worker");
    let result = play_team(vec![(transport, seated)], &mut worker).expect("driver");
    assert_eq!(result.outcomes.len(), 1);
    assert!(result.orders_sent > 0);
    assert_eq!(result.script_failures, 0);
    assert!(!result.retired);
}
