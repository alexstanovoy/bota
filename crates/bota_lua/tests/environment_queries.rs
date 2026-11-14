mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};
fn check(source: &str) {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for (index, session) in sessions.iter().enumerate() {
        let mut lua = LuaTeam::new(
            session.team(),
            &[BotSeat {
                slot: SlotId(index as u8),
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
            SlotId(index as u8),
            ScriptKind::Takeover,
            "environment",
            source.as_bytes(),
        )
        .unwrap();
        let turn = lua.think(session).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    }
}
#[test]
fn shop_locations_use_public_map_geometry_and_zero_for_missing_shops() {
    check(
        "function Think() local a=GetShopLocation(GetTeam(),SHOP_HOME); assert(a.x==a.x and a.y==a.y); local b=GetShopLocation(GetTeam(),SHOP_SIDE); assert(b.x==0 and b.y==0 and b.z==0); assert(SHOP_HOME==0 and SHOP_SIDE==1 and SHOP_SECRET==2); assert(not pcall(GetShopLocation,0/0,SHOP_HOME)); end",
    );
}
#[test]
fn absent_side_shop_distance_keeps_the_installed_integer_sentinel() {
    check(
        "function Think() local b=GetBot(); assert(b:DistanceFromSideShop()==-2147483648); assert(not pcall(b.DistanceFromSideShop,{})); end",
    );
}
#[test]
fn courier_count_is_only_for_the_own_team() {
    check("function Think() assert(GetNumCouriers()==1); end");
}
#[test]
fn courier_index_is_zero_based_own_team_and_out_of_range_is_nil() {
    check(
        "function Think() local c=GetCourier(0); assert(c and c:GetTeam()==GetTeam()); assert(GetCourier(-1)==nil and GetCourier(1)==nil); assert(c==GetCourier(0)); assert(not pcall(GetCourier,0/0)); end",
    );
}
#[test]
fn game_state_uses_the_public_game_clock() {
    check(
        "function Think() assert(GAME_STATE_PRE_GAME==4 and GAME_STATE_GAME_IN_PROGRESS==5 and GAME_STATE_POST_GAME==6); assert(GetGameState()==GAME_STATE_PRE_GAME); end",
    );
}
#[test]
fn no_current_item_is_sold_from_a_side_shop() {
    check(
        "function Think() assert(not IsItemPurchasedFromSideShop('item_boots')); assert(not IsItemPurchasedFromSideShop('unknown')); assert(not pcall(IsItemPurchasedFromSideShop,{})); end",
    );
}

#[test]
fn game_state_changes_on_startgame_without_waiting_for_the_horn() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, Order};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let command = sessions[0]
        .issue(IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::StartGame,
        }))
        .unwrap();
    let streams = local.step(&[&[command], &[]]).unwrap();
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
    for (index, session) in sessions.iter().enumerate() {
        let mut lua = LuaTeam::new(
            session.team(),
            &[BotSeat {
                slot: SlotId(index as u8),
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
            SlotId(index as u8),
            ScriptKind::Takeover,
            "started",
            b"function Think() assert(GetGameState()==GAME_STATE_GAME_IN_PROGRESS) end",
        )
        .unwrap();
        assert!(lua.think(session).unwrap().failures.is_empty());
    }
}

#[test]
fn hidden_enemy_inventory_changes_do_not_change_query_answers() {
    use bota_client_core::{ClientSession, IssuedOrder};
    use bota_proto::{Cheat, Order};
    let (base, sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let mut a = base.clone();
    let mut b = base;
    let mut sa = sessions.clone();
    let mut sb = sessions;
    let item = bota_core::item_by_key("item_gloves").expect("registered gloves");
    let command = sb[1]
        .issue(IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Item { item },
        }))
        .unwrap();
    for (session, stream) in sa.iter_mut().zip(a.step(&[&[], &[]]).unwrap()) {
        support::fold(session, stream);
    }
    for (session, stream) in sb.iter_mut().zip(b.step(&[&[], &[command]]).unwrap()) {
        support::fold(session, stream);
    }
    assert_ne!(
        sa[1].own_hero().unwrap().attack_speed,
        sb[1].own_hero().unwrap().attack_speed,
        "the hidden property really changed"
    );
    assert!(
        !sa[0]
            .current()
            .unwrap()
            .units
            .iter()
            .any(|unit| unit.kind == bota_proto::UnitKind::Hero && unit.team == Team::Dire),
        "the modified enemy must be hidden"
    );
    fn query(session: &ClientSession) -> Vec<IssuedOrder> {
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
        lua.add_script(SlotId(0),ScriptKind::Takeover,"fog",br#"function Think()
    local bot=GetBot(); local p=GetShopLocation(GetTeam(),SHOP_HOME)
    assert(GetCourier(0):GetTeam()==GetTeam()); assert(GetCourier(99)==nil)
    assert(GetGameState()==GAME_STATE_PRE_GAME); assert(bot:NumQueuedActions()==0)
    assert(bot:DistanceFromSideShop()==-2147483648); assert(not IsItemPurchasedFromSideShop('item_boots'))
    local front=GetLaneFrontAmount(GetOpposingTeam(),LANE_MID,true)
    local trees=bot:GetNearbyTrees(256)
    bot:Action_MoveToLocation(Vector(p.x+GetNumCouriers()+#trees,p.y+front,0))
  end"#).unwrap();
        let turn = lua.think(session).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        turn.bots[0].orders.clone()
    }
    assert_eq!(query(&sa[0]), query(&sb[0]));
}
