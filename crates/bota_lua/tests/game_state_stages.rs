mod support;
use bota_client_core::IssuedOrder;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Cheat, HeroId, Order, ServerMsg, SlotId, Team};
#[test]
fn the_same_vm_observes_pregame_playing_and_postgame_at_the_final_tick() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [1, 2],
            random: 3,
        },
    )
    .unwrap();
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "stages",
        b"local n=0; function Think() n=n+1; assert(GetGameState()==({4,5,6})[n]) end",
    )
    .unwrap();
    assert!(lua.think(&sessions[0]).unwrap().failures.is_empty());
    let command = sessions[0]
        .issue(IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::StartGame,
        }))
        .unwrap();
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[command], &[]]).unwrap())
    {
        support::fold(session, stream);
    }
    assert!(lua.think(&sessions[0]).unwrap().failures.is_empty());
    local.world_mut().winner = Some(Team::Radiant);
    for (session, stream) in sessions.iter_mut().zip(local.restate()) {
        for message in stream {
            if matches!(message, ServerMsg::MatchOver { .. }) {
                session.ingest(message, &mut ()).unwrap();
            }
        }
    }
    let turn = lua.think(&sessions[0]).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(
        lua.think(&sessions[0]).is_err(),
        "a completed phase is only observed once at the same tick"
    );
}
