mod support;

use bota_client_core::IssuedOrder;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, Order, SlotId, Target, Team};

fn team(slots: &[u8], limits: Limits) -> LuaTeam {
    let seats: Vec<BotSeat> = slots
        .iter()
        .map(|&slot| BotSeat {
            slot: SlotId(slot),
            hero: HeroId::SHADOW_FIEND,
        })
        .collect();
    LuaTeam::new(
        Team::Radiant,
        &seats,
        limits,
        Keys {
            hash: [1, 2],
            random: 3,
        },
    )
    .expect("team")
}

#[test]
fn lua_shadow_fiend_plays_a_complete_match_through_real_orders() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(&[0], Limits::DEFAULT);
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "bot_nevermore",
        include_bytes!("scripts/duel.lua"),
    )
    .expect("script");
    let mut sent = 0;
    let mut winner = None;
    for _ in 0..bota_core::MAP2_TICK_CAP {
        let turn = lua.think(&sessions[0]).expect("tick");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        let orders: Vec<_> = turn.bots[0]
            .orders
            .iter()
            .map(|&order| sessions[0].issue(order).expect("Lua order"))
            .collect();
        sent += orders.len();
        let middle = bota_proto::Vec2::from_ints(9216, 9216);
        let opponent = sessions[1]
            .issue(IssuedOrder::hero(Order::Attack {
                target: Target::Pos(middle),
            }))
            .expect("opponent order");
        let streams = local.step(&[&orders, &[opponent]]).expect("real step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            winner = support::fold(session, stream).or(winner);
        }
        if winner.is_some() {
            break;
        }
    }
    assert!(sent > 0, "the Lua seat issued ordinary orders");
    assert!(winner.is_some(), "the match reached MatchOver");
}

#[test]
fn script_failure_discards_only_its_own_orders_and_scopes_are_isolated() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire, Team::Radiant]);
    let mut lua = team(&[0, 2], Limits::DEFAULT);
    lua.add_script(SlotId(0), ScriptKind::Takeover, "first", b"own = 11 function Think() assert(own == 11) GetBot():Action_MoveToLocation(Vector(1,2,0)) error('failed think') end").expect("script");
    lua.add_script(SlotId(2), ScriptKind::Takeover, "second", b"own = 22 function Think() assert(own == 22) GetBot():Action_MoveToLocation(Vector(3,4,0)) end").expect("script");
    let turn = lua.think(&sessions[0]).expect("tick");
    assert!(turn.bots[0].orders.is_empty());
    assert_eq!(turn.bots[1].orders.len(), 1);
    assert_eq!(turn.failures.len(), 1);
    assert!(turn.failures[0].error.to_string().contains("failed think"));
}

#[test]
fn fuel_exhaustion_discards_orders_and_counts_without_stalling_other_seats() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire, Team::Radiant]);
    let mut lua = team(
        &[0, 2],
        Limits {
            fuel_per_call: 500,
            fuel_per_tick: 10_000,
            ..Limits::DEFAULT
        },
    );
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "spin",
        b"function Think() GetBot():Action_MoveToLocation(Vector(1,2,0)) while true do end end",
    )
    .expect("script");
    lua.add_script(
        SlotId(2),
        ScriptKind::Takeover,
        "other",
        b"function Think() GetBot():Action_MoveToLocation(Vector(3,4,0)) end",
    )
    .expect("script");
    let turn = lua.think(&sessions[0]).expect("tick");
    assert!(turn.bots[0].orders.is_empty());
    assert_eq!(turn.bots[1].orders.len(), 1);
    assert_eq!(turn.failures[0].fuel_exhaustions, 1);
}
