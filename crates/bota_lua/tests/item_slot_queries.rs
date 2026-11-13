mod support;

use bota_client_core::{ClientSession, IssuedOrder};
use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{
    Cheat, HeroId, ItemId, ItemSlot, Kit, Order, ServerMsg, SlotId, Target, Team, Vec2,
};
use bota_server::local::LocalMatch;

fn team(side: Team, slot: SlotId, source: &[u8]) -> LuaTeam {
    let mut lua = LuaTeam::new(
        side,
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
    .expect("team");
    lua.set_gc_stress(GcStress::Full);
    lua.add_script(slot, ScriptKind::Takeover, "item_slots", source)
        .expect("script");
    lua
}

fn step(local: &mut LocalMatch, sessions: &mut [ClientSession], index: usize, orders: &[Order]) {
    let outgoing: Vec<_> = orders
        .iter()
        .map(|&order| {
            sessions[index]
                .issue(IssuedOrder::hero(order))
                .expect("order")
        })
        .collect();
    let mut commands = [Vec::new(), Vec::new()];
    commands[index] = outgoing;
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&commands[0], &commands[1]]).expect("step"))
    {
        assert!(
            !stream
                .iter()
                .any(|message| matches!(message, ServerMsg::OrderRejected { .. })),
            "ordinary orders accepted"
        );
        support::fold(session, stream);
    }
}

fn assert_slot_move(turn: &bota_lua::TeamTurn, slot: i32) {
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(
        turn.bots[0].orders,
        vec![IssuedOrder::hero(Order::Move {
            target: Target::Pos(Vec2::from_ints(9216 + slot, 9416))
        })]
    );
}

#[test]
fn named_item_slots_follow_real_swaps_sales_and_the_owned_dead_kit() {
    let scroll = bota_core::item_by_key("item_tpscroll").expect("scroll");
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let slot = SlotId(index as u8);
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        {
            let world = local.world_mut();
            world.cheats = true;
            for item in [ItemId::IRON_BRANCH, ItemId::IRON_BRANCH, scroll] {
                world.cheat(slot, None, Cheat::Item { item }, &mut Vec::new());
            }
        }
        step(&mut local, &mut sessions, index, &[]);
        let mut lua = team(
            side,
            slot,
            br#"
            local phase=0
            function Think()
                phase=phase+1
                local bot=GetBot()
                local expected=phase==1 and 0 or (phase==2 and 6 or 9)
                assert(bot:FindItemSlot('item_branches')==expected)
                assert(bot:FindItemSlot('item_tpscroll')==15)
                assert(bot:FindItemSlot('item_not_in_catalogue')==-1)
                assert(bot:FindItemSlot('ITEM_BRANCHES')==-1)
                assert(not pcall(bot.FindItemSlot,{},'item_branches'))
                for _,bad in ipairs({false,0,0/0,math.huge,1e300,{}}) do
                    local ok,reason=pcall(bot.FindItemSlot,bot,bad)
                    assert(not ok and string.find(tostring(reason),'item name expected',1,true))
                end
                bot:Action_MoveToLocation(Vector(expected,200,0))
            end
        "#,
        );
        assert_slot_move(&lua.think(&sessions[index]).expect("first view"), 0);
        step(
            &mut local,
            &mut sessions,
            index,
            &[
                Order::Swap {
                    from: ItemSlot(0),
                    to: ItemSlot(6),
                },
                Order::Swap {
                    from: ItemSlot(1),
                    to: ItemSlot(9),
                },
            ],
        );
        assert_slot_move(&lua.think(&sessions[index]).expect("swapped view"), 6);
        step(
            &mut local,
            &mut sessions,
            index,
            &[Order::Sell { slot: ItemSlot(6) }],
        );
        assert_slot_move(&lua.think(&sessions[index]).expect("sold view"), 9);
        let mut view = sessions[index].current().expect("view").clone();
        view.tick += 481;
        let body = sessions[index].own_hero().expect("body");
        let own = body.id;
        let kit = Kit {
            abilities: body.abilities.clone(),
            items: body.items.clone(),
            town_portal: body.town_portal,
            sellable_slots: 0,
            reincarnating: false,
        };
        let player = view
            .players
            .iter_mut()
            .find(|player| player.slot == slot)
            .expect("player");
        player.kit = Some(kit);
        player.respawn_left = 600;
        view.units.retain(|unit| unit.id != own);
        view.aggro.clear();
        let tick = view.tick;
        sessions[index]
            .ingest(ServerMsg::Snapshot { view }, &mut ())
            .expect("received dead kit");
        sessions[index]
            .ingest(
                ServerMsg::Events {
                    tick,
                    events: Vec::new(),
                },
                &mut (),
            )
            .expect("events");
        assert!(sessions[index].entity(own).is_none());
        assert_slot_move(&lua.think(&sessions[index]).expect("dead kit view"), 9);
    }
}

#[test]
fn item_slot_queries_do_not_discover_a_hidden_enemy_purchase_or_use_an_expired_track() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    {
        let world = local.world_mut();
        world.cheats = true;
        world.cheat(
            SlotId(1),
            None,
            Cheat::Item {
                item: ItemId::IRON_BRANCH,
            },
            &mut Vec::new(),
        );
        world.cheat(
            SlotId(0),
            None,
            Cheat::AllVision { on: true },
            &mut Vec::new(),
        );
    }
    step(&mut local, &mut sessions, 0, &[]);
    let enemy = sessions[0]
        .current()
        .expect("view")
        .units
        .iter()
        .find(|unit| unit.hero.is_some() && unit.team == Team::Dire)
        .expect("enemy")
        .id;
    let mut lua=team(Team::Radiant,SlotId(0),br#"
        local phase,enemy=0,nil
        function Think()
            phase=phase+1
            if phase==1 then enemy=GetUnitList(UNIT_LIST_ENEMY_HEROES)[1] end
            if phase<3 then
                assert(enemy:FindItemSlot('item_branches')==0)
                assert(enemy:FindItemSlot('item_gloves')==-1)
                if phase==2 then assert(not enemy:CanBeSeen()) end
            else
                local ok,reason=pcall(enemy.FindItemSlot,enemy,'item_branches')
                assert(not ok and string.find(tostring(reason),'unit inventory is not observed',1,true))
            end
            GetBot():Action_MoveToLocation(Vector(phase,200,0))
        end
    "#);
    assert_slot_move(&lua.think(&sessions[0]).expect("seen"), 1);
    local.world_mut().cheat(
        SlotId(0),
        None,
        Cheat::AllVision { on: false },
        &mut Vec::new(),
    );
    let gloves = bota_core::item_by_key("item_gloves").expect("gloves");
    step(&mut local, &mut sessions, 1, &[Order::Buy { item: gloves }]);
    assert!(
        sessions[1]
            .own_hero()
            .expect("enemy body")
            .items
            .iter()
            .flatten()
            .any(|item| item.id == gloves)
    );
    assert_slot_move(&lua.think(&sessions[0]).expect("hidden purchase"), 2);
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 481;
    let tick = view.tick;
    sessions[0]
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("expired view");
    sessions[0]
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    assert!(sessions[0].entity(enemy).is_none());
    assert_slot_move(&lua.think(&sessions[0]).expect("expired enemy"), 3);
}
