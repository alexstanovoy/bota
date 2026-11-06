//! Secret-shop purchases through the same delayed order and side-view path as a served match.

use bota_proto::{
    Cheat, HeroId, ItemId, MapId, Order, OutgoingOrder, Pick, RejectReason, ServerMsg, SlotId,
    Team, TickMode, Vec2, WorldView,
};
use bota_server::{
    game::{Entity, MatchConfig, OrderDelay},
    local::LocalMatch,
};

const SHOP: Vec2 = Vec2::from_ints(4260, 10950);

struct Buyer {
    game: LocalMatch,
    view: WorldView,
}

impl Buyer {
    fn new(map: MapId) -> Self {
        let def = bota_core::map_of(map);
        let cfg = MatchConfig {
            match_id: 1,
            master_key: [3; 32],
            stream: 1,
            picks: vec![Pick {
                slot: SlotId(0),
                team: Team::Radiant,
                hero: HeroId::SHADOW_FIEND,
            }],
            map,
            mode: TickMode::Lockstep,
            ack_timeout_ticks: 150,
            cheats: true,
            spawn_modifiers: Vec::new(),
            immortal_structures: true,
            immortal_heroes: true,
            starting_gold_lost: Vec::new(),
            objectives: def.default_objectives,
            runes: def.default_runes,
            catalogue: bota_core::Catalogue::FULL,
            economy_events: Vec::new(),
        };
        let (game, streams) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
        let view = snapshot(&streams[0]);
        let mut buyer = Self { game, view };
        buyer.cheat(None, Cheat::Gold { amount: 100000 });
        buyer
    }

    fn order(&mut self, unit: Option<Entity>, order: Order) -> Option<RejectReason> {
        let tick = self.game.tick();
        let unit = unit.map(|unit| self.game.handle_of(0, unit).expect("own unit known"));
        let order = OutgoingOrder {
            seq: tick,
            unit,
            order,
            seen: tick,
            queue: false,
        };
        let streams = self.game.step(&[&[order]]).expect("order step");
        self.view = snapshot(&streams[0]);
        streams[0].iter().find_map(|msg| match msg {
            ServerMsg::OrderRejected { reason, .. } => Some(*reason),
            _ => None,
        })
    }

    fn cheat(&mut self, unit: Option<Entity>, cheat: Cheat) {
        assert_eq!(self.order(unit, Order::Cheat { cheat }), None);
    }

    fn hero(&self) -> Entity {
        self.game.hero_of(0).expect("hero")
    }
    fn courier(&self) -> Entity {
        self.game.world().seats[0].courier.expect("courier")
    }

    fn count(&self, unit: Entity, item: ItemId) -> usize {
        let id = self.game.handle_of(0, unit).expect("own unit known");
        self.view
            .unit(id)
            .expect("own unit shown")
            .items
            .iter()
            .flatten()
            .filter(|held| held.id == item)
            .count()
    }

    fn fill_hero(&mut self) {
        for _ in 0..bota_server::game::BAG_SLOTS {
            self.cheat(
                None,
                Cheat::Item {
                    item: ItemId::IRON_BRANCH,
                },
            );
        }
        assert_eq!(
            self.count(self.hero(), ItemId::IRON_BRANCH),
            bota_server::game::BAG_SLOTS
        );
    }
}

fn snapshot(stream: &[ServerMsg]) -> WorldView {
    stream
        .iter()
        .find_map(|msg| match msg {
            ServerMsg::Snapshot { view } => Some(view.clone()),
            _ => None,
        })
        .expect("a snapshot each step")
}

#[test]
fn secret_purchase_is_refused_at_base_then_received_by_the_hero_at_the_shop() {
    let mut buyer = Buyer::new(MapId(2));
    let gold = buyer.view.player(SlotId(0)).expect("seat").gold;
    assert_eq!(
        buyer.order(
            None,
            Order::Buy {
                item: ItemId::DEMON_EDGE
            }
        ),
        Some(RejectReason::NotAtShop)
    );
    assert_eq!(buyer.view.player(SlotId(0)).expect("seat").gold, gold);
    assert_eq!(buyer.count(buyer.hero(), ItemId::DEMON_EDGE), 0);
    buyer.cheat(None, Cheat::Teleport { at: SHOP });
    assert_eq!(
        buyer.order(
            None,
            Order::Buy {
                item: ItemId::DEMON_EDGE
            }
        ),
        None
    );
    assert_eq!(buyer.count(buyer.hero(), ItemId::DEMON_EDGE), 1);
}

#[test]
fn a_courier_receives_secret_purchases_and_refuses_them_when_its_bag_is_full() {
    let mut buyer = Buyer::new(MapId(2));
    let courier = buyer.courier();
    buyer.cheat(Some(courier), Cheat::Teleport { at: SHOP });
    for count in 1..=bota_proto::COURIER_ITEM_SLOTS {
        assert_eq!(
            buyer.order(
                None,
                Order::Buy {
                    item: ItemId::DEMON_EDGE
                }
            ),
            None
        );
        assert_eq!(buyer.count(courier, ItemId::DEMON_EDGE), count);
    }
    let gold = buyer.view.player(SlotId(0)).expect("seat").gold;
    assert_eq!(
        buyer.order(
            None,
            Order::Buy {
                item: ItemId::DEMON_EDGE
            }
        ),
        Some(RejectReason::InventoryFull)
    );
    assert_eq!(buyer.view.player(SlotId(0)).expect("seat").gold, gold);
    assert!(
        buyer
            .view
            .player(SlotId(0))
            .expect("seat")
            .stash
            .as_ref()
            .expect("own stash")
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn a_full_hero_at_the_shop_uses_the_courier_with_room_at_the_same_shop() {
    let mut buyer = Buyer::new(MapId(2));
    buyer.cheat(None, Cheat::Teleport { at: SHOP });
    buyer.cheat(Some(buyer.courier()), Cheat::Teleport { at: SHOP });
    buyer.fill_hero();
    assert_eq!(
        buyer.order(
            None,
            Order::Buy {
                item: ItemId::DEMON_EDGE
            }
        ),
        None
    );
    assert_eq!(buyer.count(buyer.hero(), ItemId::DEMON_EDGE), 0);
    assert_eq!(buyer.count(buyer.courier(), ItemId::DEMON_EDGE), 1);
}

#[test]
fn demo_universal_shop_sells_secret_components_at_base() {
    let mut buyer = Buyer::new(MapId(1));
    assert_eq!(
        buyer.order(
            None,
            Order::Buy {
                item: ItemId::DEMON_EDGE
            }
        ),
        None
    );
    assert_eq!(buyer.count(buyer.hero(), ItemId::DEMON_EDGE), 1);
}
