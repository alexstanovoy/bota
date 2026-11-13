//! Fixed-seed admission checks through the real lockstep order path.

use super::local::mid_config;
use crate::game::{ActionPhase, ActionState, Entity, Modifier, OrderDelay};
use crate::local::LocalMatch;
use bota_client_core::order_legality::{OrderContext, UnknownReason, Verdict, mana_affordability};
use bota_core::{Catalogue, Control, ModifierKind, map_of};
use bota_proto::{
    AbilitySlot, Cheat, EffectId, EntityId, Fixed, HeroId, ItemId, ItemSlot, Order, OutgoingOrder,
    ServerMsg, SlotId, TalentSlot, Target, Vec2, WorldView,
};

const SEED: u64 = 0x194b_70d3_18ff_2105;

fn send(local: &mut LocalMatch, order: Order, seq: u32) -> Vec<ServerMsg> {
    let given = OutgoingOrder {
        seq,
        unit: None,
        order,
        queue: false,
        seen: local.tick(),
    };
    local.step(&[&[given], &[]]).expect("step").remove(0)
}

fn base(hero: HeroId) -> (LocalMatch, Entity) {
    base_on(hero, mid_config().map)
}

fn base_on(hero: HeroId, map: bota_proto::MapId) -> (LocalMatch, Entity) {
    let mut config = mid_config();
    config.map = map;
    config.objectives = map_of(map).default_objectives;
    config.runes = map_of(map).default_runes;
    config.cheats = true;
    config.picks[0].hero = hero;
    let (mut local, _) = LocalMatch::new(&config, &[OrderDelay::NONE]).expect("match");
    for (index, cheat) in [
        Cheat::Levels { count: 24 },
        Cheat::AllVision { on: true },
        Cheat::Item {
            item: ItemId::MANGO,
        },
        Cheat::Item {
            item: ItemId::TANGO,
        },
        Cheat::Item {
            item: ItemId::MAGIC_STICK,
        },
        Cheat::Item {
            item: ItemId::BOTTLE,
        },
        Cheat::Item {
            item: ItemId::TOWN_PORTAL_SCROLL,
        },
        Cheat::Refresh,
    ]
    .into_iter()
    .enumerate()
    {
        let stream = send(&mut local, Order::Cheat { cheat }, index as u32 + 1);
        assert!(
            !stream
                .iter()
                .any(|msg| matches!(msg, ServerMsg::OrderRejected { .. })),
            "cheat {cheat:?}"
        );
    }
    for slot in 0..6 {
        send(
            &mut local,
            Order::Learn {
                slot: AbilitySlot(slot),
            },
            20 + u32::from(slot),
        );
    }
    let hero = local.hero_of(0).expect("hero");
    (local, hero)
}

fn snapshot(local: &mut LocalMatch) -> WorldView {
    local
        .restate()
        .remove(0)
        .into_iter()
        .find_map(|msg| match msg {
            ServerMsg::Snapshot { view } => Some(view),
            _ => None,
        })
        .expect("snapshot")
}

fn compare(local: LocalMatch, order: Order, queue: bool) -> Verdict {
    compare_for(local, None, order, queue)
}

fn compare_for(
    mut local: LocalMatch,
    unit: Option<EntityId>,
    order: Order,
    queue: bool,
) -> Verdict {
    let view = snapshot(&mut local);
    let player = view
        .players
        .iter()
        .find(|p| p.slot == SlotId(0))
        .expect("player");
    let verdict = OrderContext {
        view: &view,
        player,
        map: local.world().map,
        catalogue: Catalogue::FULL,
        cheats: true,
    }
    .check(unit, order, queue);
    let given = OutgoingOrder {
        seq: 1000,
        unit,
        order,
        queue,
        seen: local.tick(),
    };
    let stream = local.step(&[&[given], &[]]).expect("step").remove(0);
    let refusal = stream.iter().find_map(|msg| match msg {
        ServerMsg::OrderRejected { seq: 1000, reason } => Some(*reason),
        _ => None,
    });
    match verdict {
        Verdict::Legal => assert!(refusal.is_none(), "seed={SEED:x} {order:?}: {refusal:?}"),
        Verdict::Illegal(_) => assert!(
            refusal.is_some(),
            "seed={SEED:x} {order:?} was accepted: {verdict:?}"
        ),
        Verdict::Unknown(_) => (),
    }
    verdict
}

fn arrange(local: &mut LocalMatch, hero: Entity, state: usize) {
    // Cheats above provide the legal kit and levels. The wire has no cheat for exact
    // pools, charges or controls; fixture-only mutations isolate those boundaries.
    match state {
        1 => local.world_mut().put_modifier(
            hero,
            Modifier::new(ModifierKind::Stunned, None, None, Some(1000)),
        ),
        2 => local.world_mut().put_modifier(
            hero,
            Modifier::new(
                ModifierKind::Control {
                    control: Control::Silence,
                    effect: EffectId(92),
                },
                None,
                None,
                Some(1000),
            ),
        ),
        3 => local.world_mut().put_modifier(
            hero,
            Modifier::new(ModifierKind::HookFollowthrough, None, None, Some(1000)),
        ),
        4 => {
            local
                .world_mut()
                .abilities
                .get_mut(hero)
                .expect("book")
                .slots[0]
                .cooldown = 100
        }
        5 => local.world_mut().mana.get_mut(hero).expect("mana").mana = Fixed::ZERO,
        6 => {
            let world = local.world_mut();
            let ability = world.abilities.get(hero).expect("book").slots[0];
            let cost = world.ability_mana_cost(hero, ability.id, ability.level);
            world.mana.get_mut(hero).expect("mana").mana = Fixed::from_int(cost);
        }
        7 | 8 => {
            let stack = local
                .world_mut()
                .inventory
                .get_mut(hero)
                .expect("bag")
                .slots[2]
                .as_mut()
                .expect("stick");
            stack.charges = (state - 7) as u8;
        }
        9 => {
            let bag = local.world_mut().inventory.get_mut(hero).expect("bag");
            bag.slots.swap(0, 6);
        }
        10 => {
            local.world_mut().kill_by_hp_removal(hero, hero);
        }
        11 | 12 => {
            local
                .world_mut()
                .action
                .get_mut(hero)
                .expect("action")
                .state = ActionState::CastAbility {
                slot: AbilitySlot(0),
                target: Target::None,
                phase: if state == 11 {
                    ActionPhase::During { progress: 1 }
                } else {
                    ActionPhase::Before { progress: 1 }
                },
            };
        }
        13 => local.world_mut().put_modifier(
            hero,
            Modifier::new(ModifierKind::Feared, None, None, Some(1000)),
        ),
        _ => (),
    }
}

fn arrange_catalogue_item(local: &mut LocalMatch, hero: Entity, row: usize, random: u64) {
    let def = &bota_core::ITEMS[row % bota_core::ITEMS.len()];
    if def.retired {
        return;
    }
    let Some(mut stack) =
        crate::game::ItemStack::bought(def.id, SlotId((random & 1) as u8), local.tick())
            .or_else(|| crate::game::ItemStack::dropped(def.id))
    else {
        return;
    };
    let at = if def.limits.backpack {
        (random >> 8) as usize % 9
    } else {
        0
    };
    if def.charges > 0 || def.cast_charges > 0 {
        stack.charges = ((random >> 16) & 1) as u8;
        stack.free_charges = stack.charges.min(((random >> 17) & 1) as u8);
    }
    if def.cooldown > 0 {
        stack.cooldown = ((random >> 18) & 1) as u32;
    }
    stack.disassemble_until = local.tick() + 500;
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[at] = Some(stack);
}

#[test]
fn order_legality_fixed_seed_matches_local_match_in_disabled_and_resource_states() {
    let mut counts = [0; 3];
    let mut required = [0; 3]; // Legal Take, Put to an ally, Disassemble.
    let mut random = SEED;
    let mut catalogue_row = 0;
    for (hero_kind, map) in [
        (HeroId::SHADOW_FIEND, 0),
        (HeroId::PUDGE, 0),
        (HeroId::SHADOW_FIEND, 2),
        (HeroId::PUDGE, 2),
    ] {
        let (base, hero) = base_on(hero_kind, bota_proto::MapId(map));
        for state in 0..14 {
            let mut local = base.clone();
            let at = local.world().map.fountains[0];
            let branch =
                crate::game::ItemStack::bought(ItemId::IRON_BRANCH, SlotId(0), local.tick())
                    .expect("branch");
            local.world_mut().lay_loot(branch, at);
            let build = bota_core::ITEMS
                .iter()
                .find(|def| {
                    !def.retired
                        && !def.components.is_empty()
                        && def.disassemble != bota_core::DisassembleRule::Never
                })
                .expect("disassemblable row");
            let mut stack =
                crate::game::ItemStack::bought(build.id, SlotId(0), local.tick()).expect("build");
            stack.disassemble_until = local.tick() + 500;
            local
                .world_mut()
                .inventory
                .get_mut(hero)
                .expect("bag")
                .slots[5] = Some(stack);
            send(
                &mut local,
                Order::Cheat {
                    cheat: Cheat::AllVision { on: state % 2 == 0 },
                },
                16000,
            );
            if state != 3 || hero_kind == HeroId::PUDGE {
                arrange(&mut local, hero, state);
            }
            let view = snapshot(&mut local);
            let id = view.players[0].unit.expect("identity survives death");
            let mut targets = vec![
                Target::None,
                Target::Pos(Vec2::from_ints(9000, 9000)),
                Target::Unit(id),
                Target::Unit(EntityId {
                    idx: u32::MAX,
                    generation: u32::MAX,
                }),
            ];
            targets.extend(view.units.iter().map(|unit| Target::Unit(unit.id)));
            targets.extend(view.loot.iter().map(|loot| Target::Unit(loot.id)));
            targets.extend(view.runes.iter().map(|rune| Target::Unit(rune.id)));
            let courier = local.world().seats[0]
                .courier
                .and_then(|unit| local.handle_of(0, unit));
            let loot = view.loot.first().expect("nearby loot").id;
            for _ in 0..100 {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                let slot = ((random >> 16) % 18) as u8;
                let target = targets[((random >> 32) as usize) % targets.len()];
                let order = match random % 22 {
                    0 => Order::Move { target },
                    1 => Order::Attack { target },
                    2 => Order::MoveDirection {
                        toward: Vec2::from_ints(9000, 9000),
                    },
                    3 => Order::Cast {
                        slot: AbilitySlot(slot),
                        target,
                    },
                    4 | 5 => Order::Use {
                        slot: ItemSlot(slot),
                        target,
                    },
                    6 => Order::Put {
                        slot: ItemSlot(slot),
                        target,
                    },
                    7 => Order::Take { target },
                    8 => Order::Buy {
                        item: ItemId(((random >> 40) % (bota_core::ITEMS.len() as u64 + 1)) as u16),
                    },
                    9 => Order::Sell {
                        slot: ItemSlot(slot),
                    },
                    10 => Order::Swap {
                        from: ItemSlot(slot),
                        to: ItemSlot(((random >> 24) % 18) as u8),
                    },
                    11 => Order::Learn {
                        slot: AbilitySlot(slot),
                    },
                    12 => Order::LearnTalent {
                        talent: TalentSlot(slot),
                    },
                    13 => Order::LearnAttributeBonus,
                    14 => Order::Glyph,
                    15 => Order::SwitchWard {
                        slot: ItemSlot(slot),
                    },
                    16 => Order::Disassemble {
                        slot: ItemSlot(slot),
                    },
                    17 => Order::SetCombineLock {
                        slot: ItemSlot(slot),
                        locked: true,
                    },
                    19 => Order::Take {
                        target: Target::Unit(loot),
                    },
                    20 => Order::Put {
                        slot: ItemSlot(0),
                        target: Target::Unit(courier.expect("courier")),
                    },
                    21 => Order::Disassemble { slot: ItemSlot(5) },
                    _ => Order::Cheat {
                        cheat: Cheat::Gold { amount: 1 },
                    },
                };
                let mut candidate = local.clone();
                arrange_catalogue_item(&mut candidate, hero, catalogue_row, random);
                catalogue_row += 1;
                let verdict = compare(candidate, order, random & 64 != 0);
                if verdict == Verdict::Legal {
                    match order {
                        Order::Take { .. } => required[0] += 1,
                        Order::Put {
                            target: Target::Unit(_),
                            ..
                        } => required[1] += 1,
                        Order::Disassemble { .. } => required[2] += 1,
                        _ => (),
                    }
                }
                counts[match verdict {
                    Verdict::Legal => 0,
                    Verdict::Illegal(_) => 1,
                    Verdict::Unknown(_) => 2,
                }] += 1;
            }
        }
    }
    assert!(
        required.iter().all(|&count| count > 0),
        "legal operation coverage {required:?}"
    );
    eprintln!("legal Take / Put-to-ally / Disassemble: {required:?}");
    assert!(counts[0] > 100, "legal coverage {counts:?}");
    assert!(counts[1] > 100, "illegal coverage {counts:?}");
    assert!(counts[2] > 20, "uncertain coverage {counts:?}");
    eprintln!(
        "seed={SEED:x}: legal={} illegal={} unknown={}",
        counts[0], counts[1], counts[2]
    );
}

#[test]
fn order_legality_reports_rounded_mana_and_hidden_hook_control_as_unknown() {
    assert_eq!(
        mana_affordability(1, 1),
        Verdict::Unknown(UnknownReason::RoundedMana)
    );
    let (mut local, hero) = base(HeroId::PUDGE);
    arrange(&mut local, hero, 3);
    assert_eq!(
        compare(
            local,
            Order::Cast {
                slot: AbilitySlot(1),
                target: Target::None
            },
            false
        ),
        Verdict::Unknown(UnknownReason::HiddenState)
    );
}

#[test]
fn order_legality_exercises_valid_casts_and_items_in_each_arranged_state() {
    for map in [bota_proto::MapId(1), bota_core::MAP2_ID] {
        for state in 0..14 {
            let caster = if state == 3 {
                HeroId::PUDGE
            } else {
                HeroId::SHADOW_FIEND
            };
            let (mut local, hero) = base_on(caster, map);
            arrange(&mut local, hero, state);
            for order in [
                Order::Move {
                    target: Target::Pos(Vec2::from_ints(8000, 8000)),
                },
                Order::Attack {
                    target: Target::None,
                },
                Order::Cast {
                    slot: AbilitySlot(0),
                    target: if caster == HeroId::PUDGE {
                        Target::Pos(Vec2::from_ints(8000, 8000))
                    } else {
                        Target::None
                    },
                },
                Order::Use {
                    slot: ItemSlot(0),
                    target: Target::None,
                },
                Order::Use {
                    slot: ItemSlot(1),
                    target: Target::Pos(Vec2::from_ints(8000, 8000)),
                },
                Order::Use {
                    slot: ItemSlot(2),
                    target: Target::None,
                },
                Order::Use {
                    slot: ItemSlot(6),
                    target: Target::None,
                },
            ] {
                let verdict = compare(local.clone(), order, false);
                if matches!(order, Order::Move { .. } | Order::Attack { .. }) {
                    assert_eq!(
                        verdict.is_illegal(),
                        state == 10,
                        "body admission in state {state}"
                    );
                }
                if matches!(order, Order::Cast { .. }) && caster == HeroId::SHADOW_FIEND {
                    match state {
                        1 | 4 | 5 | 10 | 13 => {
                            assert!(verdict.is_illegal(), "blocked cast in state {state}")
                        }
                        12 => assert_eq!(verdict, Verdict::Unknown(UnknownReason::QueueState)),
                        _ => assert_eq!(verdict, Verdict::Legal, "ready cast in state {state}"),
                    }
                }
            }
        }
    }
}

#[test]
fn order_legality_distinguishes_free_and_bought_scroll_sales_only_at_the_server() {
    let (mut free, hero) = base(HeroId::SHADOW_FIEND);
    let mut bought = free.clone();
    free.world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .town_portal
        .as_mut()
        .expect("slot")
        .as_mut()
        .expect("scroll")
        .free_charges = 1;
    bought
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .town_portal
        .as_mut()
        .expect("slot")
        .as_mut()
        .expect("scroll")
        .free_charges = 0;
    let scroll = |view: WorldView| {
        view.units
            .into_iter()
            .find(|unit| Some(unit.id) == view.players[0].unit)
            .expect("hero")
            .town_portal
    };
    assert_eq!(
        scroll(snapshot(&mut free)),
        scroll(snapshot(&mut bought)),
        "provenance is absent from the item view"
    );
    let unknown = Verdict::Unknown(UnknownReason::PurchaseHistory);
    let sale = Order::Sell {
        slot: ItemSlot::TOWN_PORTAL,
    };
    assert_eq!(compare(free.clone(), sale, false), unknown);
    assert_eq!(compare(bought.clone(), sale, false), unknown);
    let refused = send(&mut free, sale, 5000);
    assert!(refused.iter().any(|msg| matches!(
        msg,
        ServerMsg::OrderRejected {
            reason: bota_proto::RejectReason::Unsellable,
            ..
        }
    )));
    let accepted = send(&mut bought, sale, 5000);
    assert!(
        !accepted
            .iter()
            .any(|msg| matches!(msg, ServerMsg::OrderRejected { .. }))
    );
}

#[test]
fn order_legality_uses_the_issued_ledger_for_a_full_cast_point_queue() {
    use bota_client_core::order_legality::pseudo_queue::PseudoQueue;
    use bota_client_core::{ControlledUnit, IssuedOrder};
    let (mut local, _) = base(HeroId::SHADOW_FIEND);
    let mut ledger = PseudoQueue::default();
    for index in 0..=bota_core::rules::MAX_QUEUED_ORDERS {
        let view = snapshot(&mut local);
        let player = &view.players[0];
        let body = view
            .units
            .iter()
            .find(|unit| Some(unit.id) == player.unit)
            .expect("hero");
        let order = Order::Cast {
            slot: AbilitySlot(if index == 0 { 5 } else { 0 }),
            target: Target::None,
        };
        ledger.give(
            200 + index as u32,
            IssuedOrder::hero(order),
            view.tick,
            body,
        );
        let stream = send(&mut local, order, 200 + index as u32);
        assert!(
            !stream
                .iter()
                .any(|msg| matches!(msg, ServerMsg::OrderRejected { .. })),
            "queue entry {index}"
        );
        let now = snapshot(&mut local);
        let body = now
            .units
            .iter()
            .find(|unit| Some(unit.id) == now.players[0].unit)
            .expect("hero");
        ledger.observe(Some(body), None);
    }
    let view = snapshot(&mut local);
    let context = OrderContext {
        view: &view,
        player: &view.players[0],
        map: local.world().map,
        catalogue: Catalogue::FULL,
        cheats: true,
    };
    let cast = Order::Cast {
        slot: AbilitySlot(0),
        target: Target::None,
    };
    assert_eq!(
        context.check(None, cast, false),
        Verdict::Unknown(UnknownReason::QueueState)
    );
    assert_eq!(
        context.check_with_ledger(None, cast, false, ledger.owed(ControlledUnit::Hero)),
        Verdict::Illegal(bota_proto::RejectReason::QueueFull)
    );
    let stream = send(&mut local, cast, 1000);
    assert!(stream.iter().any(|msg| matches!(
        msg,
        ServerMsg::OrderRejected {
            seq: 1000,
            reason: bota_proto::RejectReason::QueueFull
        }
    )));
}

#[test]
fn order_legality_handles_oversized_and_extreme_public_views_without_panicking() {
    use bota_client_core::order_legality::{Reservations, ability_readiness, item_readiness};
    let (mut local, _) = base(HeroId::SHADOW_FIEND);
    let mut view = snapshot(&mut local);
    let hero = view.players[0].unit.expect("hero");
    let unit = view
        .units
        .iter_mut()
        .find(|unit| unit.id == hero)
        .expect("unit");
    unit.mana = i32::MIN;
    unit.abilities[0].mana_cost = i32::MAX;
    assert!(ability_readiness(unit, &unit.abilities[0]).is_illegal());
    unit.items[0].as_mut().expect("item").mana_cost = i32::MIN;
    assert!(!item_readiness(unit, ItemSlot(0), unit.items[0].as_ref().expect("item")).is_legal());
    let reservations = Reservations::of(unit, &Default::default());
    assert!(!reservations.allows_cast(unit.mana, unit.abilities[0].id, i32::MAX, false));
    assert!(!reservations.names(usize::MAX));
    unit.pos.x = Fixed::MIN;
    let context = OrderContext {
        view: &view,
        player: &view.players[0],
        map: local.world().map,
        catalogue: Catalogue::FULL,
        cheats: true,
    };
    assert_eq!(
        context.check(
            None,
            Order::Move {
                target: Target::None
            },
            false
        ),
        Verdict::Unknown(UnknownReason::MalformedView)
    );
    let template = view.units[0].clone();
    view.units
        .resize(bota_proto::limits::MAX_VIEW_UNITS + 1, template);
    assert_eq!(
        OrderContext {
            view: &view,
            player: &view.players[0],
            map: local.world().map,
            catalogue: Catalogue::FULL,
            cheats: true
        }
        .check(None, Order::Glyph, false),
        Verdict::Unknown(UnknownReason::MalformedView)
    );
}

#[test]
fn order_legality_keeps_fractional_and_exact_one_mana_admission_distinct_from_the_view() {
    let (mut fraction, hero) = base(HeroId::SHADOW_FIEND);
    fraction
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[5] = crate::game::ItemStack::bought(ItemId::TOWN_PORTAL_SCROLL, SlotId(0), 0);
    fraction.world_mut().seats[0].item_clocks.clear();
    fraction
        .world_mut()
        .stats
        .get_mut(hero)
        .expect("stats")
        .mana_cost_rate_bp = 200;
    fraction.world_mut().mana.get_mut(hero).expect("mana").mana = Fixed::from_ratio(1, 2);
    let mut exact = fraction.clone();
    exact.world_mut().mana.get_mut(hero).expect("mana").mana = Fixed::ONE;
    for local in [&mut fraction, &mut exact] {
        let view = snapshot(local);
        let unit = view
            .units
            .iter()
            .find(|unit| Some(unit.id) == view.players[0].unit)
            .expect("hero");
        assert_eq!(unit.mana, 1);
        assert_eq!(unit.items[5].as_ref().expect("scroll").mana_cost, 1);
    }
    let view = snapshot(&mut exact);
    let fountain = view
        .units
        .iter()
        .find(|unit| {
            unit.kind == bota_proto::UnitKind::Fountain && unit.team == view.players[0].team
        })
        .expect("fountain")
        .id;
    let order = Order::Use {
        slot: ItemSlot(5),
        target: Target::Unit(fountain),
    };
    assert_eq!(
        compare(fraction.clone(), order, false),
        Verdict::Unknown(UnknownReason::RoundedMana)
    );
    assert_eq!(
        compare(exact.clone(), order, false),
        Verdict::Unknown(UnknownReason::RoundedMana)
    );
    let refused = send(&mut fraction, order, 6000);
    assert!(refused.iter().any(|msg| matches!(
        msg,
        ServerMsg::OrderRejected {
            reason: bota_proto::RejectReason::NotEnoughMana,
            ..
        }
    )));
    let accepted = send(&mut exact, order, 6000);
    assert!(
        !accepted
            .iter()
            .any(|msg| matches!(msg, ServerMsg::OrderRejected { .. }))
    );
}

#[test]
fn order_legality_does_not_infer_an_empty_mana_capacity_from_a_rounded_zero_maximum() {
    let (mut deficit, hero) = base(HeroId::SHADOW_FIEND);
    deficit
        .world_mut()
        .stats
        .get_mut(hero)
        .expect("stats")
        .max_mana = Fixed::from_ratio(1, 2);
    deficit.world_mut().mana.get_mut(hero).expect("mana").mana = Fixed::from_ratio(1, 4);
    let mut full = deficit.clone();
    full.world_mut().mana.get_mut(hero).expect("mana").mana = Fixed::from_ratio(1, 2);
    for local in [&mut deficit, &mut full] {
        let view = snapshot(local);
        let unit = view
            .units
            .iter()
            .find(|unit| Some(unit.id) == view.players[0].unit)
            .expect("hero");
        assert_eq!((unit.mana, unit.max_mana), (1, 0));
    }
    let order = Order::Use {
        slot: ItemSlot(0),
        target: Target::None,
    };
    let unknown = Verdict::Unknown(UnknownReason::RoundedMana);
    assert_eq!(compare(deficit.clone(), order, false), unknown);
    assert_eq!(compare(full.clone(), order, false), unknown);
    let accepted = send(&mut deficit, order, 7000);
    assert!(
        !accepted
            .iter()
            .any(|msg| matches!(msg, ServerMsg::OrderRejected { .. }))
    );
    let refused = send(&mut full, order, 7000);
    assert!(refused.iter().any(|msg| matches!(
        msg,
        ServerMsg::OrderRejected {
            reason: bota_proto::RejectReason::NotReady,
            ..
        }
    )));
}

#[test]
fn order_legality_does_not_equate_a_shown_target_with_cached_hostility_sight() {
    let (mut stale, caster) = base(HeroId::SHADOW_FIEND);
    let off = OutgoingOrder {
        seq: 8000,
        unit: None,
        seen: stale.tick(),
        queue: false,
        order: Order::Cheat {
            cheat: Cheat::AllVision { on: false },
        },
    };
    let stop = OutgoingOrder {
        seq: 8000,
        unit: None,
        seen: stale.tick(),
        queue: false,
        order: Order::Move {
            target: Target::None,
        },
    };
    stale
        .step(&[&[off], &[stop]])
        .expect("normal vision and both spawn protections ended");
    let enemy = stale.hero_of(1).expect("enemy");
    let at = stale
        .world()
        .transform
        .get(caster)
        .expect("caster position")
        .pos;
    stale
        .world_mut()
        .transform
        .get_mut(enemy)
        .expect("enemy position")
        .pos = at + Vec2::from_ints(100, 0);
    // Exercise a unit-target spell without a hidden self-control capability. The server
    // validates the held slot, not the hero's catalogue kit.
    let ability = &mut stale
        .world_mut()
        .abilities
        .get_mut(caster)
        .expect("book")
        .slots[0];
    ability.id = bota_proto::AbilityId::DISMEMBER;
    ability.level = 1;
    let mut fresh = stale.clone();
    fresh.world_mut().settle();
    let stale_view = snapshot(&mut stale);
    let fresh_view = snapshot(&mut fresh);
    let target = stale_view
        .units
        .iter()
        .find(|unit| Some(unit.id) == stale_view.players[1].unit)
        .expect("shown enemy")
        .id;
    assert_eq!(
        stale_view, fresh_view,
        "the cached hostility visibility is absent from the snapshot"
    );
    let order = Order::Cast {
        slot: AbilitySlot(0),
        target: Target::Unit(target),
    };
    let before = compare(stale.clone(), order, false);
    let after = compare(fresh.clone(), order, false);
    assert!(matches!(before, Verdict::Unknown(_)));
    assert_eq!(before, after);
    let refused = send(&mut stale, order, 8001);
    assert!(refused.iter().any(|msg| matches!(
        msg,
        ServerMsg::OrderRejected {
            reason: bota_proto::RejectReason::WrongTargetKind,
            ..
        }
    )));
    let accepted = send(&mut fresh, order, 8001);
    assert!(
        !accepted
            .iter()
            .any(|msg| matches!(msg, ServerMsg::OrderRejected { .. }))
    );
}

#[test]
fn order_legality_accepts_a_scroll_swap_into_an_empty_dedicated_hero_slot() {
    let (mut local, hero) = base(HeroId::SHADOW_FIEND);
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .town_portal
        .as_mut()
        .expect("dedicated slot")
        .take();
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[5] = crate::game::ItemStack::bought(ItemId::TOWN_PORTAL_SCROLL, SlotId(0), 0);
    assert_eq!(
        compare(
            local,
            Order::Swap {
                from: ItemSlot(5),
                to: ItemSlot::TOWN_PORTAL
            },
            false
        ),
        Verdict::Legal
    );
}

#[test]
fn order_legality_rejects_a_scroll_swap_when_a_courier_has_no_dedicated_slot() {
    let (mut local, _) = base(HeroId::SHADOW_FIEND);
    let courier = local.world().seats[0].courier.expect("courier");
    local
        .world_mut()
        .inventory
        .get_mut(courier)
        .expect("bag")
        .slots[0] = crate::game::ItemStack::bought(ItemId::TOWN_PORTAL_SCROLL, SlotId(0), 0);
    let id = local.handle_of(0, courier).expect("courier handle");
    let view = snapshot(&mut local);
    let context = OrderContext {
        view: &view,
        player: &view.players[0],
        map: local.world().map,
        catalogue: Catalogue::FULL,
        cheats: true,
    };
    let order = Order::Swap {
        from: ItemSlot(0),
        to: ItemSlot::TOWN_PORTAL,
    };
    assert_eq!(
        context.check(Some(id), order, false),
        Verdict::Illegal(bota_proto::RejectReason::WrongTargetKind)
    );
    let given = OutgoingOrder {
        seq: 9000,
        unit: Some(id),
        seen: local.tick(),
        order,
        queue: false,
    };
    let stream = local.step(&[&[given], &[]]).expect("step").remove(0);
    assert!(stream.iter().any(|msg| matches!(
        msg,
        ServerMsg::OrderRejected {
            seq: 9000,
            reason: bota_proto::RejectReason::WrongTargetKind
        }
    )));
}

#[test]
fn order_legality_rejects_provably_invalid_scroll_targets_before_ground_uncertainty() {
    let (mut local, _) = base(HeroId::SHADOW_FIEND);
    let view = snapshot(&mut local);
    let own = view.players[0].unit.expect("own hero");
    let enemy = view.players[1].unit.expect("enemy hero");
    for target in [Target::None, Target::Unit(own), Target::Unit(enemy)] {
        let order = Order::Use {
            slot: ItemSlot::TOWN_PORTAL,
            target,
        };
        let mut probe = local.clone();
        let refused = send(&mut probe, order, 10000);
        assert!(refused.iter().any(|msg| matches!(
            msg,
            ServerMsg::OrderRejected {
                reason: bota_proto::RejectReason::WrongTargetKind,
                ..
            }
        )));
        assert_eq!(
            compare(local.clone(), order, false),
            Verdict::Illegal(bota_proto::RejectReason::WrongTargetKind)
        );
    }
}

#[test]
fn order_legality_does_not_apply_the_hero_talent_view_to_a_named_courier() {
    let (mut local, _) = base(HeroId::SHADOW_FIEND);
    let view = snapshot(&mut local);
    let talents = view
        .talents
        .iter()
        .find(|talents| talents.slot == SlotId(0))
        .expect("hero talents");
    assert!(talents.may_learn.expect("own facts").has(TalentSlot(0)));
    assert_eq!(talents.may_learn_attribute_bonus, Some(true));
    let courier = local.world().seats[0].courier.expect("courier");
    let id = local.handle_of(0, courier).expect("handle");
    for order in [
        Order::LearnTalent {
            talent: TalentSlot(0),
        },
        Order::LearnAttributeBonus,
    ] {
        assert_eq!(compare(local.clone(), order, false), Verdict::Legal);
        assert_eq!(
            compare_for(local.clone(), Some(id), order, false),
            Verdict::Unknown(UnknownReason::HiddenState)
        );
    }
}

#[test]
fn order_legality_uses_the_seat_death_for_courier_stash_disassembly() {
    let (mut local, hero) = base(HeroId::SHADOW_FIEND);
    let courier = local.world().seats[0].courier.expect("courier");
    let id = local.handle_of(0, courier).expect("handle");
    let teleport = OutgoingOrder {
        seq: 100,
        unit: Some(id),
        seen: local.tick(),
        queue: false,
        order: Order::Cheat {
            cheat: Cheat::Teleport {
                at: Vec2::from_ints(9000, 9000),
            },
        },
    };
    let stream = local.step(&[&[teleport], &[]]).expect("teleport").remove(0);
    assert!(
        !stream
            .iter()
            .any(|msg| matches!(msg, ServerMsg::OrderRejected { .. }))
    );
    assert!(!local.world().at_shop(courier));
    let item = bota_core::ITEMS
        .iter()
        .find(|def| {
            !def.retired
                && !def.unbuyable
                && !def.components.is_empty()
                && def.disassemble != bota_core::DisassembleRule::Never
        })
        .expect("disassemblable item")
        .id;
    let mut stack = crate::game::ItemStack::bought(item, SlotId(0), local.tick()).expect("item");
    stack.disassemble_until = local.tick() + bota_core::rules::DISASSEMBLY_WINDOW_TICKS;
    local.world_mut().seats[0].stash.slots[0] = Some(stack);
    let order = Order::Disassemble {
        slot: ItemSlot(
            (bota_core::rules::INVENTORY_SLOTS + bota_core::rules::BACKPACK_SLOTS) as u8,
        ),
    };
    assert_eq!(
        compare_for(local.clone(), Some(id), order, false),
        Verdict::Illegal(bota_proto::RejectReason::NotAtShop)
    );
    local.world_mut().kill_by_hp_removal(hero, hero);
    local.world_mut().seats[0].stash.slots[0]
        .as_mut()
        .expect("item")
        .disassemble_until = local.tick() + bota_core::rules::DISASSEMBLY_WINDOW_TICKS;
    let view = snapshot(&mut local);
    assert!(view.players[0].respawn_left > 0);
    assert!(
        view.players[0].stash.as_ref().expect("stash")[0]
            .as_ref()
            .expect("item")
            .disassembly
            .is_some()
    );
    assert_eq!(
        compare_for(local.clone(), Some(id), order, false),
        Verdict::Legal
    );
    let given = OutgoingOrder {
        seq: 1001,
        unit: Some(id),
        order,
        queue: false,
        seen: local.tick(),
    };
    let stream = local.step(&[&[given], &[]]).expect("step").remove(0);
    assert!(
        !stream
            .iter()
            .any(|msg| matches!(msg, ServerMsg::OrderRejected { .. }))
    );
    assert!(
        local.world().seats[0]
            .stash
            .slots
            .iter()
            .flatten()
            .all(|stack| stack.id != item)
    );
}

#[test]
fn order_legality_rejects_known_bad_cheats_even_when_created_seat_status_is_hidden() {
    let (local, _) = base(HeroId::SHADOW_FIEND);
    let mut spec = bota_proto::ModifierSpec::NOMINAL;
    spec.mana_cost_rate = bota_proto::ModifierSpec::MAX_SCALE + 1;
    for cheat in [
        Cheat::ApplyModifier {
            target: Target::None,
            spec,
            ticks: 1,
        },
        Cheat::ApplyModifier {
            target: Target::None,
            spec: bota_proto::ModifierSpec::NOMINAL,
            ticks: 0,
        },
        Cheat::Item {
            item: ItemId(u16::MAX),
        },
    ] {
        let order = Order::Cheat { cheat };
        let mut probe = local.clone();
        let stream = send(&mut probe, order, 12000);
        assert!(stream.iter().any(|msg| matches!(
            msg,
            ServerMsg::OrderRejected {
                reason: bota_proto::RejectReason::BadCheat,
                ..
            }
        )));
        assert_eq!(
            compare(local.clone(), order, false),
            Verdict::Illegal(bota_proto::RejectReason::BadCheat)
        );
    }
}

fn put_after_cast_can_be_refused(mut local: LocalMatch, slot: ItemSlot) {
    let view = snapshot(&mut local);
    let context = OrderContext {
        view: &view,
        player: &view.players[0],
        map: local.world().map,
        catalogue: Catalogue::FULL,
        cheats: true,
    };
    let order = Order::Put {
        slot,
        target: Target::None,
    };
    let verdict = context.check(None, order, false);
    assert!(
        local
            .world()
            .validate_order(SlotId(0), None, &order)
            .is_ok()
    );
    assert_eq!(
        context.check_with_ledger(
            None,
            order,
            false,
            &bota_client_core::order_legality::pseudo_queue::OwedOrders::default()
        ),
        Verdict::Unknown(UnknownReason::QueueState),
        "room in a ledger cannot prove take-up acceptance"
    );
    let mut stream = send(&mut local, order, 13000);
    let mut refused = false;
    for _ in 0..64 {
        refused = stream.iter().any(|message| {
            matches!(
                message,
                ServerMsg::OrderRejected {
                    seq: 13000,
                    reason: bota_proto::RejectReason::EmptySlot
                }
            )
        });
        if refused {
            break;
        }
        stream = local.step(&[&[], &[]]).expect("cast completes").remove(0);
    }
    assert!(
        refused,
        "the held item disappears before the queued put is taken up"
    );
    assert_eq!(verdict, Verdict::Unknown(UnknownReason::QueueState));
}

#[test]
fn order_legality_defers_a_put_behind_the_last_mango_use() {
    let (mut local, hero) = base_on(HeroId::PUDGE, bota_proto::MapId(0));
    let shop = local.world().map.secret_shops[0].pos;
    local.world_mut().transform.get_mut(hero).expect("hero").pos = shop;
    local.world_mut().mana.get_mut(hero).expect("mana").mana = Fixed::from_int(75);
    local
        .world_mut()
        .stats
        .get_mut(hero)
        .expect("stats")
        .max_mana = Fixed::from_int(900);
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[0]
        .as_mut()
        .expect("mango")
        .charges = 1;
    local
        .world_mut()
        .action
        .get_mut(hero)
        .expect("action")
        .state = ActionState::UseItem {
        slot: ItemSlot(0),
        target: Target::None,
        phase: ActionPhase::Before { progress: 0 },
    };
    put_after_cast_can_be_refused(local, ItemSlot(0));
}

#[test]
fn order_legality_defers_a_put_that_can_be_sold_during_the_cast_point() {
    let (mut local, hero) = base(HeroId::SHADOW_FIEND);
    let mut item = crate::game::ItemStack::bought(ItemId::IRON_BRANCH, SlotId(0), local.tick())
        .expect("branch");
    item.for_sale = true;
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[7] = Some(item);
    local
        .world_mut()
        .action
        .get_mut(hero)
        .expect("action")
        .state = ActionState::CastAbility {
        slot: AbilitySlot(2),
        target: Target::None,
        phase: ActionPhase::Before { progress: 1 },
    };
    put_after_cast_can_be_refused(local, ItemSlot(7));
}

#[test]
fn order_legality_refuses_sale_of_another_seats_item() {
    let (mut local, hero) = base(HeroId::SHADOW_FIEND);
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[0] = crate::game::ItemStack::bought(ItemId::IRON_BRANCH, SlotId(1), 0);
    assert_eq!(
        compare(local, Order::Sell { slot: ItemSlot(0) }, false),
        Verdict::Illegal(bota_proto::RejectReason::NotYourItem)
    );
}

#[test]
fn order_legality_shift_queue_accepts_the_last_place_but_refuses_overflow() {
    let (local, hero) = base(HeroId::SHADOW_FIEND);
    for (count, expected) in [
        (31, Verdict::Legal),
        (32, Verdict::Illegal(bota_proto::RejectReason::QueueFull)),
    ] {
        let mut probe = local.clone();
        let mut queue = crate::game::OrderQueue::default();
        for seq in 0..count {
            assert!(queue.push(crate::game::Queued {
                order: Order::Move {
                    target: Target::Pos(Vec2::from_ints(7000, 7000))
                },
                seq
            }));
        }
        probe.world_mut().queue.insert(hero, queue);
        assert_eq!(
            compare(
                probe,
                Order::Move {
                    target: Target::None
                },
                true
            ),
            expected
        );
    }
}

#[test]
fn order_legality_glyph_is_ready_at_zero_but_not_one_tick() {
    let (local, _) = base(HeroId::SHADOW_FIEND);
    for (left, expected) in [
        (0, Verdict::Legal),
        (1, Verdict::Illegal(bota_proto::RejectReason::OnCooldown)),
    ] {
        let mut probe = local.clone();
        probe.world_mut().glyphs[0].ready_at = probe.tick() + left;
        assert_eq!(compare(probe, Order::Glyph, false), expected);
    }
}

#[test]
fn order_legality_bad_slot_overrides_uncertain_portal_destination() {
    let (mut local, hero) = base(HeroId::SHADOW_FIEND);
    // A portal's destination is uncertain, but its backpack slot is certainly unusable.
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[7] = crate::game::ItemStack::bought(ItemId::TOWN_PORTAL_SCROLL, SlotId(0), 0);
    assert_eq!(
        compare(
            local,
            Order::Use {
                slot: ItemSlot(7),
                target: Target::None
            },
            false
        ),
        Verdict::Illegal(bota_proto::RejectReason::WrongTargetKind)
    );
}

#[test]
fn order_legality_a_courier_cannot_take_a_rune_but_a_hero_can() {
    let (mut local, _) = base_on(HeroId::SHADOW_FIEND, bota_proto::MapId(0));
    let at = local.world().map.rune_spots[3].at;
    send(
        &mut local,
        Order::Cheat {
            cheat: Cheat::Teleport {
                at: at + Vec2::from_ints(400, 0),
            },
        },
        14999,
    );
    send(
        &mut local,
        Order::Cheat {
            cheat: Cheat::SpawnRune {
                kind: bota_proto::RuneKind::Haste,
                at,
            },
        },
        15000,
    );
    let view = snapshot(&mut local);
    let rune = view.runes.first().expect("spawned rune").id;
    let courier = local.world().seats[0].courier.expect("courier");
    let id = local.handle_of(0, courier).expect("handle");
    let order = Order::Take {
        target: Target::Unit(rune),
    };
    assert_eq!(
        compare_for(local.clone(), Some(id), order, false),
        Verdict::Illegal(bota_proto::RejectReason::WrongTargetKind)
    );
    assert_eq!(compare(local, order, false), Verdict::Legal);
}

#[test]
fn order_legality_one_tick_cooldown_still_refuses_a_cast() {
    let (mut local, hero) = base(HeroId::SHADOW_FIEND);
    local
        .world_mut()
        .abilities
        .get_mut(hero)
        .expect("book")
        .slots[0]
        .cooldown = 1;
    assert_eq!(
        compare(
            local,
            Order::Cast {
                slot: AbilitySlot(0),
                target: Target::None
            },
            false
        ),
        Verdict::Illegal(bota_proto::RejectReason::OnCooldown)
    );
}

#[test]
fn order_legality_all_body_orders_that_wait_for_a_cast_keep_take_up_uncertain() {
    let (mut local, hero) = base(HeroId::SHADOW_FIEND);
    let at = local.world().map.fountains[0];
    let branch = crate::game::ItemStack::bought(ItemId::IRON_BRANCH, SlotId(0), local.tick())
        .expect("branch");
    local.world_mut().lay_loot(branch, at);
    let loot = snapshot(&mut local).loot[0].id;
    let courier = local.world().seats[0].courier.expect("courier");
    let courier = local.handle_of(0, courier).expect("courier handle");
    arrange(&mut local, hero, 12);
    for order in [
        Order::Put {
            slot: ItemSlot(0),
            target: Target::None,
        },
        Order::Put {
            slot: ItemSlot(0),
            target: Target::Pos(at),
        },
        Order::Take {
            target: Target::Unit(loot),
        },
        Order::Move {
            target: Target::Unit(courier),
        },
        Order::Attack {
            target: Target::Unit(courier),
        },
        Order::Move {
            target: Target::Pos(at),
        },
        Order::MoveDirection { toward: at },
    ] {
        assert_eq!(
            compare(local.clone(), order, false),
            Verdict::Unknown(UnknownReason::QueueState),
            "{order:?}"
        );
    }
    assert_eq!(
        compare(
            local,
            Order::Move {
                target: Target::None
            },
            false
        ),
        Verdict::Legal,
        "a halt interrupts instead of waiting"
    );
}

#[test]
fn order_legality_take_loot_behind_a_cast_is_unknown_when_another_carrier_can_take_it_first() {
    let (mut local, hero) = base(HeroId::SHADOW_FIEND);
    let at = local.world().map.fountains[0];
    let branch = crate::game::ItemStack::bought(ItemId::IRON_BRANCH, SlotId(0), local.tick())
        .expect("branch");
    let physical_loot = local.world_mut().lay_loot(branch, at);
    let loot = snapshot(&mut local).loot[0].id;
    let courier = local.world().seats[0].courier.expect("courier");
    local
        .world_mut()
        .transform
        .get_mut(courier)
        .expect("courier position")
        .pos = at;
    let courier = local.handle_of(0, courier).expect("courier handle");
    arrange(&mut local, hero, 12);
    let view = snapshot(&mut local);
    let order = Order::Take {
        target: Target::Unit(loot),
    };
    assert_eq!(
        local.world().validate_order(
            SlotId(0),
            None,
            &Order::Take {
                target: Target::Unit(crate::game::wire_id(physical_loot))
            }
        ),
        Ok(())
    );
    let context = OrderContext {
        view: &view,
        player: &view.players[0],
        map: local.world().map,
        catalogue: Catalogue::FULL,
        cheats: true,
    };
    assert_eq!(
        context.check(None, order, false),
        Verdict::Unknown(UnknownReason::QueueState)
    );
    let stream = send(&mut local, order, 16000);
    assert!(
        !stream
            .iter()
            .any(|message| matches!(message, ServerMsg::OrderRejected { seq: 16000, .. }))
    );
    let given = OutgoingOrder {
        seq: 16001,
        unit: Some(courier),
        order,
        queue: false,
        seen: local.tick(),
    };
    let mut stream = local
        .step(&[&[given], &[]])
        .expect("courier takes loot")
        .remove(0);
    assert!(
        !snapshot(&mut local).loot.iter().any(|item| item.id == loot),
        "the other carrier took the target before the hero's cast ended"
    );
    let mut refused = false;
    for _ in 0..64 {
        refused = stream.iter().any(|message| {
            matches!(
                message,
                ServerMsg::OrderRejected {
                    seq: 16000,
                    reason: bota_proto::RejectReason::UnknownTarget
                }
            )
        });
        if refused {
            break;
        }
        stream = local.step(&[&[], &[]]).expect("cast completes").remove(0);
    }
    assert!(
        refused,
        "admitted now, rejected on take-up after the loot disappears"
    );
}

/// The client's verdict on a buyback and the server's answer to the same order, which must be
/// the same thing: legal and carried out, or refused for the reason the client names.
fn buyback_verdict(mut local: LocalMatch, unit: Option<EntityId>) -> Verdict {
    let view = snapshot(&mut local);
    let player = view.player(SlotId(0)).expect("player");
    let verdict = OrderContext {
        view: &view,
        player,
        map: local.world().map,
        catalogue: Catalogue::FULL,
        cheats: true,
    }
    .check(unit, Order::Buyback, false);
    let given = OutgoingOrder {
        seq: 1000,
        unit,
        order: Order::Buyback,
        queue: false,
        seen: local.tick(),
    };
    let stream = local.step(&[&[given], &[]]).expect("step").remove(0);
    let refusal = stream.iter().find_map(|msg| match msg {
        ServerMsg::OrderRejected { seq: 1000, reason } => Some(*reason),
        _ => None,
    });
    let served = refusal.map_or(Verdict::Legal, Verdict::Illegal);
    assert_eq!(verdict, served, "the client's verdict against the server's");
    verdict
}

#[test]
fn order_legality_buyback_is_proven_from_the_seats_own_row_in_every_state() {
    use bota_proto::RejectReason;
    let (mut local, _) = base_on(HeroId::SHADOW_FIEND, bota_core::DEMO_ID);
    let mut seq = 5000;
    let mut cheat = |local: &mut LocalMatch, cheat: Cheat| {
        seq += 1;
        let stream = send(local, Order::Cheat { cheat }, seq);
        assert!(
            !stream
                .iter()
                .any(|msg| matches!(msg, ServerMsg::OrderRejected { .. })),
            "{cheat:?}"
        );
    };
    cheat(&mut local, Cheat::Gold { amount: 5_000 });
    assert_eq!(
        buyback_verdict(local.clone(), None),
        Verdict::Illegal(RejectReason::HeroAlive)
    );
    let mut rising = local.clone();
    cheat(
        &mut rising,
        Cheat::Item {
            item: ItemId::AEGIS,
        },
    );
    cheat(&mut rising, Cheat::Suicide);
    assert_eq!(
        buyback_verdict(rising, None),
        Verdict::Illegal(RejectReason::HeroAlive),
        "rising by the Aegis"
    );
    let courier = {
        let entity = local.world().seats[0].courier.expect("a courier");
        local.handle_of(0, entity).expect("told of it")
    };
    cheat(&mut local, Cheat::Suicide);
    assert_eq!(buyback_verdict(local.clone(), None), Verdict::Legal);
    assert_eq!(
        buyback_verdict(local.clone(), Some(courier)),
        Verdict::Illegal(RejectReason::NotAHero)
    );
    let mut poor = local.clone();
    cheat(&mut poor, Cheat::Gold { amount: -1_000_000 });
    assert_eq!(
        buyback_verdict(poor, None),
        Verdict::Illegal(RejectReason::NotEnoughGold)
    );
    send(&mut local, Order::Buyback, 9000);
    cheat(&mut local, Cheat::Suicide);
    assert_eq!(
        buyback_verdict(local, None),
        Verdict::Illegal(RejectReason::OnCooldown)
    );
}
