//! The buyback calls of the bot API against a real match: every state is reached by orders and
//! cheat orders through the server, and what a script asks for is carried out by it.

mod support;

use bota_client_core::{ClientSession, IssuedOrder};
use bota_core::DEMO_ID;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind, TeamTurn};
use bota_proto::{Cheat, HeroId, ItemId, Order, ServerMsg, SlotId, Team};
use bota_server::local::LocalMatch;

const COOLDOWN_SECONDS: f64 = 480.0;

struct Duel {
    local: LocalMatch,
    sessions: Vec<ClientSession>,
}

impl Duel {
    fn new() -> Duel {
        let (local, sessions) = support::local_with_cheats(&[Team::Radiant, Team::Dire], DEMO_ID);
        Duel { local, sessions }
    }

    /// One tick in which Radiant gives these orders; none may be turned down.
    fn step(&mut self, orders: &[Order]) {
        let given: Vec<_> = orders
            .iter()
            .map(|&order| {
                self.sessions[0]
                    .issue(IssuedOrder::hero(order))
                    .expect("an order")
            })
            .collect();
        let streams = self.local.step(&[&given, &[]]).expect("a step");
        assert!(
            !streams[0]
                .iter()
                .any(|msg| matches!(msg, ServerMsg::OrderRejected { .. })),
            "{orders:?} turned down: {:?}",
            streams[0]
        );
        for (session, stream) in self.sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
    }

    fn cheat(&mut self, cheat: Cheat) {
        self.step(&[Order::Cheat { cheat }]);
    }

    fn set_gold(&mut self, gold: i32) {
        let amount = gold - self.gold();
        self.cheat(Cheat::Gold { amount });
    }

    fn gold(&self) -> i32 {
        self.me().gold.expect("own gold")
    }

    fn me(&self) -> &bota_proto::PlayerView {
        self.sessions[0].own_player().expect("own row")
    }

    fn tick(&self) -> u32 {
        self.sessions[0].current().expect("a view").tick
    }

    /// One turn of a script as the bot of a seat, on what that seat was last sent.
    fn think(&self, seat: usize, source: &str) -> TeamTurn {
        let team = [Team::Radiant, Team::Dire][seat];
        let mut lua = LuaTeam::new(
            team,
            &[BotSeat {
                slot: SlotId(seat as u8),
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
            SlotId(seat as u8),
            ScriptKind::Takeover,
            "buyback",
            source.as_bytes(),
        )
        .expect("script");
        let turn = lua.think(&self.sessions[seat]).expect("think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        turn
    }

    /// Radiant's script runs one turn, and the server carries out what it ordered.
    fn act(&mut self, source: &str) -> Vec<Order> {
        let turn = self.think(0, source);
        let orders: Vec<Order> = turn.bots[0]
            .orders
            .iter()
            .map(|issued| issued.order)
            .collect();
        self.step(&orders);
        orders
    }
}

#[test]
fn a_bot_reads_its_buyback_cost_and_buys_its_dead_hero_back_through_the_server() {
    let mut duel = Duel::new();
    duel.set_gold(2_600);
    duel.think(
        0,
        "function Think() local b=GetBot() assert(b:GetBuybackCost()==400, b:GetBuybackCost()) \
         assert(b:GetBuybackCooldown()==0) assert(b:HasBuyback()) end",
    );
    // A standing hero's action is accepted and does nothing, as the engine's silent check.
    assert_eq!(
        duel.act("function Think() GetBot():ActionImmediate_Buyback() end"),
        []
    );
    duel.cheat(Cheat::Suicide);
    assert!(duel.me().respawn_left > 0);
    duel.set_gold(2_600);
    let given = duel.tick();
    let orders = duel.act(
        "function Think() local b=GetBot() assert(not b:IsAlive()) b:ActionImmediate_Buyback() \
         b:ActionImmediate_Buyback() end",
    );
    assert_eq!(
        orders,
        [Order::Buyback],
        "asked twice in a turn, ordered once"
    );
    assert_eq!(duel.me().respawn_left, 0, "the server bought it back");
    assert_eq!(duel.gold(), 2_200);
    let ready = f64::from(given) / 30.0 + COOLDOWN_SECONDS;
    duel.think(
        0,
        &format!(
            "function Think() local b=GetBot() assert(b:IsAlive()) \
             assert(b:GetBuybackCooldown()=={ready}, b:GetBuybackCooldown()) \
             assert(b:GetBuybackCooldown()>GameTime()) assert(not b:HasBuyback()) \
             assert(b:GetBuybackCost()==200+math.floor(2200/13)) end"
        ),
    );
    // On cooldown the action does nothing, dead or not.
    duel.cheat(Cheat::Suicide);
    assert_eq!(
        duel.act("function Think() GetBot():ActionImmediate_Buyback() end"),
        []
    );
    assert!(duel.me().respawn_left > 0);
}

#[test]
fn has_buyback_needs_gold_strictly_above_the_cost_while_the_action_takes_exactly_the_cost() {
    let mut duel = Duel::new();
    duel.cheat(Cheat::Suicide);
    // 200 + 216 / 13 = 216: the seat holds exactly the cost.
    duel.set_gold(216);
    duel.think(
        0,
        "function Think() local b=GetBot() assert(b:GetBuybackCost()==216) assert(b:GetGold()==216) \
         assert(not b:HasBuyback(), 'the engine asks for more than the cost') end",
    );
    let orders = duel.act("function Think() GetBot():ActionImmediate_Buyback() end");
    assert_eq!(orders, [Order::Buyback]);
    assert_eq!((duel.me().respawn_left, duel.gold()), (0, 0));
    let mut poor = Duel::new();
    poor.cheat(Cheat::Suicide);
    poor.set_gold(214);
    assert_eq!(
        poor.act("function Think() GetBot():ActionImmediate_Buyback() end"),
        []
    );
    assert!(poor.me().respawn_left > 0, "a coin short buys nothing");
}

#[test]
fn has_buyback_is_false_on_the_tick_the_cooldown_ends_and_true_after_it() {
    let mut duel = Duel::new();
    duel.set_gold(9_000);
    duel.cheat(Cheat::Suicide);
    duel.step(&[Order::Buyback]);
    duel.cheat(Cheat::ResetBuyback);
    // The reset set the cooldown's end to the tick it was given on: one tick ago.
    duel.think(
        0,
        "function Think() local b=GetBot() assert(b:GetBuybackCooldown()<GameTime()) \
         assert(b:HasBuyback()) end",
    );
    duel.cheat(Cheat::Suicide);
    duel.step(&[Order::Buyback]);
    let ready = duel.me().buyback.expect("own buyback").ready_at;
    while duel.tick() < ready {
        duel.step(&[]);
    }
    assert_eq!(duel.tick(), ready);
    duel.think(
        0,
        "function Think() local b=GetBot() assert(b:GetBuybackCooldown()==GameTime()) \
         assert(not b:HasBuyback(), 'strictly past, by the engine') end",
    );
    duel.step(&[]);
    duel.think(0, "function Think() assert(GetBot():HasBuyback()) end");
}

#[test]
fn a_bot_cannot_buy_back_while_its_hero_rises_by_an_aegis() {
    let mut duel = Duel::new();
    duel.set_gold(9_000);
    duel.cheat(Cheat::Item {
        item: ItemId::AEGIS,
    });
    duel.cheat(Cheat::Suicide);
    assert!(duel.me().respawn_left > 0);
    assert!(duel.me().kit.as_ref().expect("what it left").reincarnating);
    let gold = duel.gold();
    assert_eq!(
        duel.act("function Think() GetBot():ActionImmediate_Buyback() end"),
        []
    );
    assert_eq!(duel.gold(), gold);
}

#[test]
fn the_other_sides_buyback_is_not_told_to_a_script_and_its_handle_buys_nothing_back() {
    let mut duel = Duel::new();
    // Dire's bot holds Radiant's hero handle once it has seen it.
    let meet = bota_core::hero_spawn_pos(bota_core::map_of(DEMO_ID), Team::Dire);
    duel.cheat(Cheat::Suicide);
    duel.step(&[Order::Buyback]);
    duel.cheat(Cheat::Teleport { at: meet });
    duel.step(&[]);
    let turn = duel.think(
        1,
        "function Think() local seen=false \
         local foes=GetUnitList(UNIT_LIST_ENEMY_HEROES) \
         for _,h in ipairs(foes) do seen=true \
           assert(h:GetBuybackCooldown()==-1, 'no cooldown of the other side') \
           assert(h:GetBuybackCost()==-1, 'no cost of the other side') \
           assert(not h:HasBuyback()) \
           assert(not pcall(h.ActionImmediate_Buyback,h), 'only the active bot acts') end \
         assert(seen, 'the other hero was in sight') \
         local b=GetBot() assert(b:GetBuybackCooldown()==0) assert(b:GetBuybackCost()>=200) end",
    );
    assert!(turn.bots[0].orders.is_empty());
}

#[test]
fn a_forged_receiver_is_refused_by_every_buyback_call() {
    let duel = Duel::new();
    duel.think(
        0,
        "function Think() local b=GetBot() local forged={} for key,value in pairs(b) do forged[key]=value end \
         for _,name in ipairs({'GetBuybackCooldown','GetBuybackCost','HasBuyback','ActionImmediate_Buyback'}) do \
           assert(not pcall(b[name],forged), name) assert(not pcall(b[name]), name) end end",
    );
}

#[test]
fn has_buyback_counts_the_gold_left_after_the_purchases_of_the_same_turn() {
    let mut duel = Duel::new();
    let price = bota_core::item_by_key("item_branches")
        .and_then(bota_core::item_def)
        .expect("branches")
        .cost;
    // 200 + 230 / 13 = 217: enough before the purchase, short of it after.
    duel.set_gold(230);
    let left = 230 - price;
    assert!(left <= 217, "the purchase takes the gold below the cost");
    let orders = duel.act(&format!(
        "function Think() local b=GetBot() assert(b:GetBuybackCost()==217, b:GetBuybackCost()) \
         assert(b:HasBuyback(), 'enough before') \
         local bought=b:ActionImmediate_PurchaseItem('item_branches') \
         assert(bought==PURCHASE_ITEM_SUCCESS, bought) \
         assert(b:GetGold()=={left}, b:GetGold()) \
         assert(not b:HasBuyback(), 'the purchase spent the buyback') end"
    ));
    assert_eq!(orders.len(), 1, "the purchase alone");
    assert_eq!(duel.gold(), left);
}
