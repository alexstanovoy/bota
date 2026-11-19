mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, ItemSlot, Order, SlotId, Team};
#[test]
fn swapping_two_slots_issues_an_ordinary_inventory_order() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for (at, session) in sessions.iter().enumerate() {
        let slot = SlotId(at as u8);
        let mut lua = LuaTeam::new(
            session.team(),
            &[BotSeat {
                slot,
                hero: HeroId::SHADOW_FIEND,
            }],
            Limits::DEFAULT,
            Keys {
                hash: [17, 19],
                random: 23,
            },
        )
        .unwrap();
        lua.add_script(
            slot,
            ScriptKind::Takeover,
            "swap",
            b"function Think() GetBot():ActionImmediate_SwapItems(0,6) end",
        )
        .unwrap();
        let turn = lua.think(session).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(turn.bots[0].orders.len(), 1);
        assert_eq!(
            turn.bots[0].orders[0].order,
            Order::Swap {
                from: ItemSlot(0),
                to: ItemSlot(6)
            }
        );
    }
}
#[test]
fn a_finite_path_has_an_interrupting_first_move_and_a_queued_tail() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let session = &sessions[0];
    let mut lua = LuaTeam::new(
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
    )
    .unwrap();
    lua.add_script(SlotId(0),ScriptKind::Takeover,"path",b"function Think() GetBot():Action_MovePath({Vector(1,2,0),Vector(3,4,0)}); GetBot():ActionQueue_MoveToLocation(Vector(5,6,0)); assert(GetBot():NumQueuedActions()==3); assert(GetBot():GetQueuedActionType(0)==BOT_ACTION_TYPE_MOVE_TO); assert(GetBot():GetQueuedActionType(2)==BOT_ACTION_TYPE_MOVE_TO); assert(GetBot():GetQueuedActionType(3)==-1) end").unwrap();
    let turn = lua.think(session).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    let orders = &turn.bots[0].orders;
    assert_eq!(
        orders.len(),
        1,
        "only the controller head is sent to the server"
    );
    assert_eq!(
        orders[0].order,
        Order::Move {
            target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(9217, 9218))
        }
    );
    assert!(
        !orders[0].queue,
        "waiting waypoints are client state, not server Shift orders"
    );
}
#[test]
fn oversized_paths_fail_without_publishing_a_partial_path() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new(
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
    )
    .unwrap();
    lua.add_script(SlotId(0),ScriptKind::Takeover,"path_limit",b"function Think() local p={} for i=1,100 do p[i]=Vector(i,1,0) end GetBot():Action_MovePath(p) end").unwrap();
    let turn = lua.think(&sessions[0]).unwrap();
    assert_eq!(turn.failures.len(), 1);
    assert!(format!("{:?}", turn.failures).contains("movement path exceeds the waypoint limit"));
    assert!(turn.bots.iter().all(|bot| bot.orders.is_empty()));
}
#[test]
fn a_tree_cast_maps_the_host_owned_tree_id_to_an_ordinary_item_target() {
    use bota_proto::{Cheat, ItemId, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    local.world_mut().cheat(
        SlotId(0),
        None,
        Cheat::Item {
            item: ItemId::TANGO,
        },
        &mut Vec::new(),
    );
    for (session, stream) in sessions.iter_mut().zip(local.step(&[&[], &[]]).unwrap()) {
        support::fold(session, stream);
    }
    let mut lua = LuaTeam::new(
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
    )
    .unwrap();
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "tree_cast",
        b"function Think() GetBot():Action_UseAbilityOnTree(GetBot():GetItemInSlot(0),0) end",
    )
    .unwrap();
    let turn = lua.think(&sessions[0]).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(
        turn.bots[0].orders[0].order,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::Pos(sessions[0].statics().trees()[0])
        }
    );
}
#[test]
fn catching_a_path_order_limit_error_cannot_publish_its_partial_prefix() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new(
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
    )
    .unwrap();
    lua.add_script(SlotId(0),ScriptKind::Takeover,"path_capacity",b"function Think() local bot=GetBot(); for i=1,64 do bot:ActionImmediate_SwapItems(0,1) end local ok,why=pcall(bot.Action_MovePath,bot,{Vector(1,2,0),Vector(3,4,0)}); assert(not ok and string.find(tostring(why),'order limit exceeded',1,true)) end").unwrap();
    let turn = lua.think(&sessions[0]).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 64);
    assert!(
        turn.bots[0]
            .orders
            .iter()
            .all(|order| matches!(order.order, Order::Swap { .. }))
    );
}
