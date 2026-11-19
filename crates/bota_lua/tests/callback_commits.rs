mod support;
use bota_client_core::IssuedOrder;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, ItemId, Order, SlotId, Target, Team, Vec2};
fn host() -> LuaTeam {
    LuaTeam::new_measured(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
        true,
    )
    .unwrap()
}
fn movement(x: i32) -> IssuedOrder {
    IssuedOrder::hero(Order::Move {
        target: Target::Pos(Vec2::from_ints(9216 + x, 9216)),
    })
}
#[test]
fn callback_errors_keep_issued_orders_and_do_not_skip_later_callbacks() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = host();
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "main",
        b"function Think() GetBot():Action_MoveToLocation(Vector(1,0,0)) end",
    )
    .unwrap();
    lua.add_script(SlotId(0),ScriptKind::AbilityItemUsage,"usage",b"function AbilityUsageThink() GetBot():Action_MoveToLocation(Vector(2,0,0)); error('ability callback failed') end function ItemUsageThink() GetBot():Action_MoveToLocation(Vector(3,0,0)) end").unwrap();
    lua.add_script(SlotId(0),ScriptKind::ItemPurchase,"purchase",b"function ItemPurchaseThink() assert(GetBot():ActionImmediate_PurchaseItem('item_branches')==PURCHASE_ITEM_SUCCESS); error('purchase callback failed') end").unwrap();
    let turn = lua.think(&sessions[0]).unwrap();
    for expected in ["ability callback failed", "purchase callback failed"] {
        assert!(
            turn.failures
                .iter()
                .any(|failure| failure.error.to_string().contains(expected)),
            "{expected}: {:?}",
            turn.failures
        );
    }
    assert_eq!(
        turn.bots[0].orders,
        vec![
            movement(1),
            movement(2),
            movement(3),
            IssuedOrder::hero(Order::Buy {
                item: ItemId::IRON_BRANCH
            })
        ]
    );
    let outgoing: Vec<_> = turn.bots[0]
        .orders
        .iter()
        .map(|&order| sessions[0].issue(order).unwrap())
        .collect();
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&outgoing, &[]]).unwrap())
    {
        support::fold(session, stream);
    }
    assert!(
        sessions[0]
            .own_hero()
            .unwrap()
            .items
            .iter()
            .flatten()
            .any(|item| item.id == ItemId::IRON_BRANCH)
    );
}
#[test]
fn a_team_callback_error_does_not_cancel_the_heros_think() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = host();
    lua.add_script(SlotId(0),ScriptKind::TeamDesires,"team",b"function TeamThink() GetBot():Action_MoveToLocation(Vector(1,0,0)); error('team callback failed') end").unwrap();
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "main",
        b"function Think() GetBot():Action_MoveToLocation(Vector(2,0,0)) end",
    )
    .unwrap();
    let turn = lua.think(&sessions[0]).unwrap();
    assert!(
        turn.failures
            .iter()
            .any(|failure| failure.error.to_string().contains("team callback failed"))
    );
    assert_eq!(turn.bots[0].orders, vec![movement(1), movement(2)]);
}


#[test]
fn callback_error_summaries_are_bounded_without_losing_the_failure_count() {
    let teams = [
        Team::Radiant,
        Team::Radiant,
        Team::Radiant,
        Team::Radiant,
        Team::Radiant,
        Team::Dire,
    ];
    let (_, sessions) = support::local(&teams);
    let seats: Vec<_> = (0..5)
        .map(|slot| BotSeat {
            slot: SlotId(slot),
            hero: HeroId::SHADOW_FIEND,
        })
        .collect();
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &seats,
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .unwrap();
    for slot in 0..5 {
        for mode in 1..=15 {
            lua.add_script(
                SlotId(slot),
                ScriptKind::Mode(mode),
                "failing_mode",
                b"function GetDesire() error('mode failed') end",
            )
            .unwrap();
        }
    }
    let turn = lua.think(&sessions[0]).unwrap();
    assert_eq!(
        turn.failures.len(),
        64,
        "bounded retained callback diagnostics"
    );
    assert!(
        turn.failures.len() as u64 + turn.hook_activity.suppressed_callback_errors >= 75,
        "each attempted mode failure must be counted"
    );
    assert_eq!(turn.bots.len(), 5, "every configured seat still executes");
}



#[test]
fn a_retained_callback_error_does_not_keep_the_scripts_large_string_allocation() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = host();
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "large_error",
        b"function Think() error(string.rep('x',65536)) end",
    )
    .unwrap();
    lua.add_script(
        SlotId(0),
        ScriptKind::AbilityItemUsage,
        "after_error",
        b"function ItemUsageThink() GetBot():Action_MoveToLocation(Vector(1,0,0)) end",
    )
    .unwrap();
    let turn = lua.think(&sessions[0]).unwrap();
    assert_eq!(turn.failures.len(), 1);
    let luagee::Error::Runtime { message, .. } = &turn.failures[0].error else {
        panic!("runtime failure expected");
    };
    assert!(message.len() <= 2048);
    assert!(
        message.capacity() <= 2048,
        "retained string storage must be bounded, got {}",
        message.capacity()
    );
    assert_eq!(turn.bots[0].orders, vec![movement(1)]);
}
