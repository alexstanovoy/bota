//! Buyback (Dota 7.41f) end to end: every step is an order or a cheat a seat gives an in-process
//! match, and every check reads what the seats were sent back: rejections, each side's snapshot,
//! each side's events, and the replay's records.

use bota_core::{DEMO_ID, MAP2_ID, hero_spawn_pos, item_def, map_of, rules};
use bota_proto::{
    AbilitySlot, BuybackView, Cheat, EffectId, EntityId, EventKind, Fixed, ItemId, MapId, Order,
    OutgoingOrder, PlayerView, RejectReason, ReplayRecord, ServerMsg, SlotId, Target, Team,
    UnitView, WorldView, encode_frame_to_vec,
};

use super::local::mid_config;
use crate::game::OrderDelay;
use crate::local::{LocalError, LocalMatch};

const RADIANT: usize = 0;
const DIRE: usize = 1;
const COOLDOWN: u32 = 480 * rules::TICKS_PER_SECOND;
const PENALTY: u32 = 25 * rules::TICKS_PER_SECOND;

/// A cheat-enabled match of Shadow Fiend against Shadow Fiend, and what each seat was last sent.
struct Duel {
    local: LocalMatch,
    seq: u32,
    /// Each seat's latest snapshot.
    views: [WorldView; 2],
    /// Each seat's events of the latest step.
    events: [Vec<EventKind>; 2],
    /// Each seat's rejections of the latest step, in the order sent.
    rejected: [Vec<RejectReason>; 2],
    /// Every message each seat was sent, encoded as the wire carries it.
    wire: [Vec<u8>; 2],
    /// The winner, once the match is over.
    over: Option<Team>,
}

impl Duel {
    fn on(map: MapId, cheats: bool, recorded: bool) -> Duel {
        let mut cfg = mid_config();
        cfg.map = map;
        cfg.cheats = cheats;
        cfg.objectives = map_of(map).default_objectives;
        cfg.runes = map_of(map).default_runes;
        let (local, opening) = if recorded {
            LocalMatch::new_recorded(&cfg, &[OrderDelay::NONE])
        } else {
            LocalMatch::new(&cfg, &[OrderDelay::NONE])
        }
        .expect("a match");
        let mut duel = Duel {
            local,
            seq: 0,
            views: [empty_view(), empty_view()],
            events: [Vec::new(), Vec::new()],
            rejected: [Vec::new(), Vec::new()],
            wire: [Vec::new(), Vec::new()],
            over: None,
        };
        duel.take(opening);
        duel
    }

    /// A match on the map without a death limit, so a hero may die as often as a scenario needs.
    fn new() -> Duel {
        Duel::on(DEMO_ID, true, false)
    }

    fn take(&mut self, streams: Vec<Vec<ServerMsg>>) {
        for (seat, stream) in streams.into_iter().enumerate() {
            self.events[seat].clear();
            self.rejected[seat].clear();
            for msg in stream {
                self.wire[seat].extend(encode_frame_to_vec(&msg).expect("a frame"));
                match msg {
                    ServerMsg::Snapshot { view } => self.views[seat] = view,
                    ServerMsg::Events { events, .. } => self.events[seat].extend(events),
                    ServerMsg::OrderRejected { reason, .. } => self.rejected[seat].push(reason),
                    ServerMsg::MatchOver { winner, .. } => self.over = Some(winner),
                    _ => {}
                }
            }
        }
    }

    /// One tick with these orders of each seat, each naming who it names.
    fn step_naming(&mut self, orders: [&[(Option<EntityId>, Order)]; 2]) {
        let seen = self.local.tick();
        let mut out: [Vec<OutgoingOrder>; 2] = [Vec::new(), Vec::new()];
        for (seat, given) in orders.into_iter().enumerate() {
            for &(unit, order) in given {
                self.seq += 1;
                out[seat].push(OutgoingOrder {
                    seq: self.seq,
                    unit,
                    order,
                    seen,
                    queue: false,
                });
            }
        }
        let streams = self.local.step(&[&out[0], &out[1]]).expect("a step");
        self.take(streams);
    }

    /// One tick in which a seat gives one order naming nobody.
    fn give(&mut self, seat: usize, order: Order) {
        let given = [(None, order)];
        let mut orders: [&[(Option<EntityId>, Order)]; 2] = [&[], &[]];
        orders[seat] = &given;
        self.step_naming(orders);
    }

    /// One tick in which a seat gives one order that must not be turned down.
    fn done(&mut self, seat: usize, order: Order) {
        self.give(seat, order);
        assert_eq!(self.rejected[seat], [], "{order:?} was turned down");
    }

    fn cheat(&mut self, seat: usize, cheat: Cheat) {
        self.done(seat, Order::Cheat { cheat });
    }

    fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step_naming([&[], &[]]);
        }
    }

    fn run_to(&mut self, tick: u32) {
        assert!(self.tick() <= tick, "the match is past tick {tick}");
        self.run(tick - self.tick());
    }

    fn tick(&self) -> u32 {
        self.views[RADIANT].tick
    }

    /// A seat's row in what `viewer` was last sent.
    fn row(&self, viewer: usize, of: usize) -> &PlayerView {
        self.views[viewer]
            .player(SlotId(of as u8))
            .expect("every seat has a row")
    }

    /// A seat's own row.
    fn me(&self, seat: usize) -> &PlayerView {
        self.row(seat, seat)
    }

    /// A seat's buyback as it is told it.
    fn buyback(&self, seat: usize) -> BuybackView {
        self.me(seat)
            .buyback
            .expect("a seat is told its own buyback")
    }

    fn gold(&self, seat: usize) -> i32 {
        self.me(seat).gold.expect("a seat is told its own gold")
    }

    /// A seat's hero in its own view, while it stands.
    fn hero(&self, seat: usize) -> Option<&UnitView> {
        self.views[seat].unit(self.me(seat).unit?)
    }

    /// Sets a seat's gold to exactly this much, by Dota's `-gold`.
    fn set_gold(&mut self, seat: usize, gold: i32) {
        let amount = gold - self.gold(seat);
        self.cheat(seat, Cheat::Gold { amount });
        assert_eq!(self.gold(seat), gold);
    }

    /// A seat's hero dies by Dota's `-suicide`.
    fn kill(&mut self, seat: usize) {
        self.cheat(seat, Cheat::Suicide);
        assert!(self.me(seat).respawn_left > 0, "the hero lies dead");
    }

    /// A seat's hero dies, and the seat buys it back at once. Answers the tick the buyback was
    /// given on, which is the tick it is counted from: an order given on the view of tick T
    /// is carried out on it, and tick T + 1 shows what it did.
    fn die_and_buy_back(&mut self, seat: usize) -> u32 {
        self.kill(seat);
        let given = self.tick();
        self.done(seat, Order::Buyback);
        assert_eq!(self.me(seat).respawn_left, 0, "the hero stands");
        given
    }
}

fn empty_view() -> WorldView {
    WorldView {
        tick: 0,
        viewer: None,
        units: Vec::new(),
        projectiles: Vec::new(),
        players: Vec::new(),
        felled_trees: Vec::new(),
        planted_trees: Vec::new(),
        loot: Vec::new(),
        aggro: Vec::new(),
        order_delays: Vec::new(),
        skipped_ticks: Vec::new(),
        stock: Vec::new(),
        talents: Vec::new(),
        teleports: Vec::new(),
        glyphs: Vec::new(),
        clock: None,
        runes: Vec::new(),
        pools: Vec::new(),
        lairs: Vec::new(),
    }
}

fn bought_back(events: &[EventKind]) -> Vec<SlotId> {
    events
        .iter()
        .filter_map(|event| match event {
            EventKind::BoughtBack { slot } => Some(*slot),
            _ => None,
        })
        .collect()
}

#[test]
fn a_dead_hero_bought_back_stands_at_its_fountain_with_full_pools_and_the_seat_pays_the_cost() {
    let mut duel = Duel::new();
    duel.kill(RADIANT);
    let (gold, cost) = (duel.gold(RADIANT), duel.buyback(RADIANT).cost);
    assert!(duel.hero(RADIANT).is_none(), "a dead hero stands nowhere");
    duel.done(RADIANT, Order::Buyback);
    let me = duel.me(RADIANT);
    assert_eq!(me.respawn_left, 0);
    assert_eq!(me.deaths, 1, "the death bought out of stays counted");
    assert_eq!(duel.gold(RADIANT), gold - cost);
    let hero = duel.hero(RADIANT).expect("the hero stands again");
    assert_eq!((hero.hp, hero.mana), (hero.max_hp, hero.max_mana));
    let fountain = hero_spawn_pos(map_of(DEMO_ID), Team::Radiant);
    assert!(
        hero.pos.within(fountain, Fixed::from_int(1)),
        "at {:?}, the fountain is at {fountain:?}",
        hero.pos
    );
    assert!(
        hero.effects
            .iter()
            .any(|effect| effect.id == EffectId::FOUNTAIN_INVULNERABLE),
        "risen as at a respawn"
    );
}

#[test]
fn a_buyback_starts_a_480_second_cooldown_and_the_seat_may_buy_back_from_the_tick_it_ends_on() {
    let mut duel = Duel::new();
    duel.set_gold(RADIANT, 50_000);
    assert_eq!(duel.buyback(RADIANT).ready_at, 0, "ready from the start");
    let bought = duel.die_and_buy_back(RADIANT);
    assert_eq!(duel.buyback(RADIANT).ready_at, bought + COOLDOWN);
    duel.kill(RADIANT);
    duel.give(RADIANT, Order::Buyback);
    assert_eq!(duel.rejected[RADIANT], [RejectReason::OnCooldown]);
    duel.run_to(bought + COOLDOWN - 100);
    assert_eq!(duel.me(RADIANT).respawn_left, 0, "long risen by itself");
    duel.kill(RADIANT);
    duel.run_to(bought + COOLDOWN - 1);
    assert!(duel.me(RADIANT).respawn_left > 2, "dead across both ticks");
    duel.give(RADIANT, Order::Buyback);
    assert_eq!(
        duel.rejected[RADIANT],
        [RejectReason::OnCooldown],
        "given a tick before the cooldown ends"
    );
    let (gold, cost) = (duel.gold(RADIANT), duel.buyback(RADIANT).cost);
    assert_eq!(duel.tick(), bought + COOLDOWN);
    duel.done(RADIANT, Order::Buyback);
    assert_eq!(
        duel.me(RADIANT).respawn_left,
        0,
        "given on the tick it is ready from"
    );
    assert_eq!(duel.gold(RADIANT), gold - cost);
    assert_eq!(duel.buyback(RADIANT).ready_at, bought + 2 * COOLDOWN);
}

#[test]
fn a_standing_hero_cannot_buy_back() {
    let mut duel = Duel::new();
    let gold = duel.gold(RADIANT);
    duel.give(RADIANT, Order::Buyback);
    assert_eq!(duel.rejected[RADIANT], [RejectReason::HeroAlive]);
    assert_eq!(duel.gold(RADIANT), gold, "nothing is paid");
    assert_eq!(duel.me(RADIANT).bought_back_at, None);
    assert_eq!(bought_back(&duel.events[RADIANT]), []);
}

#[test]
fn a_buyback_needs_its_cost_in_gold_and_exactly_the_cost_is_enough() {
    let mut duel = Duel::new();
    duel.kill(RADIANT);
    // With no item of worth, a seat holding 214 is worth 214: 200 + 214 / 13 = 216.
    duel.set_gold(RADIANT, 214);
    assert_eq!(duel.buyback(RADIANT).cost, 216);
    duel.give(RADIANT, Order::Buyback);
    assert_eq!(duel.rejected[RADIANT], [RejectReason::NotEnoughGold]);
    assert!(
        duel.me(RADIANT).respawn_left > 0,
        "the hero still lies dead"
    );
    assert_eq!(duel.gold(RADIANT), 214);
    // 200 + 216 / 13 = 216: the seat holds the cost and not a coin more.
    duel.set_gold(RADIANT, 216);
    assert_eq!(duel.buyback(RADIANT).cost, 216);
    duel.done(RADIANT, Order::Buyback);
    assert_eq!(duel.me(RADIANT).respawn_left, 0);
    assert_eq!(duel.gold(RADIANT), 0);
}

#[test]
fn the_cost_is_200_and_a_thirteenth_of_gold_and_owned_items_whole() {
    let mut duel = Duel::new();
    // The scroll every hero starts with is free and worth nothing.
    for (gold, cost) in [(0, 200), (12, 200), (13, 201), (600, 246), (99_999, 7892)] {
        duel.set_gold(RADIANT, gold);
        assert_eq!(duel.buyback(RADIANT).cost, cost, "holding {gold}");
    }
    duel.set_gold(RADIANT, 600);
    let boots = item_def(ItemId::BOOTS).expect("boots").cost;
    assert_eq!(boots, 500);
    duel.cheat(
        RADIANT,
        Cheat::Item {
            item: ItemId::BOOTS,
        },
    );
    assert_eq!(duel.buyback(RADIANT).cost, 200 + (600 + 500) / 13);
    // Gold turned into an item at the shop leaves the seat worth the same.
    let tango = item_def(ItemId::TANGO).expect("tango").cost;
    duel.done(
        RADIANT,
        Order::Buy {
            item: ItemId::TANGO,
        },
    );
    assert_eq!(duel.gold(RADIANT), 600 - tango);
    assert_eq!(duel.buyback(RADIANT).cost, 200 + (600 + 500) / 13);
    // Dire's cost is Dire's own.
    assert_eq!(duel.buyback(DIRE).cost, 200 + duel.gold(DIRE) / 13);
}

#[test]
fn the_cost_knows_neither_the_clock_nor_the_level_nor_the_death() {
    let mut duel = Duel::new();
    duel.set_gold(RADIANT, 2_600);
    assert_eq!(duel.buyback(RADIANT).cost, 400);
    duel.cheat(RADIANT, Cheat::Levels { count: 15 });
    assert_eq!(duel.me(RADIANT).level, 16);
    assert_eq!(duel.buyback(RADIANT).cost, 400, "no level term");
    duel.cheat(RADIANT, Cheat::StartGame);
    duel.run(20 * 60 * rules::TICKS_PER_SECOND);
    assert!(duel.gold(RADIANT) > 2_600, "the clock paid gold meanwhile");
    assert_eq!(
        duel.buyback(RADIANT).cost,
        200 + duel.gold(RADIANT) / 13,
        "it follows the gold"
    );
    duel.set_gold(RADIANT, 2_600);
    assert_eq!(
        duel.buyback(RADIANT).cost,
        400,
        "no clock term twenty minutes in"
    );
    // A death costs gold, and the cost falls with it: it is priced when asked, not at the fall.
    duel.kill(RADIANT);
    let left = duel.gold(RADIANT);
    assert!(left < 2_600, "the death cost gold");
    assert_eq!(duel.buyback(RADIANT).cost, 200 + left / 13);
    duel.set_gold(RADIANT, 2_600);
    duel.done(RADIANT, Order::Buyback);
    assert_eq!(duel.gold(RADIANT), 2_200);
}

#[test]
fn the_next_death_after_a_buyback_lasts_25_seconds_longer_and_the_one_after_does_not() {
    let mut duel = Duel::new();
    duel.set_gold(RADIANT, 50_000);
    duel.kill(RADIANT);
    let plain = duel.me(RADIANT).respawn_left;
    // Level one waits twelve seconds; one tick of them has passed in the step that killed it.
    assert_eq!(plain, 12 * rules::TICKS_PER_SECOND - 1);
    assert_eq!(duel.buyback(RADIANT).respawn_penalty, 0);
    duel.done(RADIANT, Order::Buyback);
    assert_eq!(duel.buyback(RADIANT).respawn_penalty, PENALTY);
    // The penalty has no window: it waits for the death, however late.
    duel.run(60 * rules::TICKS_PER_SECOND);
    assert_eq!(duel.buyback(RADIANT).respawn_penalty, PENALTY);
    duel.kill(RADIANT);
    assert_eq!(duel.me(RADIANT).respawn_left, plain + PENALTY);
    assert_eq!(
        duel.buyback(RADIANT).respawn_penalty,
        0,
        "this death paid it"
    );
    assert_eq!(
        duel.row(DIRE, RADIANT).respawn_left,
        plain + PENALTY,
        "the wait is public"
    );
    duel.run(plain + PENALTY);
    assert_eq!(duel.me(RADIANT).respawn_left, 0);
    duel.kill(RADIANT);
    assert_eq!(duel.me(RADIANT).respawn_left, plain, "owed once");
}

#[test]
fn a_reincarnating_hero_cannot_buy_back_and_its_rising_leaves_the_penalty_owed() {
    let mut duel = Duel::new();
    duel.set_gold(RADIANT, 50_000);
    duel.die_and_buy_back(RADIANT);
    duel.cheat(RADIANT, Cheat::ResetBuyback);
    duel.cheat(
        RADIANT,
        Cheat::Item {
            item: ItemId::AEGIS,
        },
    );
    let deaths = duel.me(RADIANT).deaths;
    duel.cheat(RADIANT, Cheat::Suicide);
    let rising = duel.me(RADIANT).respawn_left;
    assert!(rising > 0, "it lies where it fell until it rises");
    assert_eq!(
        duel.me(RADIANT).deaths,
        deaths,
        "a reincarnation is no death"
    );
    assert_eq!(
        rising,
        item_def(ItemId::AEGIS)
            .expect("aegis")
            .reincarnation
            .expect("it reincarnates")
            - 1,
        "the Aegis's wait, with nothing of the buyback's added"
    );
    duel.give(RADIANT, Order::Buyback);
    assert_eq!(duel.rejected[RADIANT], [RejectReason::HeroAlive]);
    // Dota checks the gold before it checks the reincarnation.
    let gold = duel.gold(RADIANT);
    duel.set_gold(RADIANT, 0);
    duel.give(RADIANT, Order::Buyback);
    assert_eq!(duel.rejected[RADIANT], [RejectReason::NotEnoughGold]);
    duel.set_gold(RADIANT, gold);
    assert_eq!(duel.buyback(RADIANT).respawn_penalty, PENALTY, "still owed");
    duel.run(duel.me(RADIANT).respawn_left);
    assert!(duel.hero(RADIANT).is_some(), "risen");
    assert_eq!(duel.gold(RADIANT), gold, "nothing was paid");
    assert_eq!(duel.buyback(RADIANT).respawn_penalty, PENALTY);
    duel.kill(RADIANT);
    assert_eq!(
        duel.me(RADIANT).respawn_left,
        12 * rules::TICKS_PER_SECOND - 1 + PENALTY,
        "the next death that is one pays it"
    );
}

#[test]
fn a_buyback_refreshes_no_ability_cooldown() {
    let mut duel = Duel::new();
    duel.done(
        RADIANT,
        Order::Learn {
            slot: AbilitySlot(0),
        },
    );
    duel.done(
        RADIANT,
        Order::Cast {
            slot: AbilitySlot(0),
            target: Target::None,
        },
    );
    let mut waited = 0;
    while duel.hero(RADIANT).expect("standing").abilities[0].cooldown_left == 0 {
        duel.run(1);
        waited += 1;
        assert!(waited < 90, "the raze went off");
    }
    let before = duel.hero(RADIANT).expect("standing").abilities[0].cooldown_left;
    assert!(
        before > 10,
        "a cooldown long enough to outlast the scenario"
    );
    duel.die_and_buy_back(RADIANT);
    let after = duel.hero(RADIANT).expect("standing").abilities[0].cooldown_left;
    assert_eq!(
        after,
        before - 2,
        "two ticks ran, and nothing else took it down"
    );
}

#[test]
fn everyone_is_told_of_a_buyback_and_only_its_side_of_its_cost_and_cooldown() {
    let mut duel = Duel::new();
    assert_eq!(duel.row(DIRE, RADIANT).bought_back_at, None);
    assert_eq!(duel.row(DIRE, RADIANT).buyback, None);
    let bought = duel.die_and_buy_back(RADIANT);
    for viewer in [RADIANT, DIRE] {
        assert_eq!(
            bought_back(&duel.events[viewer]),
            [SlotId(0)],
            "seat {viewer} is told of the buyback"
        );
        assert_eq!(duel.row(viewer, RADIANT).bought_back_at, Some(bought));
        assert_eq!(duel.row(viewer, DIRE).bought_back_at, None);
    }
    let theirs = duel.row(DIRE, RADIANT);
    assert_eq!(
        theirs.buyback, None,
        "neither cost nor cooldown for the other side"
    );
    assert_eq!(theirs.gold, None);
    assert_eq!(
        duel.me(RADIANT).buyback,
        Some(BuybackView {
            cost: 200 + duel.gold(RADIANT) / 13,
            ready_at: bought + COOLDOWN,
            respawn_penalty: PENALTY,
        })
    );
    duel.run(1);
    assert_eq!(bought_back(&duel.events[DIRE]), [], "told once");
}

#[test]
fn a_replay_keeps_the_buyback_order_its_event_and_both_seats_buyback() {
    let mut duel = Duel::on(DEMO_ID, true, true);
    let bought = duel.die_and_buy_back(RADIANT);
    let records = duel.local.take_records();
    let ordered = records.iter().any(|record| {
        matches!(record, ReplayRecord::Orders { tick, orders }
            if *tick == bought + 1 && orders.iter().any(|told| told.order == Order::Buyback))
    });
    assert!(ordered, "the order is kept on the tick it was applied");
    let told = records.iter().any(|record| {
        matches!(record, ReplayRecord::Msg(ServerMsg::Events { tick, events })
            if *tick == bought + 1 && bought_back(events) == [SlotId(0)])
    });
    assert!(told, "the event is kept");
    let last = records
        .iter()
        .rev()
        .find_map(|record| match record {
            ReplayRecord::Msg(ServerMsg::Snapshot { view }) => Some(view),
            _ => None,
        })
        .expect("a snapshot");
    assert_eq!(last.viewer, None);
    let rows: Vec<_> = last
        .players
        .iter()
        .map(|player| {
            (
                player.bought_back_at,
                player.buyback.map(|told| told.ready_at),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [(Some(bought), Some(bought + COOLDOWN)), (None, Some(0))],
        "a watcher of everything is told both seats' buyback"
    );
}

#[test]
fn the_same_orders_buy_back_to_the_same_match_bit_for_bit() {
    let play = || {
        let mut duel = Duel::new();
        duel.set_gold(RADIANT, 9_000);
        duel.run(40);
        duel.die_and_buy_back(RADIANT);
        duel.kill(DIRE);
        duel.done(DIRE, Order::Buyback);
        duel.run(200);
        duel.kill(RADIANT);
        duel.run(100);
        (duel.local.world().hash(), duel.wire)
    };
    let (first, second) = (play(), play());
    assert_eq!(first.0, second.0, "the world");
    assert!(first.1 == second.1, "what each seat was sent");
}

#[test]
fn a_buyback_names_nobody_or_the_seats_own_hero() {
    let mut duel = Duel::new();
    let hero = |duel: &Duel, seat: usize| duel.me(seat).unit.expect("its hero");
    let courier = {
        let entity = duel.local.world().seats[RADIANT]
            .courier
            .expect("a courier");
        duel.local.handle_of(RADIANT, entity).expect("told of it")
    };
    // The heroes are brought face to face, so that each side holds a name for the other's.
    let meet = hero_spawn_pos(map_of(DEMO_ID), Team::Radiant);
    duel.cheat(DIRE, Cheat::Teleport { at: meet });
    duel.run(1);
    let own = hero(&duel, RADIANT);
    let theirs = duel.views[RADIANT]
        .units
        .iter()
        .find(|unit| unit.team == Team::Dire && unit.kind == bota_proto::UnitKind::Hero)
        .expect("the other hero is in sight")
        .id;
    duel.kill(RADIANT);
    let gold = duel.gold(RADIANT);
    for (named, reason) in [
        (courier, RejectReason::NotAHero),
        (theirs, RejectReason::NotYourUnit),
    ] {
        duel.step_naming([&[(Some(named), Order::Buyback)], &[]]);
        assert_eq!(duel.rejected[RADIANT], [reason]);
        assert!(duel.me(RADIANT).respawn_left > 0);
        assert_eq!(duel.gold(RADIANT), gold);
    }
    // The other seat cannot buy this hero back, whatever it names.
    let dire_gold = duel.gold(DIRE);
    duel.step_naming([&[], &[(None, Order::Buyback)]]);
    assert_eq!(duel.rejected[DIRE], [RejectReason::HeroAlive]);
    assert!(duel.me(RADIANT).respawn_left > 0);
    assert_eq!((duel.gold(RADIANT), duel.gold(DIRE)), (gold, dire_gold));
    duel.step_naming([&[(Some(own), Order::Buyback)], &[]]);
    assert_eq!(duel.rejected[RADIANT], []);
    assert_eq!(duel.me(RADIANT).respawn_left, 0, "its own hero, named");
}

#[test]
fn buyback_orders_given_at_once_buy_back_once() {
    let mut duel = Duel::new();
    duel.set_gold(RADIANT, 50_000);
    duel.kill(RADIANT);
    let (gold, cost) = (duel.gold(RADIANT), duel.buyback(RADIANT).cost);
    let spam = [(None, Order::Buyback); 5];
    duel.step_naming([&spam, &[]]);
    assert_eq!(duel.rejected[RADIANT], [RejectReason::HeroAlive; 4]);
    assert_eq!(duel.gold(RADIANT), gold - cost, "paid once");
    assert_eq!(bought_back(&duel.events[DIRE]), [SlotId(0)], "told once");
    assert_eq!(duel.me(RADIANT).respawn_left, 0);
}

#[test]
fn a_buyback_on_the_last_tick_of_a_death_buys_back_and_one_on_the_tick_after_is_refused() {
    let mut duel = Duel::new();
    duel.set_gold(RADIANT, 50_000);
    duel.kill(RADIANT);
    duel.run(duel.me(RADIANT).respawn_left - 1);
    assert_eq!(
        duel.me(RADIANT).respawn_left,
        1,
        "the hero rises on the next tick"
    );
    let (gold, cost) = (duel.gold(RADIANT), duel.buyback(RADIANT).cost);
    duel.done(RADIANT, Order::Buyback);
    assert_eq!(
        duel.gold(RADIANT),
        gold - cost,
        "still dead when the order came"
    );
    assert_eq!(duel.buyback(RADIANT).respawn_penalty, PENALTY);

    let mut late = Duel::new();
    late.set_gold(RADIANT, 50_000);
    late.kill(RADIANT);
    late.run(late.me(RADIANT).respawn_left);
    assert_eq!(late.me(RADIANT).respawn_left, 0, "the hero rose by itself");
    let gold = late.gold(RADIANT);
    late.give(RADIANT, Order::Buyback);
    assert_eq!(late.rejected[RADIANT], [RejectReason::HeroAlive]);
    assert_eq!(late.gold(RADIANT), gold);
    assert_eq!(late.me(RADIANT).bought_back_at, None);
}

#[test]
fn resetbuyback_ends_the_cooldown_and_leaves_the_penalty_and_respawn_clears_the_penalty() {
    let mut duel = Duel::new();
    duel.set_gold(RADIANT, 50_000);
    duel.die_and_buy_back(RADIANT);
    // Dota's `-refresh` and `-wtf` leave a buyback's cooldown as it is.
    duel.cheat(RADIANT, Cheat::Refresh);
    duel.cheat(RADIANT, Cheat::Wtf { on: true });
    duel.kill(RADIANT);
    duel.give(RADIANT, Order::Buyback);
    assert_eq!(duel.rejected[RADIANT], [RejectReason::OnCooldown]);
    duel.cheat(RADIANT, Cheat::ResetBuyback);
    assert_eq!(
        duel.buyback(RADIANT).ready_at,
        duel.tick() - 1,
        "the tick it was given on"
    );
    duel.done(RADIANT, Order::Buyback);
    assert_eq!(duel.buyback(RADIANT).respawn_penalty, PENALTY);
    duel.cheat(RADIANT, Cheat::ResetBuyback);
    assert_eq!(
        duel.buyback(RADIANT).respawn_penalty,
        PENALTY,
        "the reset is the cooldown's"
    );
    duel.cheat(RADIANT, Cheat::Respawn);
    assert_eq!(
        duel.buyback(RADIANT).respawn_penalty,
        0,
        "Dota's respawn clears it"
    );
    assert!(duel.buyback(RADIANT).ready_at <= duel.tick());
}

#[test]
fn solo_mid_has_buyback_and_the_death_bought_out_of_still_counts_towards_the_two_that_end_it() {
    let mut duel = Duel::on(MAP2_ID, true, false);
    duel.kill(RADIANT);
    assert_eq!(duel.over, None);
    let cost = duel.buyback(RADIANT).cost;
    let gold = duel.gold(RADIANT);
    assert_eq!(cost, 200 + gold / 13);
    duel.done(RADIANT, Order::Buyback);
    assert!(duel.hero(RADIANT).is_some(), "bought back");
    assert_eq!(duel.gold(RADIANT), gold - cost);
    assert_eq!(duel.me(RADIANT).deaths, 1);
    assert_eq!(duel.over, None);
    duel.cheat(RADIANT, Cheat::Suicide);
    assert_eq!(
        duel.over,
        Some(Team::Dire),
        "the second death ends the match"
    );
    assert_eq!(
        duel.local.step(&[&[], &[]]).err(),
        Some(LocalError::MatchOver),
        "a finished match takes no buyback"
    );
}

#[test]
fn the_buyback_cheats_need_a_match_with_cheats() {
    let mut duel = Duel::on(DEMO_ID, false, false);
    for cheat in [Cheat::Suicide, Cheat::ResetBuyback] {
        duel.give(RADIANT, Order::Cheat { cheat });
        assert_eq!(duel.rejected[RADIANT], [RejectReason::NoCheats]);
    }
    assert_eq!(duel.me(RADIANT).respawn_left, 0);
}

#[test]
fn an_item_another_seat_owns_adds_nothing_to_the_carriers_cost() {
    let mut duel = Duel::new();
    let meet = hero_spawn_pos(map_of(DEMO_ID), Team::Radiant);
    duel.cheat(DIRE, Cheat::Teleport { at: meet });
    duel.cheat(
        DIRE,
        Cheat::Item {
            item: ItemId::BOOTS,
        },
    );
    duel.set_gold(RADIANT, 600);
    duel.set_gold(DIRE, 600);
    assert_eq!(
        duel.buyback(DIRE).cost,
        200 + (600 + 500) / 13,
        "its owner's"
    );
    duel.done(
        DIRE,
        Order::Put {
            slot: bota_proto::ItemSlot(0),
            target: Target::None,
        },
    );
    let mut waited = 0;
    while duel.views[RADIANT].loot.is_empty() {
        duel.run(1);
        waited += 1;
        assert!(waited < 300, "the boots lie on the ground");
    }
    assert_eq!(
        duel.buyback(DIRE).cost,
        246,
        "what lies on the ground counts for nothing"
    );
    let boots = duel.views[RADIANT].loot[0].id;
    duel.done(
        RADIANT,
        Order::Take {
            target: Target::Unit(boots),
        },
    );
    let carries = |duel: &Duel| {
        duel.hero(RADIANT).is_some_and(|hero| {
            hero.items
                .iter()
                .flatten()
                .any(|item| item.id == ItemId::BOOTS)
        })
    };
    while !carries(&duel) {
        duel.run(1);
        waited += 1;
        assert!(waited < 600, "Radiant took the boots up");
    }
    duel.set_gold(RADIANT, 600);
    assert_eq!(
        duel.buyback(RADIANT).cost,
        246,
        "Dire's boots in Radiant's bag are not Radiant's worth"
    );
    duel.set_gold(DIRE, 600);
    assert_eq!(duel.buyback(DIRE).cost, 246, "nor Dire's, out of its hands");
}

#[test]
fn a_touched_buyback_and_nothing_else_moves_the_world_fingerprint() {
    let mut touched = Duel::new();
    let mut plain = Duel::new();
    touched.run(5);
    plain.run(5);
    assert_eq!(touched.local.world().hash(), plain.local.world().hash());
    // Each gives one cheat that changes nothing else: a reset of a cooldown that does not run,
    // and no gold.
    touched.cheat(RADIANT, Cheat::ResetBuyback);
    plain.cheat(RADIANT, Cheat::Gold { amount: 0 });
    assert_eq!(touched.gold(RADIANT), plain.gold(RADIANT));
    assert_ne!(
        touched.local.world().hash(),
        plain.local.world().hash(),
        "the buyback's cooldown is part of the state"
    );
}

#[test]
fn suicide_does_nothing_to_a_hero_that_lies_dead() {
    let mut duel = Duel::new();
    duel.kill(RADIANT);
    let left = duel.me(RADIANT).respawn_left;
    duel.cheat(RADIANT, Cheat::Suicide);
    assert_eq!(duel.me(RADIANT).deaths, 1, "one death");
    assert_eq!(
        duel.me(RADIANT).respawn_left,
        left - 1,
        "the same wait runs on"
    );
}

/// The charges of what a seat's hero holds in a bag slot, as the seat is told; none for an empty
/// slot.
fn charges_in(duel: &Duel, seat: usize, slot: usize) -> Option<u8> {
    let held = duel.hero(seat)?.items.get(slot).copied().flatten()?;
    Some(held.charges.unwrap_or(0))
}

/// What a seat is worth by its buyback's cost, less its gold: the worth of what it owns, to
/// the thirteen gold the cost's division hides.
fn cost_with(duel: &Duel, seat: usize, items: i32) -> i32 {
    200 + (duel.gold(seat) + items) / 13
}

#[test]
fn a_bottle_is_worth_its_whole_cost_full_and_empty() {
    let mut duel = Duel::new();
    let bottle = item_def(ItemId::BOTTLE).expect("a bottle").cost;
    assert_eq!(bottle, 675);
    // Away from the fountain, which would fill it again.
    let fountains = map_of(DEMO_ID).fountains;
    let away = bota_proto::Vec2::from_ints(
        (fountains[0].x.to_int() + fountains[1].x.to_int()) / 2,
        (fountains[0].y.to_int() + fountains[1].y.to_int()) / 2,
    );
    duel.cheat(RADIANT, Cheat::Teleport { at: away });
    duel.cheat(
        RADIANT,
        Cheat::Item {
            item: ItemId::BOTTLE,
        },
    );
    duel.set_gold(RADIANT, 600);
    assert_eq!(charges_in(&duel, RADIANT, 0), Some(3), "bought full");
    assert_eq!(duel.buyback(RADIANT).cost, cost_with(&duel, RADIANT, 675));
    let mut waited = 0;
    while charges_in(&duel, RADIANT, 0) != Some(0) {
        duel.give(
            RADIANT,
            Order::Use {
                slot: bota_proto::ItemSlot(0),
                target: Target::None,
            },
        );
        duel.run(15);
        waited += 1;
        assert!(waited < 400, "the bottle was drunk dry");
    }
    duel.set_gold(RADIANT, 600);
    assert_eq!(
        duel.buyback(RADIANT).cost,
        200 + (600 + 675) / 13,
        "an empty Bottle stays, and stays worth what it cost"
    );
}

#[test]
fn a_stack_of_consumables_is_worth_its_charges_and_nothing_once_used_up() {
    let mut duel = Duel::new();
    let clarity = item_def(ItemId::CLARITY).expect("a clarity").cost;
    duel.cheat(
        RADIANT,
        Cheat::Item {
            item: ItemId::CLARITY,
        },
    );
    duel.cheat(
        RADIANT,
        Cheat::Item {
            item: ItemId::CLARITY,
        },
    );
    duel.set_gold(RADIANT, 600);
    assert_eq!(charges_in(&duel, RADIANT, 0), Some(2), "one stack of two");
    assert_eq!(
        duel.buyback(RADIANT).cost,
        200 + (600 + 2 * clarity) / 13,
        "a stack is worth each of its charges"
    );
    let own = duel.me(RADIANT).unit.expect("its hero");
    let drink = Order::Use {
        slot: bota_proto::ItemSlot(0),
        target: Target::Unit(own),
    };
    for (left, worth) in [(Some(1), clarity), (None, 0)] {
        let mut waited = 0;
        while charges_in(&duel, RADIANT, 0) != left {
            duel.give(RADIANT, drink);
            duel.run(10);
            waited += 1;
            assert!(waited < 400, "a clarity was drunk");
        }
        duel.set_gold(RADIANT, 600);
        assert_eq!(
            duel.buyback(RADIANT).cost,
            200 + (600 + worth) / 13,
            "with {left:?} left"
        );
    }
}

#[test]
fn a_charge_holder_that_comes_empty_is_worth_its_whole_cost_and_a_free_charge_nothing() {
    let mut duel = Duel::new();
    let stick = item_def(ItemId::MAGIC_STICK).expect("a stick").cost;
    duel.cheat(
        RADIANT,
        Cheat::Item {
            item: ItemId::MAGIC_STICK,
        },
    );
    duel.set_gold(RADIANT, 600);
    assert_eq!(charges_in(&duel, RADIANT, 0), Some(0), "it comes empty");
    assert_eq!(duel.buyback(RADIANT).cost, 200 + (600 + stick) / 13);
    // A bought scroll joins the free one every hero starts with: the stack of two is worth
    // the one that was paid for.
    let scroll = item_def(ItemId::TOWN_PORTAL_SCROLL).expect("a scroll").cost;
    duel.set_gold(RADIANT, 600 + scroll);
    duel.done(
        RADIANT,
        Order::Buy {
            item: ItemId::TOWN_PORTAL_SCROLL,
        },
    );
    assert_eq!(duel.gold(RADIANT), 600);
    assert_eq!(
        duel.hero(RADIANT)
            .expect("standing")
            .town_portal
            .and_then(|held| held.charges),
        Some(2)
    );
    assert_eq!(
        duel.buyback(RADIANT).cost,
        200 + (600 + stick + scroll) / 13
    );
}

#[test]
fn a_ward_dispenser_is_worth_the_wards_it_holds() {
    let mut duel = Duel::new();
    let sentry = item_def(ItemId::SENTRY_WARD).expect("a sentry").cost;
    let observer = item_def(ItemId::OBSERVER_WARD).expect("an observer").cost;
    assert_eq!((observer, sentry), (0, 50));
    for item in [
        ItemId::OBSERVER_WARD,
        ItemId::SENTRY_WARD,
        ItemId::SENTRY_WARD,
    ] {
        duel.cheat(RADIANT, Cheat::Item { item });
    }
    duel.set_gold(RADIANT, 600);
    let held = duel.hero(RADIANT).expect("standing").items[0].expect("the wards");
    assert_eq!(held.id, ItemId::WARD_DISPENSER, "the wards came together");
    assert_eq!(duel.buyback(RADIANT).cost, 200 + (600 + 2 * sentry) / 13);
}

/// A dead hero holding boots received during this death, its seat holding a hundred gold: the
/// boots sell back for their cost, and only with that gold is the buyback paid for.
fn dead_with_boots_to_sell() -> Duel {
    let mut duel = Duel::new();
    duel.kill(RADIANT);
    duel.cheat(
        RADIANT,
        Cheat::Item {
            item: ItemId::BOOTS,
        },
    );
    duel.set_gold(RADIANT, 100);
    assert_eq!(duel.buyback(RADIANT).cost, 200 + (100 + 500) / 13);
    duel
}

#[test]
fn a_buyback_is_paid_from_what_a_sale_given_before_it_on_the_same_tick_brought() {
    let mut duel = dead_with_boots_to_sell();
    let sell = Order::Sell {
        slot: bota_proto::ItemSlot(0),
    };
    duel.step_naming([&[(None, sell), (None, Order::Buyback)], &[]]);
    assert_eq!(
        duel.rejected[RADIANT],
        [],
        "the sale's gold pays for the buyback"
    );
    assert_eq!(duel.me(RADIANT).respawn_left, 0, "bought back");
    // 100 and the boots' 500 back, less 200 + 600 / 13.
    assert_eq!(duel.gold(RADIANT), 354);
}

#[test]
fn a_buyback_given_before_the_sale_that_would_pay_for_it_is_refused() {
    let mut duel = dead_with_boots_to_sell();
    let sell = Order::Sell {
        slot: bota_proto::ItemSlot(0),
    };
    duel.step_naming([&[(None, Order::Buyback), (None, sell)], &[]]);
    assert_eq!(duel.rejected[RADIANT], [RejectReason::NotEnoughGold]);
    assert!(
        duel.me(RADIANT).respawn_left > 0,
        "the hero still lies dead"
    );
    assert_eq!(duel.gold(RADIANT), 600, "the sale went through after it");
}
