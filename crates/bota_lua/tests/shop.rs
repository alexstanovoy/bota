mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, Order, ServerMsg, SlotId, Team};

#[test]
fn purchase_failures_report_missing_names_gold_and_received_stock_without_orders() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let gold = sessions[0]
        .own_player()
        .expect("player")
        .gold
        .expect("gold");
    let expensive = bota_core::Catalogue::FULL
        .sold_items()
        .filter_map(bota_core::item_def)
        .find(|item| item.cost > gold)
        .expect("expensive item");
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
    let stock = view
        .stock
        .iter_mut()
        .find(|stock| stock.team == Team::Radiant)
        .expect("stock");
    stock.left = 0;
    let sold_out = bota_core::item_def(stock.item).expect("stock item").key;
    let tick = view.tick;
    sessions[0]
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("snapshot");
    sessions[0]
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    let source = format!(
        "function Think() local b=GetBot() assert(b:ActionImmediate_PurchaseItem('missing_item')==7,'invalid name result') assert(b:ActionImmediate_PurchaseItem('{}')==3,'insufficient gold result') assert(b:ActionImmediate_PurchaseItem('{}')==1,'out of stock result') end",
        expensive.key, sold_out
    );
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
    .expect("team");
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "purchase_refusals",
        source.as_bytes(),
    )
    .expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(
        !turn.bots[0]
            .orders
            .iter()
            .any(|issued| matches!(issued.order, Order::Buy { .. }))
    );
}

#[test]
fn town_portal_and_stash_slots_resolve_without_becoming_active_bag_slots() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let scroll = sessions[0]
        .issue(bota_client_core::IssuedOrder::hero(Order::Buy {
            item: bota_proto::ItemId::TOWN_PORTAL_SCROLL,
        }))
        .expect("buy");
    let streams = local.step(&[&[scroll], &[]]).expect("step");
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
    for _ in 0..180 {
        let order = sessions[0]
            .issue(bota_client_core::IssuedOrder::hero(Order::Move {
                target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(9216, 9216)),
            }))
            .expect("move");
        let streams = local.step(&[&[order], &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
    }
    let item = bota_core::item_by_key("item_branches").expect("item");
    let buy = sessions[0]
        .issue(bota_client_core::IssuedOrder::hero(Order::Buy { item }))
        .expect("buy into stash");
    let streams = local.step(&[&[buy], &[]]).expect("step");
    for (session, stream) in sessions.iter_mut().zip(streams) {
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
    .expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "scroll_slot", b"function Think() local b=GetBot() local s=b:GetItemInSlot(15) assert(s~=nil and s:GetName()=='item_tpscroll') local stash=b:GetItemInSlot(9) assert(stash~=nil and stash:GetName()=='item_branches' and not stash:IsFullyCastable()) assert(b:GetItemInSlot(-1)==nil and b:GetItemInSlot(16)==nil) end").expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}

#[test]
fn purchases_in_one_think_reserve_gold_before_reporting_success() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let gold = sessions[0]
        .own_player()
        .expect("player")
        .gold
        .expect("gold");
    let item = bota_core::Catalogue::FULL
        .sold_items()
        .filter_map(bota_core::item_def)
        .find(|item| {
            item.components.is_empty()
                && !item.secret_shop
                && item.stock.is_none()
                && item.cost > gold / 2
                && item.cost <= gold
        })
        .expect("one affordable item");
    let source = format!(
        "function Think() local b=GetBot() assert(b:ActionImmediate_PurchaseItem('{}')==0) assert(b:GetGold()=={},'projected gold') assert(b:ActionImmediate_PurchaseItem('{}')==3,'second purchase must reserve gold') end",
        item.key,
        gold - item.cost,
        item.key
    );
    let turn = run_script(&sessions[0], source.as_bytes());
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
    let orders: Vec<_> = turn.bots[0]
        .orders
        .iter()
        .map(|&issued| sessions[0].issue(issued).expect("order"))
        .collect();
    let streams = local.step(&[&orders, &[]]).expect("step");
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
    assert_eq!(
        sessions[0]
            .own_hero()
            .expect("hero")
            .items
            .iter()
            .flatten()
            .filter(|held| held.id == item.id)
            .count(),
        1
    );
}

#[test]
fn purchases_in_one_think_reserve_the_last_public_stock_unit() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
    let stock = view
        .stock
        .iter_mut()
        .find(|stock| stock.team == Team::Radiant)
        .expect("stock");
    stock.left = 1;
    let item = bota_core::item_def(stock.item).expect("item").key;
    replace_view(&mut sessions[0], view);
    let source = format!(
        "function Think() local b=GetBot() assert(b:ActionImmediate_PurchaseItem('{item}')==0) assert(b:ActionImmediate_PurchaseItem('{item}')==1,'last stock is reserved') end"
    );
    let turn = run_script(&sessions[0], source.as_bytes());
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
}

#[test]
fn purchases_in_one_think_reserve_the_last_inventory_space() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let branches = bota_core::item_by_key("item_branches").expect("item");
    let buy = sessions[0]
        .issue(bota_client_core::IssuedOrder::hero(Order::Buy {
            item: branches,
        }))
        .expect("buy");
    let streams = local.step(&[&[buy], &[]]).expect("step");
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
    let unit = view
        .units
        .iter_mut()
        .find(|unit| unit.owner == Some(SlotId(0)) && unit.hero.is_some())
        .expect("hero");
    let item = unit.items[0];
    unit.items = vec![item; 9];
    unit.items[8] = None;
    view.players
        .iter_mut()
        .find(|player| player.slot == SlotId(0))
        .expect("player")
        .stash = Some(vec![item; 6]);
    replace_view(&mut sessions[0], view);
    let turn = run_script(&sessions[0], b"function Think() local b=GetBot() assert(b:ActionImmediate_PurchaseItem('item_branches')==0) assert(b:ActionImmediate_PurchaseItem('item_branches')==1,'last slot is reserved') end");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
}

#[test]
fn a_failed_bot_turn_releases_purchase_reservations_for_its_allied_bot() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire, Team::Radiant]);
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
    let stock = view
        .stock
        .iter_mut()
        .find(|stock| stock.team == Team::Radiant)
        .expect("stock");
    stock.left = 1;
    let item = bota_core::item_def(stock.item).expect("item").key;
    replace_view(&mut sessions[0], view);
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[
            BotSeat {
                slot: SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            },
            BotSeat {
                slot: SlotId(2),
                hero: HeroId::SHADOW_FIEND,
            },
        ],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    let failing = format!(
        "function Think() assert(GetBot():ActionImmediate_PurchaseItem('{item}')==0) error('cancel purchase turn') end"
    );
    let allied = format!(
        "function Think() assert(GetBot():ActionImmediate_PurchaseItem('{item}')==0,'failed turn released stock') end"
    );
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "failed_purchase",
        failing.as_bytes(),
    )
    .expect("script");
    lua.add_script(
        SlotId(2),
        ScriptKind::Takeover,
        "allied_purchase",
        allied.as_bytes(),
    )
    .expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert_eq!(turn.failures.len(), 1);
    assert!(
        turn.failures[0]
            .error
            .to_string()
            .contains("cancel purchase turn")
    );
    assert!(turn.bots[0].orders.is_empty());
    assert_eq!(turn.bots[1].orders.len(), 1);
}

fn run_script(session: &bota_client_core::ClientSession, source: &[u8]) -> bota_lua::TeamTurn {
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
    .expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "shop", source)
        .expect("script");
    lua.think(session).expect("think")
}

fn replace_view(session: &mut bota_client_core::ClientSession, view: bota_proto::WorldView) {
    let tick = view.tick;
    session
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("snapshot");
    session
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
}

fn down_shopper(
    reincarnating: bool,
) -> (
    bota_server::local::LocalMatch,
    Vec<bota_client_core::ClientSession>,
) {
    use bota_proto::{Fixed, ItemId, Vec2};
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let world = local.world_mut();
    let hero = world.seats[0].unit.expect("hero");
    world.creeps_halted = true;
    world.seats[0].gold = 10_000;
    world.seats[0].respawn_left = 90;
    world.seats[0].reincarnating_at = reincarnating.then_some(Vec2::from_ints(9216, 9216));
    assert!(world.transform.remove(hero).is_some());
    world.health.get_mut(hero).expect("health").hp = Fixed::ZERO;
    let blocker =
        bota_server::game::ItemStack::bought(ItemId::QUELLING_BLADE, SlotId(0), world.tick)
            .expect("owned item");
    let bag = world.inventory.get_mut(hero).expect("inventory");
    bag.slots.fill(Some(blocker));
    let last = bag.slots.len() - 1;
    bag.slots[last] = None;
    world.seats[0].stash.slots.fill(Some(blocker));
    let streams = local.step(&[&[], &[]]).expect("publish arranged respawn");
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
    assert!(sessions[0].own_hero().is_none());
    let player = sessions[0].own_player().expect("player");
    assert!(player.respawn_left > 0);
    let kit = player.kit.as_ref().expect("private saved kit");
    assert_eq!(kit.reincarnating, reincarnating);
    assert!(kit.items.last().expect("last carried slot").is_none());
    assert!(
        player
            .stash
            .as_ref()
            .expect("stash")
            .iter()
            .all(Option::is_some)
    );
    (local, sessions)
}

#[test]
fn normal_respawn_purchases_reserve_the_saved_kit_before_reporting_success() {
    use bota_proto::{EventKind, ItemId};
    let (mut local, mut sessions) = down_shopper(false);
    let gold = local.world().seats[0].gold;
    let price = bota_core::item_def(ItemId::BOOTS).expect("boots").cost;
    let source = format!(
        "function Think() local b=GetBot() assert(b:ActionImmediate_PurchaseItem('item_boots')==PURCHASE_ITEM_SUCCESS,'normal respawn receives purchases in its saved kit') assert(b:GetGold()=={}) assert(b:ActionImmediate_PurchaseItem('item_boots')==PURCHASE_ITEM_OUT_OF_STOCK,'the saved kit slot is reserved') assert(b:GetGold()=={}) end",
        gold - price,
        gold - price
    );
    let turn = run_script(&sessions[0], source.as_bytes());
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
    assert!(matches!(
        turn.bots[0].orders[0].order,
        Order::Buy {
            item: ItemId::BOOTS
        }
    ));
    let orders: Vec<_> = turn.bots[0]
        .orders
        .iter()
        .map(|&issued| sessions[0].issue(issued).expect("dead hero purchase order"))
        .collect();
    let streams = local.step(&[&orders, &[]]).expect("real server purchase");
    assert!(
        !streams[0]
            .iter()
            .any(|message| matches!(message, ServerMsg::OrderRejected { .. }))
    );
    assert!(streams[0].iter().any(|message| matches!(message,
        ServerMsg::Events { events, .. } if events.iter().any(|event| matches!(event,
            EventKind::ItemBought { item: ItemId::BOOTS, .. })))));
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
    let player = sessions[0].own_player().expect("player");
    assert_eq!(player.gold, Some(gold - price));
    assert_eq!(
        player
            .kit
            .as_ref()
            .expect("still respawning")
            .items
            .last()
            .and_then(Option::as_ref)
            .map(|item| item.id),
        Some(ItemId::BOOTS)
    );
    assert!(
        player
            .stash
            .as_ref()
            .expect("stash")
            .iter()
            .flatten()
            .all(|item| item.id == ItemId::QUELLING_BLADE)
    );
}

#[test]
fn reincarnation_does_not_grant_the_saved_kit_purchase_space() {
    let (_, sessions) = down_shopper(true);
    let gold = sessions[0]
        .own_player()
        .expect("player")
        .gold
        .expect("gold");
    let source = format!(
        "function Think() local b=GetBot() assert(b:ActionImmediate_PurchaseItem('item_boots')==PURCHASE_ITEM_OUT_OF_STOCK,'reincarnation still needs room in the stash') assert(b:GetGold()=={gold}) end"
    );
    let turn = run_script(&sessions[0], source.as_bytes());
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(turn.bots[0].orders.is_empty());
}

#[test]
fn respawning_without_a_shop_does_not_allow_secret_shop_purchases() {
    let (_, sessions) = down_shopper(false);
    let gold = sessions[0]
        .own_player()
        .expect("player")
        .gold
        .expect("gold");
    let source = format!(
        "function Think() local b=GetBot() assert(b:ActionImmediate_PurchaseItem('item_demon_edge')==PURCHASE_ITEM_NOT_AT_SECRET_SHOP,'the saved kit is not a physical carrier') assert(b:GetGold()=={gold}) end"
    );
    let turn = run_script(&sessions[0], source.as_bytes());
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(turn.bots[0].orders.is_empty());
}
