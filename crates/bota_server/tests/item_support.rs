//! Support items through delayed orders and the snapshots a match publishes.

use bota_core::rules;
use bota_proto::{
    Cheat, HeroId, ItemId, MapId, Order, OutgoingOrder, Pick, ServerMsg, SlotId, Team, TickMode,
    Vec2, WorldView,
};
use bota_server::{
    game::{MatchConfig, OrderDelay},
    local::LocalMatch,
};

struct Match {
    game: LocalMatch,
    views: Vec<WorldView>,
    facts: Vec<Vec<bota_proto::EventKind>>,
}

impl Match {
    fn new() -> Self {
        Self::with_immortality(true)
    }

    fn with_immortality(immortal_heroes: bool) -> Self {
        Self::with_options(immortal_heroes, bota_proto::RuneSet::NONE)
    }

    fn with_options(immortal_heroes: bool, runes: bota_proto::RuneSet) -> Self {
        let map = bota_core::map_of(MapId(2));
        let config = MatchConfig {
            match_id: 1,
            master_key: [3; 32],
            stream: 1,
            picks: [Team::Radiant, Team::Radiant, Team::Dire]
                .into_iter()
                .enumerate()
                .map(|(index, team)| Pick {
                    slot: SlotId(index as u8),
                    team,
                    hero: HeroId::SHADOW_FIEND,
                })
                .collect(),
            map: map.id,
            mode: TickMode::Lockstep,
            ack_timeout_ticks: 150,
            cheats: true,
            spawn_modifiers: Vec::new(),
            immortal_structures: true,
            immortal_heroes,
            starting_gold_lost: Vec::new(),
            objectives: map.default_objectives,
            runes,
            catalogue: bota_core::Catalogue::FULL,
            economy_events: Vec::new(),
        };
        let (game, streams) = LocalMatch::new(&config, &[OrderDelay::NONE; 3]).expect("match");
        let mut result = Self {
            game,
            views: snapshots(&streams),
            facts: event_facts(&streams),
        };
        result.cheat(0, Cheat::CreepSpawning { on: false });
        result.cheat(0, Cheat::AllVision { on: true });
        let positions = [
            Vec2::from_ints(8000, 5000),
            Vec2::from_ints(7800, 5000),
            Vec2::from_ints(8600, 5000),
        ];
        for (seat, at) in positions.into_iter().enumerate() {
            result.cheat(seat, Cheat::Teleport { at });
            result.order(
                seat,
                Order::Move {
                    target: bota_proto::Target::None,
                },
            );
        }
        result.advance(rules::FOUNTAIN_HEAL_LINGER_TICKS + 1);
        for seat in 0..3 {
            assert!(
                result.hero(seat).effects.is_empty(),
                "fixture is outside building auras"
            );
        }
        result
    }

    fn order(&mut self, seat: usize, order: Order) {
        assert_eq!(self.try_order(seat, order), Vec::new(), "{order:?}");
    }

    fn try_order(&mut self, seat: usize, order: Order) -> Vec<bota_proto::RejectReason> {
        self.try_order_for(seat, None, order)
    }

    fn try_order_for(
        &mut self,
        seat: usize,
        unit: Option<bota_proto::EntityId>,
        order: Order,
    ) -> Vec<bota_proto::RejectReason> {
        let tick = self.game.tick();
        let outgoing = [OutgoingOrder {
            seq: tick,
            seen: tick,
            unit,
            order,
            queue: false,
        }];
        let mut orders: [&[OutgoingOrder]; 3] = [&[], &[], &[]];
        orders[seat] = &outgoing;
        let streams = self.game.step(&orders).expect("order step");
        self.views = snapshots(&streams);
        self.facts = event_facts(&streams);
        streams[seat]
            .iter()
            .filter_map(|message| match message {
                ServerMsg::OrderRejected { reason, .. } => Some(*reason),
                _ => None,
            })
            .collect()
    }

    fn cheat(&mut self, seat: usize, cheat: Cheat) {
        self.order(seat, Order::Cheat { cheat });
    }

    fn advance(&mut self, ticks: u32) {
        for _ in 0..ticks {
            let streams = self.game.step(&[&[], &[], &[]]).expect("tick");
            self.views = snapshots(&streams);
            self.facts = event_facts(&streams);
        }
    }

    fn order_for(&mut self, seat: usize, unit: bota_proto::EntityId, order: Order) {
        assert_eq!(
            self.try_order_for(seat, Some(unit), order),
            Vec::new(),
            "{order:?}"
        );
    }

    fn take_cheated_rune(&mut self, kind: bota_proto::RuneKind) {
        let at = bota_core::map_of(MapId(2)).rune_spots[0].at;
        self.cheat(0, Cheat::Teleport { at });
        self.order(
            0,
            Order::Move {
                target: bota_proto::Target::None,
            },
        );
        self.cheat(0, Cheat::SpawnRune { kind, at });
        if self.rune_activated(kind) {
            return;
        }
        self.advance(1);
        if self.rune_activated(kind) {
            return;
        }
        let rune = *self.views[0]
            .runes
            .iter()
            .find(|rune| rune.kind == kind)
            .expect("cheated rune");
        self.order(
            0,
            Order::Take {
                target: bota_proto::Target::Unit(rune.id),
            },
        );
        for _ in 0..30 {
            if self.rune_activated(kind) {
                return;
            }
            self.advance(1);
        }
        panic!("rune was not activated from its cheated position");
    }

    fn rune_activated(&self, kind: bota_proto::RuneKind) -> bool {
        let hero = self.hero(0).id;
        self.facts[0].iter().any(|event| matches!(event, bota_proto::EventKind::RuneActivated { unit, kind: activated, .. } if *unit == hero && *activated == kind))
    }

    fn item_active(&self, seat: usize, slot: u8) -> Option<bool> {
        let hero = self.game.hero_of(seat)?;
        let stack = self
            .game
            .world()
            .inventory
            .get(hero)?
            .slots
            .get(usize::from(slot))?
            .as_ref()?;
        Some(stack.active && stack.passive_wait == 0)
    }

    fn hero(&self, seat: usize) -> &bota_proto::UnitView {
        let hero = self.game.hero_of(seat).expect("hero");
        let handle = self.game.handle_of(seat, hero).expect("own handle");
        self.views[seat].unit(handle).expect("own hero shown")
    }
}

fn event_facts(streams: &[Vec<ServerMsg>]) -> Vec<Vec<bota_proto::EventKind>> {
    streams
        .iter()
        .map(|stream| {
            stream
                .iter()
                .find_map(|message| match message {
                    ServerMsg::Events { events, .. } => Some(events.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        })
        .collect()
}

fn snapshots(streams: &[Vec<ServerMsg>]) -> Vec<WorldView> {
    streams
        .iter()
        .map(|stream| {
            stream
                .iter()
                .find_map(|message| match message {
                    ServerMsg::Snapshot { view } => Some(view.clone()),
                    _ => None,
                })
                .expect("snapshot each step")
        })
        .collect()
}

#[test]
fn a_cheated_headdress_emits_regeneration_to_allies_and_lingers_after_they_leave() {
    let mut game = Match::new();
    let before = game.hero(1).hp_regen;
    let enemy_before = game.hero(2).hp_regen;
    game.cheat(
        0,
        Cheat::Item {
            item: item("item_headdress"),
        },
    );
    game.advance(1);
    assert!(
        game.hero(1).hp_regen > before,
        "the ally sees the aura's regeneration"
    );
    assert_eq!(
        game.hero(2).hp_regen,
        enemy_before,
        "the enemy receives none"
    );
    game.cheat(
        1,
        Cheat::Teleport {
            at: Vec2::from_ints(11000, 5000),
        },
    );
    assert!(game.hero(1).hp_regen > before, "the aura lingers");
    game.advance(30);
    assert_eq!(
        game.hero(1).hp_regen,
        before,
        "the aura expires out of range; hero={:?}",
        game.hero(1)
    );
}

#[test]
fn a_pool_cheat_sets_missing_health_and_mana_without_playing_the_match() {
    let mut game = Match::new();
    game.cheat(
        1,
        Cheat::SetPools {
            health: 25,
            mana: 10,
        },
    );
    let hero = game.hero(1);
    assert!(
        (hero.hp - hero.max_hp / 4).abs() <= 1,
        "health is a quarter full: {hero:?}"
    );
    assert!(
        (hero.mana - hero.max_mana / 10).abs() <= 1,
        "mana is a tenth full"
    );
    assert_eq!(
        game.hero(0).hp,
        game.hero(0).max_hp,
        "another seat's pools are untouched"
    );
}

#[test]
fn a_pool_cheat_rejects_out_of_range_percentages_with_bad_cheat() {
    let mut game = Match::new();
    for (health, mana) in [(0, 50), (101, 50), (50, 101), (255, 255)] {
        assert_eq!(
            game.try_order(
                0,
                Order::Cheat {
                    cheat: Cheat::SetPools { health, mana }
                }
            ),
            vec![bota_proto::RejectReason::BadCheat]
        );
        assert_eq!(game.hero(0).hp, game.hero(0).max_hp);
        assert_eq!(game.hero(0).mana, game.hero(0).max_mana);
    }
}

#[test]
fn a_pool_cheat_in_a_match_without_cheats_is_refused_without_changing_pools() {
    let mut game = Match::new();
    game.game.world_mut().cheats = false;
    assert_eq!(
        game.try_order(
            0,
            Order::Cheat {
                cheat: Cheat::SetPools {
                    health: 25,
                    mana: 10
                }
            }
        ),
        vec![bota_proto::RejectReason::NoCheats]
    );
    assert_eq!(game.hero(0).hp, game.hero(0).max_hp);
    assert_eq!(game.hero(0).mana, game.hero(0).max_mana);
}

fn item(key: &str) -> ItemId {
    bota_core::ITEMS
        .iter()
        .find(|def| def.key == key)
        .unwrap_or_else(|| panic!("missing item {key}"))
        .id
}

fn source(key: &str, value: &str) -> bota_core::Dec {
    bota_core::item_def(item(key))
        .expect("catalogue row")
        .value(value)
        .unwrap_or_else(|| panic!("missing {key}.{value}"))[0]
}

fn give(game: &mut Match, seat: usize, key: &str) {
    game.cheat(seat, Cheat::Item { item: item(key) });
    game.advance(1);
}

fn use_first(game: &mut Match, seat: usize) {
    game.order(
        seat,
        Order::Use {
            slot: bota_proto::ItemSlot(0),
            target: bota_proto::Target::None,
        },
    );
}

#[test]
fn basilius_shares_mana_regeneration_and_duplicates_do_not_stack() {
    let mut game = Match::new();
    let before = game.hero(1).mana_regen;
    let enemy = game.hero(2).mana_regen;
    give(&mut game, 0, "item_ring_of_basilius");
    let shared = game.hero(1).mana_regen;
    let expected = bota_proto::Fixed::from_ratio(
        source("item_ring_of_basilius", "aura_mana_regen").hundredths(),
        100,
    );
    assert!((shared.raw - before.raw - expected.raw).abs() < 64);
    give(&mut game, 0, "item_ring_of_basilius");
    assert_eq!(game.hero(1).mana_regen, shared);
    assert_eq!(game.hero(2).mana_regen, enemy);
}

#[test]
fn buckler_shares_armor_with_combatants_without_buffing_enemies() {
    let mut game = Match::new();
    let before = game.hero(1).armor;
    let enemy = game.hero(2).armor;
    give(&mut game, 0, "item_buckler");
    let expected = bota_proto::Fixed::from_int(source("item_buckler", "bonus_aoe_armor").int());
    assert_eq!(game.hero(1).armor - before, expected);
    assert_eq!(game.hero(2).armor, enemy);
    game.order(
        0,
        Order::Swap {
            from: bota_proto::ItemSlot(0),
            to: bota_proto::ItemSlot(6),
        },
    );
    game.advance(rules::AURA_LINGER_TICKS + 1);
    assert_eq!(game.hero(1).armor, before, "backpack items emit nothing");
}

#[test]
fn mekansm_restores_allies_and_honors_the_shared_recovery_lockout() {
    let mut game = Match::new();
    for seat in 0..2 {
        give(&mut game, seat, "item_mekansm");
    }
    game.cheat(
        1,
        Cheat::SetPools {
            health: 25,
            mana: 100,
        },
    );
    game.cheat(
        2,
        Cheat::SetPools {
            health: 25,
            mana: 100,
        },
    );
    let before = game.hero(1).hp;
    let enemy = game.hero(2).hp;
    use_first(&mut game, 0);
    let restored = game.hero(1).hp - before;
    let expected = source("item_mekansm", "heal_amount").int();
    assert!(
        (expected..=expected + 2).contains(&restored),
        "{restored} instead of {expected}"
    );
    assert!(game.hero(2).hp - enemy <= 1, "an enemy is not healed");
    game.cheat(
        1,
        Cheat::SetPools {
            health: 25,
            mana: 100,
        },
    );
    let before = game.hero(1).hp;
    use_first(&mut game, 1);
    assert!(
        game.hero(1).hp - before <= 1,
        "a different caster cannot bypass the receiver's lockout"
    );
}

#[test]
fn arcane_boots_restore_nearby_allies_mana_and_retain_boots_rules() {
    let mut game = Match::new();
    give(&mut game, 0, "item_arcane_boots");
    for seat in 0..3 {
        game.cheat(
            seat,
            Cheat::SetPools {
                health: 100,
                mana: 0,
            },
        );
    }
    let before = game.hero(1).mana;
    let enemy = game.hero(2).mana;
    use_first(&mut game, 0);
    let restored = game.hero(1).mana - before;
    let expected = source("item_arcane_boots", "replenish_amount").int();
    // Nonzero pools smaller than one are shown as one; integer-view deltas may lose a point.
    assert!(
        (expected - 1..=expected + 1).contains(&restored),
        "restored {restored}, expected {expected}; hero={:?}",
        game.hero(1)
    );
    assert!(game.hero(2).mana - enemy <= 1);
    let speed = game.hero(0).move_speed;
    give(&mut game, 0, "item_boots");
    assert_eq!(game.hero(0).move_speed, speed, "boots do not add together");
}

#[test]
fn drums_grant_endurance_without_charges_and_expire_at_the_script_duration() {
    let mut game = Match::new();
    give(&mut game, 0, "item_ancient_janggo");
    let speed = game.hero(1).move_speed;
    let attack = game.hero(1).attack_speed;
    assert_eq!(
        game.hero(0).items[0].as_ref().expect("drum").charges,
        None,
        "charges were removed; the old tooltip value is inert"
    );
    use_first(&mut game, 0);
    assert_eq!(
        game.hero(1).attack_speed - attack,
        source("item_ancient_janggo", "bonus_attack_speed_pct").int()
    );
    assert!(game.hero(1).move_speed > speed);
    game.advance(source("item_ancient_janggo", "duration").secs().ticks() + 1);
    assert_eq!(game.hero(1).attack_speed, attack);
    assert_eq!(game.hero(1).move_speed, speed);
}

#[test]
fn pipe_grants_magic_resistance_and_a_barrier_with_a_receiver_lockout() {
    let mut game = Match::new();
    let resistance = game.hero(1).magic_resist;
    for seat in 0..2 {
        give(&mut game, seat, "item_pipe");
    }
    assert!(game.hero(1).magic_resist > resistance);
    use_first(&mut game, 0);
    let barrier = game
        .hero(1)
        .effects
        .iter()
        .find(|effect| {
            effect.inflictor == Some(bota_proto::Inflictor::Item(item("item_pipe")))
                && effect.stacks.is_some()
        })
        .cloned()
        .expect("barrier capacity is shown");
    assert_eq!(
        barrier.stacks,
        Some(source("item_pipe", "barrier_block").int() as u32)
    );
    let left = barrier.ticks_left.expect("barrier expires");
    use_first(&mut game, 1);
    let after = game
        .hero(1)
        .effects
        .iter()
        .find(|effect| effect.id == barrier.id)
        .expect("old barrier remains");
    assert!(
        after.ticks_left.expect("timed") < left,
        "a second pipe does not refresh the barrier inside the receiver lockout"
    );
}

#[test]
fn greaves_boost_only_the_low_health_wearer_and_restore_allied_pools() {
    let mut game = Match::new();
    give(&mut game, 0, "item_guardian_greaves");
    for seat in 0..2 {
        game.cheat(
            seat,
            Cheat::SetPools {
                health: 10,
                mana: 0,
            },
        );
    }
    game.advance(1);
    assert!(
        game.hero(0).hp_regen > game.hero(1).hp_regen,
        "the threshold boost belongs to the wearer"
    );
    let before = game.hero(1).hp;
    let mana = game.hero(1).mana;
    use_first(&mut game, 0);
    assert!(game.hero(1).hp - before >= source("item_guardian_greaves", "replenish_health").int());
    assert!(
        game.hero(1).mana - mana >= source("item_guardian_greaves", "replenish_mana").int() - 1
    );
}

#[test]
fn assault_emits_distinct_allied_and_enemy_armor_auras() {
    let mut game = Match::new();
    let allied_armor = game.hero(1).armor;
    let allied_attack = game.hero(1).attack_speed;
    let enemy_armor = game.hero(2).armor;
    give(&mut game, 0, "item_assault");
    assert_eq!(
        game.hero(1).armor - allied_armor,
        bota_proto::Fixed::from_int(source("item_assault", "aura_positive_armor").int())
    );
    assert_eq!(
        game.hero(1).attack_speed - allied_attack,
        source("item_assault", "aura_attack_speed").int()
    );
    assert_eq!(
        game.hero(2).armor - enemy_armor,
        bota_proto::Fixed::from_int(source("item_assault", "aura_negative_armor").int())
    );
}

#[test]
fn vladmir_shares_damage_armor_regeneration_and_attack_lifesteal() {
    let mut game = Match::new();
    let before = game.hero(1).clone();
    give(&mut game, 0, "item_vladmir");
    assert!(game.hero(1).attack_damage > before.attack_damage);
    assert!(game.hero(1).armor > before.armor);
    assert!(game.hero(1).mana_regen > before.mana_regen);
    game.cheat(
        1,
        Cheat::SetPools {
            health: 25,
            mana: 100,
        },
    );
    game.cheat(
        1,
        Cheat::Teleport {
            at: Vec2::from_ints(8350, 5000),
        },
    );
    let hp = game.hero(1).hp;
    let regeneration = game.hero(1).hp_regen.to_int() + 1;
    let target = game
        .game
        .handle_of(1, game.game.hero_of(2).expect("enemy"))
        .expect("visible enemy");
    game.order(
        1,
        Order::Attack {
            target: bota_proto::Target::Unit(target),
        },
    );
    game.advance(30);
    assert!(
        game.hero(1).hp > hp + regeneration + 1,
        "a physical attack supplies healing beyond ordinary regeneration"
    );
}

#[test]
fn radiance_toggle_controls_nonstacking_burn_and_preserves_carried_evasion() {
    let mut game = Match::new();
    give(&mut game, 0, "item_radiance");
    let evasion = game.hero(0).evasion;
    assert_eq!(game.item_active(0, 0), Some(true));
    game.advance(rules::TICKS_PER_SECOND + 1);
    let single_loss = game.hero(2).max_hp - game.hero(2).hp;
    assert!(single_loss > 0, "an enemy in the burn loses health");
    give(&mut game, 1, "item_radiance");
    game.cheat(
        2,
        Cheat::SetPools {
            health: 100,
            mana: 100,
        },
    );
    game.advance(rules::TICKS_PER_SECOND);
    assert_eq!(
        game.hero(2).max_hp - game.hero(2).hp,
        single_loss,
        "two carriers do not double the burn"
    );
    use_first(&mut game, 0);
    use_first(&mut game, 1);
    assert_eq!(game.item_active(0, 0), Some(false));
    assert_eq!(
        game.hero(0).evasion,
        evasion,
        "toggling the aura does not remove evasion"
    );
    game.advance(rules::AURA_LINGER_TICKS + 1);
    game.cheat(
        2,
        Cheat::SetPools {
            health: 100,
            mana: 100,
        },
    );
    game.advance(rules::TICKS_PER_SECOND + 1);
    assert_eq!(
        game.hero(2).hp,
        game.hero(2).max_hp,
        "the disabled burn expires"
    );
}

#[test]
fn shivas_expanding_wave_follows_its_caster_and_hits_each_enemy_once() {
    let mut game = Match::new();
    give(&mut game, 0, "item_shivas_guard");
    let hp = game.hero(2).hp;
    use_first(&mut game, 0);
    game.advance(10);
    assert_eq!(
        game.hero(2).hp,
        hp,
        "the wave has not reached an enemy 600 units away"
    );
    game.cheat(
        0,
        Cheat::Teleport {
            at: Vec2::from_ints(9400, 5000),
        },
    );
    game.advance(40);
    assert_eq!(
        game.hero(2).hp,
        hp,
        "the wave follows its caster, leaving this enemy outside its current radius"
    );
    game.advance(15);
    let hit = game.hero(2).hp;
    assert!(hit < hp, "the expanding wave eventually reaches the enemy");
    assert!(game.hero(2).statuses.bits & bota_proto::StatusFlags::SLOWED != 0);
    game.advance(20);
    assert!(
        game.hero(2).hp >= hit,
        "one wave does not hit the same enemy twice"
    );
}

#[test]
fn typed_damage_cheat_enters_mitigation_and_consumes_only_matching_barriers() {
    let mut game = Match::new();
    give(&mut game, 0, "item_pipe");
    use_first(&mut game, 0);
    let capacity = |game: &Match| {
        game.hero(1)
            .effects
            .iter()
            .find_map(|effect| {
                (effect.inflictor == Some(bota_proto::Inflictor::Item(item("item_pipe"))))
                    .then_some(effect.stacks)
                    .flatten()
            })
            .expect("barrier shown")
    };
    let full = capacity(&game);
    let hp = game.hero(1).hp;
    game.cheat(
        1,
        Cheat::Damage {
            target: bota_proto::Target::None,
            amount: 100,
            kind: bota_proto::DamageKind::Magical,
        },
    );
    let magic_resist_bp = (i64::from(game.hero(1).magic_resist.raw) * i64::from(rules::NOMINAL_BP)
        / i64::from(bota_proto::Fixed::ONE.raw)) as i32;
    let reduced = bota_core::mitigate(
        100,
        bota_proto::DamageKind::Magical,
        game.hero(1).armor,
        magic_resist_bp,
    );
    assert_eq!(
        full - capacity(&game),
        reduced as u32,
        "the barrier is consumed after magic resistance"
    );
    assert_eq!(game.hero(1).hp, hp, "the barrier protects health");
    let after_magic = capacity(&game);
    game.cheat(
        1,
        Cheat::Damage {
            target: bota_proto::Target::None,
            amount: 100,
            kind: bota_proto::DamageKind::Physical,
        },
    );
    assert_eq!(
        capacity(&game),
        after_magic,
        "physical damage passes the magic barrier"
    );
    assert!(game.hero(1).hp < hp, "physical damage reaches health");
}

#[test]
fn damage_cheats_reject_unbounded_amounts_and_are_disabled_in_ordinary_matches() {
    let mut game = Match::new();
    for amount in [0, 1001, u16::MAX] {
        let order = Order::Cheat {
            cheat: Cheat::Damage {
                target: bota_proto::Target::None,
                amount,
                kind: bota_proto::DamageKind::Pure,
            },
        };
        assert_eq!(
            game.try_order(0, order),
            vec![bota_proto::RejectReason::BadCheat]
        );
        assert_eq!(game.hero(0).hp, game.hero(0).max_hp);
    }
    game.game.world_mut().cheats = false;
    let order = Order::Cheat {
        cheat: Cheat::Damage {
            target: bota_proto::Target::None,
            amount: 100,
            kind: bota_proto::DamageKind::Pure,
        },
    };
    assert_eq!(
        game.try_order(0, order),
        vec![bota_proto::RejectReason::NoCheats]
    );
    assert_eq!(game.hero(0).hp, game.hero(0).max_hp);
}

#[test]
fn pipe_protects_an_allied_mid_tower_from_magic_damage() {
    let mut game = Match::new();
    let tower_at = bota_core::map_of(MapId(2))
        .radiant_towers
        .iter()
        .find(|(lane, tier, _)| *lane == rules::LANE_MID && *tier == 1)
        .expect("mid tier one")
        .2;
    let tower = game.views[0]
        .units
        .iter()
        .find(|unit| {
            unit.kind == bota_proto::UnitKind::Tower
                && unit.team == Team::Radiant
                && unit.pos == tower_at
        })
        .expect("the tower is shown")
        .id;
    game.cheat(
        0,
        Cheat::Teleport {
            at: tower_at + Vec2::from_ints(0, -1000),
        },
    );
    give(&mut game, 0, "item_pipe");
    use_first(&mut game, 0);
    let capacity = |game: &Match| {
        game.views[0]
            .unit(tower)
            .expect("tower")
            .effects
            .iter()
            .find_map(|effect| {
                (effect.inflictor == Some(bota_proto::Inflictor::Item(item("item_pipe"))))
                    .then_some(effect.stacks)
                    .flatten()
            })
            .expect("tower's barrier is shown")
    };
    let full = capacity(&game);
    let before = game.views[0].unit(tower).expect("tower").hp;
    game.cheat(
        0,
        Cheat::Damage {
            target: bota_proto::Target::Unit(tower),
            amount: 100,
            kind: bota_proto::DamageKind::Magical,
        },
    );
    assert!(capacity(&game) < full, "building barriers absorb damage");
    assert_eq!(game.views[0].unit(tower).expect("tower").hp, before);
}

#[test]
fn tranquil_boots_break_on_a_landed_attack_and_refresh_does_not_restore_the_passive() {
    let mut game = Match::new();
    let regen = game.hero(0).hp_regen;
    give(&mut game, 0, "item_tranquil_boots");
    let normal_speed = game.hero(0).move_speed;
    let healthy_regen = game.hero(0).hp_regen;
    let expected_regen = bota_proto::Fixed::from_ratio(
        source("item_tranquil_boots", "bonus_health_regen").hundredths(),
        100,
    );
    assert!((healthy_regen.raw - regen.raw - expected_regen.raw).abs() < 64);
    let target = game.hero(0).id;
    let before = game.hero(0).hp;
    game.order(
        2,
        Order::Attack {
            target: bota_proto::Target::Unit(target),
        },
    );
    for _ in 0..90 {
        if game.hero(0).hp < before {
            break;
        }
        game.advance(1);
    }
    assert!(game.hero(0).hp < before, "an actual attack landed");
    game.order(
        2,
        Order::Move {
            target: bota_proto::Target::None,
        },
    );
    assert_eq!(
        game.hero(0).hp_regen,
        regen,
        "broken boots grant no regeneration"
    );
    let lost_speed = source("item_tranquil_boots", "bonus_movement_speed").int()
        - source("item_tranquil_boots", "broken_movement_speed").int();
    assert_eq!(
        normal_speed - game.hero(0).move_speed,
        bota_proto::Fixed::from_int(lost_speed)
    );
    game.cheat(0, Cheat::Refresh);
    assert_eq!(
        game.hero(0).hp_regen,
        regen,
        "refresh resets cooldowns without repairing boots"
    );
    game.advance(source("item_tranquil_boots", "break_time").secs().ticks());
    assert_eq!(
        game.hero(0).hp_regen,
        healthy_regen,
        "the passive returns after its imported recovery time"
    );
    assert_eq!(game.hero(0).move_speed, normal_speed);
}

#[test]
fn tranquil_boots_disassembled_from_stash_remember_the_break_when_recombined() {
    use bota_proto::{ItemSlot, Target};
    let mut game = Match::new();
    let fountain = bota_core::fountain_pos(bota_core::map_of(MapId(2)), Team::Radiant);
    game.cheat(0, Cheat::Teleport { at: fountain });
    game.cheat(0, Cheat::Gold { amount: 10000 });
    game.order(
        0,
        Order::Buy {
            item: item("item_tranquil_boots"),
        },
    );
    let normal_regen = game.hero(0).hp_regen;
    game.order(
        0,
        Order::Move {
            target: Target::None,
        },
    );
    game.order(
        0,
        Order::Swap {
            from: ItemSlot(0),
            to: ItemSlot(9),
        },
    );
    game.order(0, Order::Disassemble { slot: ItemSlot(9) });
    let parts: Vec<_> = game.views[0].players[0]
        .stash
        .as_ref()
        .expect("own stash")
        .iter()
        .enumerate()
        .filter_map(|(slot, item)| item.map(|item| (ItemSlot(9 + slot as u8), item.id)))
        .collect();
    assert_eq!(
        parts.len(),
        bota_core::item_def(item("item_tranquil_boots"))
            .expect("definition")
            .components
            .len()
    );
    for (slot, _) in parts {
        game.order(
            0,
            Order::SetCombineLock {
                slot,
                locked: false,
            },
        );
    }
    let stash = game.views[0].players[0].stash.as_ref().expect("own stash");
    let slot = stash
        .iter()
        .position(|held| held.is_some_and(|held| held.id == item("item_tranquil_boots")))
        .expect("boots reassembled");
    game.order(
        0,
        Order::Swap {
            from: ItemSlot(9 + slot as u8),
            to: ItemSlot(0),
        },
    );
    game.cheat(
        0,
        Cheat::Teleport {
            at: Vec2::from_ints(8000, 5000),
        },
    );
    game.advance(rules::FOUNTAIN_HEAL_LINGER_TICKS + 1);
    assert!(
        game.hero(0).hp_regen < normal_regen,
        "7.41f: taking boots apart in stash starts the remembered break"
    );
}

#[test]
fn status_resistance_shortens_shivas_slow_duration_without_weakening_its_strength() {
    let mut game = Match::new();
    game.cheat(
        2,
        Cheat::ApplyModifier {
            target: bota_proto::Target::None,
            spec: bota_proto::ModifierSpec {
                status_resist: 5000,
                ..bota_proto::ModifierSpec::NOMINAL
            },
            ticks: 1000,
        },
    );
    give(&mut game, 0, "item_shivas_guard");
    let speed = game.hero(2).move_speed;
    use_first(&mut game, 0);
    let slow = bota_core::item_def(item("item_shivas_guard"))
        .expect("Shiva")
        .modifiers[1]
        .effect;
    for _ in 0..90 {
        if game.hero(2).effects.iter().any(|effect| effect.id == slow) {
            break;
        }
        game.advance(1);
    }
    let left = game
        .hero(2)
        .effects
        .iter()
        .find(|effect| effect.id == slow)
        .expect("blast reached enemy")
        .ticks_left
        .expect("timed slow");
    let duration = source("item_shivas_guard", "blast_debuff_duration")
        .secs()
        .ticks();
    // Valve 7.34: status resistance affects slow duration; slow resistance affects strength.
    // https://www.dota2.com/patches/7.34
    assert!(
        (i64::from(left) - i64::from(duration / 2)).abs() <= 1,
        "half-duration slow: {left}/{duration}"
    );
    let strength = source("item_shivas_guard", "blast_movement_speed").int();
    let expected_speed = i64::from(speed.raw) * i64::from(100 + strength) / 100;
    assert!(
        (i64::from(game.hero(2).move_speed.raw) - expected_speed).abs() <= 1,
        "status resistance leaves slow strength unchanged"
    );
}

#[test]
fn bearing_shares_regeneration_and_grants_temporary_slow_immunity_with_endurance() {
    let mut game = Match::new();
    let regen = game.hero(1).hp_regen;
    give(&mut game, 0, "item_boots_of_bearing");
    let aura_regen = bota_proto::Fixed::from_ratio(
        source("item_boots_of_bearing", "aura_health_regen").hundredths(),
        100,
    );
    assert!((game.hero(1).hp_regen.raw - regen.raw - aura_regen.raw).abs() < 64);
    give(&mut game, 2, "item_shivas_guard");
    use_first(&mut game, 2);
    let slow = bota_core::item_def(item("item_shivas_guard"))
        .expect("Shiva")
        .modifiers[1]
        .effect;
    for _ in 0..90 {
        if game.hero(1).effects.iter().any(|effect| effect.id == slow) {
            break;
        }
        game.advance(1);
    }
    assert!(
        game.hero(1).effects.iter().any(|effect| effect.id == slow),
        "Shiva slowed the ally"
    );
    let slowed = game.hero(1).move_speed;
    let attack_speed = game.hero(1).attack_speed;
    use_first(&mut game, 0);
    let immune = game.hero(1).move_speed;
    assert!(
        immune > slowed,
        "endurance grants movement and prevents slowing"
    );
    assert_eq!(
        game.hero(1).attack_speed - attack_speed,
        source("item_boots_of_bearing", "bonus_attack_speed_pct").int()
    );
    game.advance(
        source("item_boots_of_bearing", "bonus_ms_duration")
            .secs()
            .ticks(),
    );
    assert!(
        game.hero(1).move_speed < immune,
        "slow immunity ends while endurance remains"
    );
    assert!(
        game.hero(1).move_speed > slowed,
        "endurance's movement bonus still applies"
    );
}

#[test]
fn a_passive_aura_expands_and_contracts_when_an_area_bonus_is_equipped_or_backpacked() {
    let mut game = Match::new();
    let baseline = game.hero(1).hp_regen;
    give(&mut game, 0, "item_headdress");
    let radius = source("item_headdress", "aura_radius").int();
    let bonus = source("item_shivas_guard", "bonus_aoe").int();
    let origin = game.hero(0).pos;
    let distance = radius + bonus / 2;
    let at = [(distance, 0), (0, distance), (-distance, 0), (0, -distance)]
        .into_iter()
        .find_map(|(x, y)| {
            let candidate = origin + Vec2::from_ints(x, y);
            game.game.world_mut().open_landing(candidate).filter(|at| {
                !at.within(origin, bota_proto::Fixed::from_int(radius))
                    && at.within(origin, bota_proto::Fixed::from_int(radius + bonus))
            })
        })
        .expect("walkable point between the original and expanded radii");
    game.cheat(1, Cheat::Teleport { at });
    game.order(
        1,
        Order::Move {
            target: bota_proto::Target::None,
        },
    );
    assert!(
        !game
            .hero(1)
            .pos
            .within(game.hero(0).pos, bota_proto::Fixed::from_int(radius)),
        "teleport is outside the original radius"
    );
    game.advance(30);
    assert_eq!(
        game.hero(1).pos,
        at,
        "receiver stays at the chosen boundary point"
    );
    assert_eq!(game.hero(1).hp_regen, baseline, "outside the original aura");
    give(&mut game, 0, "item_shivas_guard");
    assert!(
        game.hero(1).hp_regen > baseline,
        "equipping the area bonus expands an existing aura"
    );
    game.order(
        0,
        Order::Swap {
            from: bota_proto::ItemSlot(1),
            to: bota_proto::ItemSlot(6),
        },
    );
    game.advance(30);
    assert_eq!(
        game.hero(1).hp_regen,
        baseline,
        "backpacking the area bonus contracts the aura after its linger"
    );
}

#[test]
fn an_area_recovery_does_not_emit_empty_presentation_facts() {
    let mut game = Match::new();
    give(&mut game, 0, "item_mekansm");
    let ally = game.hero(1).id;
    game.cheat(
        1,
        Cheat::SetPools {
            health: 100,
            mana: 100,
        },
    );
    use_first(&mut game, 0);
    assert!(!game.facts[0].iter().any(|event| matches!(event,
        bota_proto::EventKind::Healed { target, amount: 0, mana: 0, inflictor: Some(bota_proto::Inflictor::Item(used)), .. }
            if *target == ally && *used == item("item_mekansm"))),
        "a full-pool receiver has no empty recovery event");
    assert_eq!(
        game.hero(1).hp,
        game.hero(1).max_hp,
        "no excess restoration"
    );
}

#[test]
fn tranquil_boots_ignore_spell_damage_and_attacks_the_bearer_lands_on_creeps() {
    use bota_proto::{DamageKind, EventKind, Target, UnitKind};
    let mut game = Match::new();
    give(&mut game, 0, "item_tranquil_boots");
    let health = game.hero(0).hp;
    game.cheat(
        0,
        Cheat::Damage {
            target: Target::None,
            amount: 20,
            kind: DamageKind::Magical,
        },
    );
    assert!(game.hero(0).hp < health, "ordinary magical damage arrived");
    assert_eq!(
        game.item_active(0, 0),
        Some(true),
        "spell damage does not break the passive"
    );
    game.cheat(
        0,
        Cheat::CreateCreeps {
            enemy: false,
            at: Vec2::from_ints(7600, 5000),
        },
    );
    let creep = game.views[0]
        .units
        .iter()
        .find(|unit| unit.team == Team::Radiant && unit.kind == UnitKind::CreepMelee)
        .expect("created allied creep")
        .clone();
    let damage = u16::try_from(creep.hp * 3 / 4).expect("bounded wave-one health");
    game.cheat(
        0,
        Cheat::Damage {
            target: Target::Unit(creep.id),
            amount: damage,
            kind: DamageKind::Pure,
        },
    );
    let attacker = game.hero(0).id;
    game.order(
        0,
        Order::Attack {
            target: Target::Unit(creep.id),
        },
    );
    let mut landed = false;
    for _ in 0..90 {
        landed |= game.facts[0].iter().any(|event| matches!(event, EventKind::Damaged { source: Some(source), target, inflictor: None, .. } if *source == attacker && *target == creep.id));
        if landed {
            break;
        }
        game.advance(1);
    }
    assert!(landed, "the hero landed a real denying attack");
    game.order(
        0,
        Order::Move {
            target: Target::None,
        },
    );
    assert_eq!(
        game.item_active(0, 0),
        Some(true),
        "attacking a creep does not break the passive"
    );
}

#[test]
fn tranquil_boots_recovery_uses_the_wearers_cooldown_reduction() {
    use bota_proto::{ModifierSpec, Target};
    let mut game = Match::new();
    game.cheat(
        0,
        Cheat::ApplyModifier {
            target: Target::None,
            spec: ModifierSpec {
                cooldown_rate: 5000,
                ..ModifierSpec::NOMINAL
            },
            ticks: 1000,
        },
    );
    give(&mut game, 0, "item_tranquil_boots");
    let intact = game.hero(0).hp_regen;
    let target = game.hero(0).id;
    let health = game.hero(0).hp;
    game.order(
        2,
        Order::Attack {
            target: Target::Unit(target),
        },
    );
    for _ in 0..90 {
        if game.hero(0).hp < health {
            break;
        }
        game.advance(1);
    }
    assert!(game.hero(0).hp < health, "attack landed");
    game.order(
        2,
        Order::Move {
            target: Target::None,
        },
    );
    let left = game.hero(0).items[0].expect("boots").cooldown_left;
    let expected = source("item_tranquil_boots", "break_time").secs().ticks() / 2;
    assert_eq!(left, expected - 1, "one tick elapsed after the landed blow");
    game.advance(left - 1);
    assert!(
        game.hero(0).hp_regen < intact,
        "still broken one tick before recovery"
    );
    game.advance(1);
    assert_eq!(
        game.hero(0).hp_regen,
        intact,
        "recovery agrees with the reduced countdown"
    );
}

#[test]
fn endurance_and_swiftness_from_drums_and_bearing_use_shared_buff_families() {
    let mut game = Match::new();
    give(&mut game, 0, "item_ancient_janggo");
    let aura_speed = game.hero(0).move_speed;
    give(&mut game, 1, "item_boots_of_bearing");
    assert_eq!(
        game.hero(0).move_speed,
        aura_speed,
        "both swiftness auras grant only one movement bonus"
    );
    let attack_speed = game.hero(0).attack_speed;
    use_first(&mut game, 0);
    let drums_bonus = source("item_ancient_janggo", "bonus_movement_speed_pct").int();
    let drums_speed = i64::from(aura_speed.raw) * i64::from(100 + drums_bonus) / 100;
    assert!(
        (i64::from(game.hero(0).move_speed.raw) - drums_speed).abs() <= 1,
        "drums uses its imported movement bonus"
    );
    use_first(&mut game, 1);
    let bearing_bonus = source("item_boots_of_bearing", "bonus_movement_speed_pct").int();
    let bearing_speed = i64::from(aura_speed.raw) * i64::from(100 + bearing_bonus) / 100;
    assert!(
        (i64::from(game.hero(0).move_speed.raw) - bearing_speed).abs() <= 1,
        "only the stronger imported endurance movement bonus applies"
    );
    assert_eq!(
        game.hero(0).attack_speed - attack_speed,
        source("item_boots_of_bearing", "bonus_attack_speed_pct").int(),
        "one attack-speed bonus"
    );
}

#[test]
fn drums_and_bearing_in_one_inventory_share_their_activation_cooldown() {
    use bota_proto::{ItemSlot, RejectReason, Target};
    let mut game = Match::new();
    give(&mut game, 0, "item_ancient_janggo");
    give(&mut game, 0, "item_boots_of_bearing");
    use_first(&mut game, 0);
    assert!(
        game.hero(0).items[1].expect("bearing held").cooldown_left > 0,
        "bearing shares the wait"
    );
    assert_eq!(
        game.try_order(
            0,
            Order::Use {
                slot: ItemSlot(1),
                target: Target::None
            }
        ),
        vec![RejectReason::OnCooldown]
    );
    game.cheat(0, Cheat::Refresh);
    game.order(
        0,
        Order::Use {
            slot: ItemSlot(1),
            target: Target::None,
        },
    );
    assert!(
        game.hero(0).items[0].expect("drums held").cooldown_left > 0,
        "drums shares bearing's wait too"
    );
}

#[test]
fn greaves_restore_mana_while_mekansms_shared_health_lockout_is_active() {
    use bota_proto::{EventKind, Inflictor};
    let mut game = Match::new();
    give(&mut game, 0, "item_mekansm");
    give(&mut game, 1, "item_guardian_greaves");
    use_first(&mut game, 0);
    game.cheat(
        0,
        Cheat::SetPools {
            health: 25,
            mana: 0,
        },
    );
    let receiver = game.hero(0).id;
    use_first(&mut game, 1);
    assert!(
        game.facts[0].iter().any(|event| matches!(event,
        EventKind::Healed { target, amount: 0, mana, inflictor: Some(Inflictor::Item(used)), .. }
        if *target == receiver && *mana > 0 && *used == item("item_guardian_greaves"))),
        "Mend restores mana before checking the shared health lockout"
    );
    assert!(game.hero(0).mana > source("item_guardian_greaves", "replenish_mana").int() - 2);
}

#[test]
fn shared_recovery_lockout_is_fixed_by_the_prior_successful_casters_cooldown() {
    use bota_proto::{EventKind, Inflictor, ModifierSpec, Target};
    for (first, second, first_rate, second_rate) in [
        ("item_mekansm", "item_guardian_greaves", 10_000, 10_000),
        ("item_guardian_greaves", "item_mekansm", 10_000, 10_000),
        ("item_mekansm", "item_guardian_greaves", 5_000, 10_000),
        ("item_guardian_greaves", "item_mekansm", 10_000, 5_000),
    ] {
        let mut game = Match::new();
        for (seat, rate) in [(0, first_rate), (1, second_rate)] {
            game.cheat(
                seat,
                Cheat::ApplyModifier {
                    target: Target::None,
                    spec: ModifierSpec {
                        cooldown_rate: rate,
                        ..ModifierSpec::NOMINAL
                    },
                    ticks: 10_000,
                },
            );
        }
        give(&mut game, 0, first);
        give(&mut game, 1, second);
        game.cheat(
            0,
            Cheat::SetPools {
                health: 25,
                mana: 100,
            },
        );
        use_first(&mut game, 0);
        let cast_at = game.game.tick();
        let interval = |key, rate| {
            bota_core::item_def(item(key)).expect("item").cooldown * rate / 10_000
                - rules::TICKS_PER_SECOND
        };
        let prior = interval(first, first_rate as u32);
        let next = interval(second, second_rate as u32);
        assert_ne!(
            prior, next,
            "fixture distinguishes prior and later cooldowns"
        );
        let probe = (prior + next) / 2;
        game.cheat(
            0,
            Cheat::SetPools {
                health: 25,
                mana: 100,
            },
        );
        let elapsed = game.game.tick() - cast_at;
        game.advance(probe - elapsed - 1);
        let receiver = game.hero(0).id;
        use_first(&mut game, 1);
        assert_eq!(
            game.game.tick() - cast_at,
            probe,
            "cast at the chosen boundary probe"
        );
        let healed = game.facts[0].iter().any(|event| {
            matches!(event,
            EventKind::Healed { target, amount, inflictor: Some(Inflictor::Item(used)), .. }
                if *target == receiver && *amount > 0 && *used == item(second))
        });
        assert_eq!(
            healed,
            probe >= prior,
            "{first}@{first_rate} then {second}@{second_rate}: prior={prior}, later={next}, probe={probe}"
        );
    }
}

#[test]
fn pipe_uses_the_prior_casters_receiver_timer_after_the_barrier_has_expired() {
    use bota_proto::{ModifierSpec, Target};
    let mut game = Match::new();
    game.cheat(
        0,
        Cheat::ApplyModifier {
            target: Target::None,
            spec: ModifierSpec {
                cooldown_rate: 5000,
                ..ModifierSpec::NOMINAL
            },
            ticks: 10_000,
        },
    );
    give(&mut game, 0, "item_pipe");
    give(&mut game, 1, "item_pipe");
    let def = bota_core::item_def(item("item_pipe")).expect("Pipe");
    let barrier = def.modifiers[1].effect;
    let prior = def.cooldown / 2 - rules::TICKS_PER_SECOND;
    assert!(prior > source("item_pipe", "barrier_duration").secs().ticks());
    use_first(&mut game, 0);
    game.advance(prior - 2);
    assert!(
        !game
            .hero(0)
            .effects
            .iter()
            .any(|effect| effect.id == barrier),
        "the earlier barrier expired"
    );
    use_first(&mut game, 1);
    assert!(
        !game
            .hero(0)
            .effects
            .iter()
            .any(|effect| effect.id == barrier),
        "receiver timer blocks one tick before expiry"
    );
    game.cheat(1, Cheat::Refresh);
    use_first(&mut game, 1);
    assert!(
        game.hero(0)
            .effects
            .iter()
            .any(|effect| effect.id == barrier),
        "the prior timer expired despite the new caster's longer cooldown"
    );
}

#[test]
fn receiver_cooldown_modifiers_are_visible_with_their_remaining_duration() {
    for (key, index) in [("item_mekansm", 1), ("item_pipe", 2)] {
        let mut game = Match::new();
        give(&mut game, 0, key);
        use_first(&mut game, 0);
        let def = bota_core::item_def(item(key)).expect("item");
        let timer = def.modifiers[index].effect;
        let shown = game
            .hero(1)
            .effects
            .iter()
            .find(|effect| effect.id == timer)
            .expect("receiver cooldown is shown as a modifier");
        assert_eq!(
            shown.ticks_left,
            Some(def.cooldown - rules::TICKS_PER_SECOND)
        );
    }
}

#[test]
fn receiver_cooldowns_end_on_death_and_allow_recovery_after_a_cheated_respawn() {
    use bota_proto::{DamageKind, EventKind, Inflictor, Target};
    for key in ["item_mekansm", "item_pipe"] {
        let mut game = Match::with_immortality(false);
        give(&mut game, 0, key);
        give(&mut game, 1, key);
        use_first(&mut game, 0);
        game.cheat(
            0,
            Cheat::Damage {
                target: Target::None,
                amount: 1000,
                kind: DamageKind::Pure,
            },
        );
        assert!(
            game.views[0].player(SlotId(0)).expect("seat").kit.is_some(),
            "the receiver died"
        );
        game.cheat(0, Cheat::Respawn);
        game.cheat(
            0,
            Cheat::Teleport {
                at: Vec2::from_ints(8000, 5000),
            },
        );
        game.order(
            0,
            Order::Move {
                target: Target::None,
            },
        );
        game.advance(rules::FOUNTAIN_HEAL_LINGER_TICKS + 1);
        game.cheat(
            0,
            Cheat::SetPools {
                health: 25,
                mana: 100,
            },
        );
        let receiver = game.hero(0).id;
        use_first(&mut game, 1);
        if key == "item_mekansm" {
            assert!(
                game.facts[0].iter().any(|event| matches!(event,
                EventKind::Healed { target, amount, inflictor: Some(Inflictor::Item(used)), .. }
                    if *target == receiver && *amount > 0 && *used == item(key))),
                "death removed the previous health lockout"
            );
        } else {
            let barrier = bota_core::item_def(item(key)).expect("item").modifiers[1].effect;
            assert!(
                game.hero(0)
                    .effects
                    .iter()
                    .any(|effect| effect.id == barrier),
                "death removed the previous barrier lockout"
            );
        }
    }
}

#[test]
fn greaves_self_dispel_removes_a_shiva_slow_before_restoring_the_caster() {
    let mut game = Match::new();
    give(&mut game, 0, "item_guardian_greaves");
    give(&mut game, 2, "item_shivas_guard");
    use_first(&mut game, 2);
    let slow = bota_core::item_def(item("item_shivas_guard"))
        .expect("Shiva")
        .modifiers[1]
        .effect;
    for _ in 0..90 {
        if game.hero(0).effects.iter().any(|effect| effect.id == slow) {
            break;
        }
        game.advance(1);
    }
    assert!(
        game.hero(0).effects.iter().any(|effect| effect.id == slow),
        "the wave applied its dispellable slow"
    );
    let speed = game.hero(0).move_speed;
    use_first(&mut game, 0);
    assert!(
        !game.hero(0).effects.iter().any(|effect| effect.id == slow),
        "Mend's self dispel removes the slow"
    );
    assert!(
        game.hero(0).move_speed > speed,
        "movement is restored in the published snapshot"
    );
}

fn next_item_damage(
    game: &mut Match,
    target: bota_proto::EntityId,
    used: ItemId,
) -> (Option<bota_proto::EntityId>, i32) {
    for _ in 0..2 * rules::TICKS_PER_SECOND + 1 {
        if let Some(hit) = game.facts[0].iter().find_map(|event| match event {
            bota_proto::EventKind::Damaged {
                source,
                target: victim,
                amount,
                inflictor: Some(bota_proto::Inflictor::Item(item)),
                ..
            } if *victim == target && *item == used => Some((*source, *amount)),
            _ => None,
        }) {
            return hit;
        }
        game.advance(1);
    }
    panic!("no item damage reached the target");
}

#[test]
fn radiance_from_an_illusion_uses_its_imported_burn_and_yields_to_a_real_carrier() {
    use bota_proto::{DamageKind, Fixed, RuneKind, RuneSet, Target, UnitKind};
    let mut game = Match::with_options(true, RuneSet::of(RuneKind::Illusion));
    give(&mut game, 0, "item_radiance");
    game.take_cheated_rune(RuneKind::Illusion);
    let real = game.hero(0).id;
    let illusion = game.views[0]
        .units
        .iter()
        .find(|unit| {
            unit.kind == UnitKind::Hero && unit.owner == Some(SlotId(0)) && unit.id != real
        })
        .expect("rune illusion")
        .id;
    game.order_for(
        0,
        illusion,
        Order::Cheat {
            cheat: Cheat::Teleport {
                at: Vec2::from_ints(8900, 5000),
            },
        },
    );
    game.order_for(
        0,
        illusion,
        Order::Move {
            target: Target::None,
        },
    );
    let target = game.views[0]
        .units
        .iter()
        .find(|unit| unit.kind == UnitKind::Hero && unit.owner == Some(SlotId(2)))
        .expect("enemy")
        .clone();
    let radiance = item("item_radiance");
    let radius = Fixed::from_int(source("item_radiance", "aura_radius").int());
    assert!(
        !game.hero(0).pos.within(target.pos, radius),
        "real carrier is outside burn range"
    );
    assert!(
        game.views[0]
            .unit(illusion)
            .expect("illusion")
            .pos
            .within(target.pos, radius),
        "illusion is in burn range"
    );
    let mitigated = |damage| {
        bota_core::mitigate(
            damage,
            DamageKind::Magical,
            Fixed::ZERO,
            bota_core::magic_resist_bp(target.magic_resist),
        )
    };
    let (dealer, damage) = next_item_damage(&mut game, target.id, radiance);
    assert_eq!(dealer, Some(illusion));
    assert_eq!(
        damage,
        mitigated(source("item_radiance", "aura_damage_illusions").int()),
        "the illusion's explicit burn is not its attack-damage share"
    );
    game.cheat(
        0,
        Cheat::Teleport {
            at: Vec2::from_ints(8000, 5000),
        },
    );
    game.order(
        0,
        Order::Move {
            target: Target::None,
        },
    );
    let (dealer, damage) = next_item_damage(&mut game, target.id, radiance);
    assert_eq!(dealer, Some(real), "real carrier takes priority");
    assert_eq!(
        damage,
        mitigated(source("item_radiance", "aura_damage").int())
    );
}

#[test]
fn illusion_attacks_and_existing_pierces_keep_their_captured_damage_share() {
    use bota_proto::{DamageKind, RuneKind, RuneSet, Target, UnitKind};
    let mut game = Match::with_options(true, RuneSet::of(RuneKind::Illusion));
    give(&mut game, 0, "item_monkey_king_bar");
    game.take_cheated_rune(RuneKind::Illusion);
    let real = game.hero(0).id;
    let illusion = game.views[0]
        .units
        .iter()
        .find(|unit| {
            unit.kind == UnitKind::Hero && unit.owner == Some(SlotId(0)) && unit.id != real
        })
        .expect("rune illusion")
        .id;
    game.order_for(
        0,
        illusion,
        Order::Cheat {
            cheat: Cheat::Teleport {
                at: Vec2::from_ints(8900, 5000),
            },
        },
    );
    game.order_for(
        0,
        illusion,
        Order::Move {
            target: Target::None,
        },
    );
    let attacker = game.views[0].unit(illusion).expect("illusion").clone();
    let target = game.views[0]
        .units
        .iter()
        .find(|unit| unit.kind == UnitKind::Hero && unit.owner == Some(SlotId(2)))
        .expect("enemy")
        .clone();
    game.order_for(
        0,
        illusion,
        Order::Attack {
            target: Target::Unit(target.id),
        },
    );
    let mut magical = None;
    let mut physical = Vec::new();
    for _ in 0..300 {
        for event in &game.facts[0] {
            if let bota_proto::EventKind::Damaged {
                source: Some(source),
                target: victim,
                amount,
                kind,
                inflictor,
                ..
            } = event
                && *source == illusion
                && *victim == target.id
            {
                match kind {
                    DamageKind::Physical => physical.push(*amount),
                    DamageKind::Magical
                        if *inflictor
                            == Some(bota_proto::Inflictor::Item(item("item_monkey_king_bar"))) =>
                    {
                        magical = Some(*amount)
                    }
                    _ => {}
                }
            }
        }
        if magical.is_some() {
            break;
        }
        game.advance(1);
    }
    let dealt = bota_core::ILLUSION_DEALT_BP;
    let mitigated = |amount, kind| {
        bota_core::mitigate(
            bota_core::amplify(amount, dealt),
            kind,
            target.armor,
            bota_core::magic_resist_bp(target.magic_resist),
        )
    };
    assert_eq!(
        magical,
        Some(mitigated(
            source("item_monkey_king_bar", "bonus_chance_damage").int(),
            DamageKind::Magical
        )),
        "the pre-existing rider path keeps its captured share"
    );
    assert!(
        !physical.is_empty(),
        "a real attack landed alongside its rider"
    );
    let minimum = mitigated(attacker.attack_damage_min, DamageKind::Physical);
    let maximum = mitigated(attacker.attack_damage_max, DamageKind::Physical);
    assert!(
        physical
            .iter()
            .all(|amount| (minimum..=maximum).contains(amount)),
        "illusion attack share stays in its imported damage range: {physical:?} vs{minimum}..{maximum}"
    );
}

#[test]
fn radiance_toggle_preserves_rune_invisibility_and_hides_unseen_activations() {
    use bota_proto::{EventKind, RuneKind, RuneSet, StatusFlags, Target};
    let mut game = Match::with_options(true, RuneSet::of(RuneKind::Invisibility));
    give(&mut game, 0, "item_radiance");
    game.take_cheated_rune(RuneKind::Invisibility);
    game.cheat(
        0,
        Cheat::Teleport {
            at: Vec2::from_ints(8000, 5000),
        },
    );
    game.order(
        0,
        Order::Move {
            target: Target::None,
        },
    );
    game.cheat(0, Cheat::AllVision { on: false });
    game.advance(bota_core::INVISIBILITY_FADE_TICKS + 1);
    assert_ne!(
        game.hero(0).statuses.bits & StatusFlags::INVISIBLE,
        0,
        "the rune faded the carrier"
    );
    assert!(
        !game.views[2]
            .units
            .iter()
            .any(|unit| unit.owner == Some(SlotId(0))),
        "enemy cannot see the carrier"
    );
    use_first(&mut game, 0);
    assert_ne!(
        game.hero(0).statuses.bits & StatusFlags::INVISIBLE,
        0,
        "7.41: toggling Burn preserves invisibility"
    );
    assert_eq!(game.item_active(0, 0), Some(false));
    assert!(!game.facts[2].iter().any(|event| matches!(event, EventKind::ItemUsed { item: used, .. } if *used == item("item_radiance"))), "an unseen invisible toggle is not reported to the enemy");
}

#[test]
fn radiance_toggle_preserves_an_active_town_portal_channel() {
    use bota_proto::{ItemSlot, Target, UnitKind};
    let mut game = Match::new();
    give(&mut game, 0, "item_radiance");
    game.cheat(
        0,
        Cheat::Item {
            item: ItemId::TOWN_PORTAL_SCROLL,
        },
    );
    game.cheat(0, Cheat::StartGame);
    game.cheat(0, Cheat::Refresh);
    let tower = game.views[0]
        .units
        .iter()
        .find(|unit| unit.kind == UnitKind::Tower && unit.team == Team::Radiant)
        .expect("allied tower")
        .id;
    game.order(
        0,
        Order::Use {
            slot: ItemSlot(rules::TOWN_PORTAL_SLOT as u8),
            target: Target::Unit(tower),
        },
    );
    for _ in 0..30 {
        if game.views[0]
            .teleports
            .iter()
            .any(|teleport| teleport.slot == SlotId(0))
        {
            break;
        }
        game.advance(1);
    }
    let before = *game.views[0]
        .teleports
        .iter()
        .find(|teleport| teleport.slot == SlotId(0))
        .unwrap_or_else(|| {
            panic!(
                "channel did not start: cast={:?}, scroll={:?}",
                game.hero(0).cast,
                game.hero(0).town_portal
            )
        });
    use_first(&mut game, 0);
    let after = game.views[0]
        .teleports
        .iter()
        .find(|teleport| teleport.slot == SlotId(0))
        .expect("7.41: toggle preserves channel");
    assert_eq!(after.building, before.building);
    assert_eq!(after.ticks_left, before.ticks_left - 1);
    assert_eq!(game.item_active(0, 0), Some(false));
}

#[test]
fn shivas_freezing_aura_pierces_a_cheated_spell_immunity_predicate() {
    use bota_proto::{StatusFlags, Target};
    let mut game = Match::new();
    let speed = game.hero(2).attack_speed;
    game.cheat(
        2,
        Cheat::SpellImmunity {
            target: Target::None,
            ticks: 10 * rules::TICKS_PER_SECOND,
        },
    );
    assert_ne!(
        game.hero(2).statuses.bits & StatusFlags::MAGIC_IMMUNE,
        0,
        "the timed setup modifier applies immunity"
    );
    give(&mut game, 0, "item_shivas_guard");
    assert_eq!(
        game.hero(2).attack_speed,
        speed + source("item_shivas_guard", "aura_attack_speed").int(),
        "7.41a: Freezing Aura pierces the immunity predicate"
    );
}

#[test]
fn a_spell_immunity_setup_cheat_rejects_unbounded_duration_and_disabled_cheats() {
    use bota_proto::{RejectReason, Target};
    let mut game = Match::new();
    assert_eq!(
        game.try_order(
            0,
            Order::Cheat {
                cheat: Cheat::SpellImmunity {
                    target: Target::None,
                    ticks: 0
                }
            }
        ),
        vec![RejectReason::BadCheat]
    );
    assert_eq!(
        game.try_order(
            0,
            Order::Cheat {
                cheat: Cheat::SpellImmunity {
                    target: Target::None,
                    ticks: u32::MAX
                }
            }
        ),
        vec![RejectReason::BadCheat]
    );
    game.game.world_mut().cheats = false;
    assert_eq!(
        game.try_order(
            0,
            Order::Cheat {
                cheat: Cheat::SpellImmunity {
                    target: Target::None,
                    ticks: 1
                }
            }
        ),
        vec![RejectReason::NoCheats]
    );
}

#[test]
fn tranquil_boots_remain_broken_on_rune_illusions_without_breaking_the_real_pair() {
    use bota_proto::{Fixed, RuneKind, RuneSet, UnitKind};
    let mut game = Match::with_options(true, RuneSet::of(RuneKind::Illusion));
    let base = game.hero(0).clone();
    give(&mut game, 0, "item_tranquil_boots");
    let intact = game.hero(0).hp_regen;
    game.take_cheated_rune(RuneKind::Illusion);
    let real = game.hero(0).id;
    let illusions: Vec<_> = game.views[0]
        .units
        .iter()
        .filter(|unit| {
            unit.kind == UnitKind::Hero && unit.owner == Some(SlotId(0)) && unit.id != real
        })
        .map(|unit| unit.id)
        .collect();
    assert_eq!(illusions.len(), bota_core::ILLUSION_RUNE_COUNT);
    for &illusion in &illusions {
        let unit = game.views[0].unit(illusion).expect("illusion");
        assert_eq!(
            unit.hp_regen, base.hp_regen,
            "illusion gets no Tranquil regeneration"
        );
        assert_eq!(
            unit.move_speed,
            base.move_speed
                + Fixed::from_int(source("item_tranquil_boots", "broken_movement_speed").int())
        );
    }
    assert_eq!(
        game.hero(0).hp_regen,
        intact,
        "the real pair remains intact"
    );
    game.advance(source("item_tranquil_boots", "break_time").secs().ticks() + 1);
    for illusion in illusions {
        assert_eq!(
            game.views[0]
                .unit(illusion)
                .expect("illusion remains alive")
                .hp_regen,
            base.hp_regen,
            "illusion passive stays broken beyond ordinary recovery"
        );
    }
}

#[test]
fn assault_auras_modify_allied_and_enemy_mid_towers_from_the_imported_values() {
    use bota_proto::{Fixed, Target, UnitKind};
    for team in [Team::Radiant, Team::Dire] {
        let mut game = Match::new();
        let map = bota_core::map_of(MapId(2));
        let towers = if team == Team::Radiant {
            map.radiant_towers
        } else {
            map.dire_towers
        };
        let at = towers
            .iter()
            .find(|(lane, tier, _)| *lane == rules::LANE_MID && *tier == 1)
            .expect("mid tower")
            .2;
        let tower = game.views[0]
            .units
            .iter()
            .find(|unit| unit.kind == UnitKind::Tower && unit.team == team && unit.pos == at)
            .expect("tower shown")
            .clone();
        game.cheat(
            0,
            Cheat::Teleport {
                at: at + Vec2::from_ints(0, -1000),
            },
        );
        game.order(
            0,
            Order::Move {
                target: Target::None,
            },
        );
        give(&mut game, 0, "item_assault");
        let shown = game.views[0].unit(tower.id).expect("tower shown");
        let key = if team == Team::Radiant {
            "aura_positive_armor"
        } else {
            "aura_negative_armor"
        };
        assert_eq!(
            shown.armor - tower.armor,
            Fixed::from_int(source("item_assault", key).int())
        );
        let attack_speed = if team == Team::Radiant {
            source("item_assault", "aura_attack_speed").int()
        } else {
            0
        };
        assert_eq!(shown.attack_speed - tower.attack_speed, attack_speed);
    }
}

// Valve 7.41c fixes other-restoration inflation, missing regeneration and overheals:
// https://www.dota2.com/news/updates/8659/
// The ordinary heal oracle keeps positive restoration separate from negative restoration
// and allocates diversion in proportion to the recipient's actual capped gain.
#[test]
fn croziers_passive_reduces_an_enemy_heroes_incoming_healing() {
    use bota_proto::{EventKind, Inflictor};
    let mut game = Match::new();
    give(&mut game, 0, "item_crellas_crozier");
    give(&mut game, 2, "item_mekansm");
    game.cheat(
        2,
        Cheat::SetPools {
            health: 25,
            mana: 100,
        },
    );
    let target = game.hero(2).id;
    use_first(&mut game, 2);
    let healed = game.facts[2]
        .iter()
        .find_map(|event| match event {
            EventKind::Healed {
                target: healed,
                amount,
                inflictor: Some(Inflictor::Item(used)),
                ..
            } if *healed == target && *used == item("item_mekansm") => Some(*amount),
            _ => None,
        })
        .expect("the recipient's recovery event");
    let expected = source("item_mekansm", "heal_amount").int()
        * (100 - source("item_crellas_crozier", "health_steal").int())
        / 100;
    assert!(
        (healed - expected).abs() <= 1,
        "passive restoration reduction: {healed} vs {expected}"
    );
}

fn crozier_heal_trial(
    amplified: bool,
    missing: Option<u16>,
    cast_heal: bool,
) -> (i32, Option<i32>) {
    use bota_proto::{DamageKind, EventKind, Inflictor, Target};
    let mut game = Match::new();
    give(&mut game, 0, "item_crellas_crozier");
    give(&mut game, 2, "item_mekansm");
    if amplified {
        give(&mut game, 2, "item_sange");
    }
    game.cheat(
        0,
        Cheat::SetPools {
            health: 10,
            mana: 100,
        },
    );
    game.cheat(
        2,
        Cheat::SetPools {
            health: if missing.is_some() { 100 } else { 25 },
            mana: 100,
        },
    );
    use_first(&mut game, 0);
    let period = source("item_crellas_crozier", "interval").secs().ticks();
    assert!(
        period > 2,
        "the collection boundary leaves room for two setup orders"
    );
    game.advance(period - 3);
    if let Some(amount) = missing.filter(|amount| *amount > 0) {
        game.cheat(
            2,
            Cheat::Damage {
                target: Target::None,
                amount,
                kind: DamageKind::Pure,
            },
        );
        assert_eq!(
            game.hero(2).max_hp - game.hero(2).hp,
            i32::from(amount),
            "exact missing health through real damage"
        );
    } else {
        game.order(
            2,
            Order::Move {
                target: Target::None,
            },
        );
    }
    let before = game.hero(0).hp;
    let recipient = game.hero(2).id;
    if cast_heal {
        use_first(&mut game, 2);
    } else {
        game.order(
            2,
            Order::Move {
                target: Target::None,
            },
        );
    }
    let healed = game.facts[2].iter().find_map(|event| match event {
        EventKind::Healed {
            target,
            amount,
            inflictor: Some(Inflictor::Item(used)),
            ..
        } if *target == recipient && *used == item("item_mekansm") => Some(*amount),
        _ => None,
    });
    game.advance(1);
    (game.hero(0).hp - before, healed)
}

#[test]
fn crozier_redirects_the_same_candidate_heal_with_and_without_positive_restoration() {
    let requested = source("item_mekansm", "heal_amount").int();
    let redirected_pct = source("item_crellas_crozier", "active_health_steal").int();
    for amplified in [false, true] {
        let (control, _) = crozier_heal_trial(amplified, None, false);
        let (collected, healed) = crozier_heal_trial(amplified, None, true);
        let amp = if amplified {
            source("item_sange", "hp_regen_amp").int()
        } else {
            0
        };
        let kept = requested * (100 + amp - redirected_pct) / 100;
        assert!(
            (healed.expect("recovery event") - kept).abs() <= 1,
            "restoration amp adds to the remaining heal"
        );
        let expected = requested * redirected_pct / 100;
        assert!(
            (collected - control - expected).abs() <= 1,
            "positive restoration must not inflate candidate diversion: amplified={amplified}, gain={}, expected={expected}",
            collected - control
        );
    }
}

#[test]
fn crozier_excludes_overheals_and_proportions_near_full_healing_after_the_health_cap() {
    let redirected_pct = source("item_crellas_crozier", "active_health_steal").int();
    for amplified in [false, true] {
        let amp = if amplified {
            source("item_sange", "hp_regen_amp").int()
        } else {
            0
        };
        for missing in [0, 10] {
            let (control, _) = crozier_heal_trial(amplified, Some(missing), false);
            let (collected, healed) = crozier_heal_trial(amplified, Some(missing), true);
            if missing == 0 {
                assert_eq!(healed, None, "no zero-amount healing fact");
            } else {
                assert!(healed.is_some(), "positive actual recovery is reported");
            }
            let expected = i32::from(missing) * redirected_pct / (100 + amp - redirected_pct);
            assert!(
                (collected - control - expected).abs() <= 1,
                "capped actual gain controls diversion: amplified={amplified}, missing={missing}, gain={}, expected={expected}",
                collected - control
            );
        }
    }
}

fn crozier_regeneration_trial(active: bool, full: bool) -> (bota_proto::Fixed, i32) {
    use bota_proto::Target;
    let mut game = Match::new();
    give(&mut game, 2, "item_headdress");
    let regeneration_per_second = game.hero(2).hp_regen;
    give(&mut game, 0, "item_crellas_crozier");
    game.cheat(
        0,
        Cheat::SetPools {
            health: 10,
            mana: 100,
        },
    );
    game.cheat(
        2,
        Cheat::SetPools {
            health: if full { 100 } else { 25 },
            mana: 100,
        },
    );
    let before = game.hero(0).hp;
    if active {
        use_first(&mut game, 0);
    } else {
        game.order(
            0,
            Order::Move {
                target: Target::None,
            },
        );
    }
    game.advance(source("item_crellas_crozier", "interval").secs().ticks());
    (regeneration_per_second, game.hero(0).hp - before)
}

#[test]
fn crozier_redirects_missing_health_regeneration_and_ignores_full_health_recipients() {
    let redirected_pct = source("item_crellas_crozier", "active_health_steal").int();
    let period = source("item_crellas_crozier", "interval").secs().ticks();
    for full in [false, true] {
        let (per_second, control) = crozier_regeneration_trial(false, full);
        let (_, collected) = crozier_regeneration_trial(true, full);
        let expected = if full {
            0
        } else {
            // UnitView reports per second; the collection period is in simulation ticks.
            i64::from(per_second.raw) * i64::from(period) * i64::from(redirected_pct)
                / (100 * i64::from(bota_proto::Fixed::ONE.raw) * i64::from(rules::TICKS_PER_SECOND))
        };
        assert!(
            (i64::from(collected - control) - expected).abs() <= 1,
            "only eligible regeneration is collected: full={full}, gain={}, expected={expected}",
            collected - control
        );
    }
}

#[test]
fn croziers_movement_buff_tracks_current_enemy_count_while_enemy_stacks_accumulate() {
    let mut game = Match::new();
    give(&mut game, 0, "item_crellas_crozier");
    let def = bota_core::item_def(item("item_crellas_crozier")).expect("item");
    let steal = def
        .modifiers
        .iter()
        .find_map(|template| template.movement_steal)
        .expect("movement transfer");
    let buff = def.modifiers[usize::from(steal.buff)].effect;
    let debuff = def.modifiers[usize::from(steal.debuff)].effect;
    let count = |game: &Match, seat, effect| {
        game.hero(seat)
            .effects
            .iter()
            .find(|held| held.id == effect)
            .and_then(|held| held.stacks)
    };
    use_first(&mut game, 0);
    assert_eq!(
        count(&game, 0, buff),
        Some(1),
        "one enemy in the creation pulse"
    );
    assert_eq!(count(&game, 2, debuff), Some(1));
    let first_speed = game.hero(0).move_speed;
    game.advance(steal.period);
    assert_eq!(
        count(&game, 0, buff),
        Some(1),
        "native caster count is set, not accumulated, each pulse"
    );
    assert_eq!(
        count(&game, 2, debuff),
        Some(2),
        "receiver count accumulates"
    );
    assert_eq!(
        game.hero(0).move_speed,
        first_speed,
        "caster speed follows this pulse's receiver count"
    );
}

#[test]
fn crozier_movement_receivers_share_the_existing_named_modifier_across_casters() {
    let mut game = Match::new();
    give(&mut game, 0, "item_crellas_crozier");
    give(&mut game, 1, "item_crellas_crozier");
    let def = bota_core::item_def(item("item_crellas_crozier")).expect("item");
    let steal = def
        .modifiers
        .iter()
        .find_map(|template| template.movement_steal)
        .expect("movement transfer");
    let debuff = def.modifiers[usize::from(steal.debuff)].effect;
    use_first(&mut game, 0);
    use_first(&mut game, 1);
    let received: Vec<_> = game
        .hero(2)
        .effects
        .iter()
        .filter(|held| held.id == debuff)
        .collect();
    assert_eq!(
        received.len(),
        1,
        "native lookup takes the existing named receiver without a caster filter"
    );
    assert_eq!(
        received[0].stacks,
        Some(2),
        "both creation pulses increment the shared receiver"
    );
}

#[test]
fn crozier_uses_shared_ghost_form_with_imported_vulnerability_and_inertness_while_immune() {
    use bota_proto::{DamageKind, EventKind, StatusFlags, Target};
    let mut game = Match::new();
    give(&mut game, 0, "item_crellas_crozier");
    game.cheat(
        0,
        Cheat::SetPools {
            health: 50,
            mana: 100,
        },
    );
    let plain_resist = game.hero(0).magic_resist;
    use_first(&mut game, 0);
    assert!(
        game.hero(0)
            .effects
            .iter()
            .any(|held| held.id == bota_proto::EffectId::GHOST_FORM)
    );
    let target = game.hero(0).id;
    let damage = |game: &Match| {
        game.facts[0].iter().find_map(|event| match event {
            EventKind::Damaged {
                target: hit,
                amount,
                ..
            } if *hit == target => Some(*amount),
            _ => None,
        })
    };
    game.cheat(
        0,
        Cheat::Damage {
            target: Target::None,
            amount: 100,
            kind: DamageKind::Physical,
        },
    );
    assert_eq!(
        damage(&game),
        None,
        "shared ghost form rejects physical damage"
    );
    let through = rules::NOMINAL_BP - bota_core::magic_resist_bp(plain_resist);
    let vulnerability = -source("item_crellas_crozier", "extra_spell_damage_percent").int();
    game.cheat(
        0,
        Cheat::Damage {
            target: Target::None,
            amount: 100,
            kind: DamageKind::Magical,
        },
    );
    let expected = 100_i64 * i64::from(through) * i64::from(100 + vulnerability)
        / (100 * i64::from(rules::NOMINAL_BP));
    assert_eq!(
        damage(&game),
        Some(expected as i32),
        "the originating item supplies ghost's vulnerability"
    );
    game.cheat(
        0,
        Cheat::SpellImmunity {
            target: Target::None,
            ticks: rules::TICKS_PER_SECOND,
        },
    );
    game.advance(1);
    assert!(game.hero(0).statuses.bits & StatusFlags::MAGIC_IMMUNE != 0);
    game.cheat(
        0,
        Cheat::Damage {
            target: Target::None,
            amount: 100,
            kind: DamageKind::Physical,
        },
    );
    assert!(
        damage(&game).is_some_and(|amount| amount > 0),
        "Dota7.39e shared ghost is ineffective while immune"
    );
    let collector = bota_core::item_def(item("item_crellas_crozier"))
        .expect("item")
        .modifiers
        .iter()
        .find(|template| template.collection_period.is_some())
        .expect("collector")
        .effect;
    assert!(
        game.hero(0).effects.iter().any(|held| held.id == collector),
        "the independent active remains under the setup predicate"
    );
}

#[test]
fn crozier_and_ghost_scepter_share_their_imported_cooldown_group() {
    use bota_proto::{ItemSlot, RejectReason, Target};
    let mut game = Match::new();
    give(&mut game, 0, "item_crellas_crozier");
    give(&mut game, 0, "item_ghost");
    use_first(&mut game, 0);
    assert!(game.hero(0).items[1].expect("ghost held").cooldown_left > 0);
    assert_eq!(
        game.try_order(
            0,
            Order::Use {
                slot: ItemSlot(1),
                target: Target::None
            }
        ),
        vec![RejectReason::OnCooldown]
    );
    game.cheat(0, Cheat::Refresh);
    game.order(
        0,
        Order::Use {
            slot: ItemSlot(1),
            target: Target::None,
        },
    );
    assert!(game.hero(0).items[0].expect("crozier held").cooldown_left > 0);
    assert_eq!(
        game.try_order(
            0,
            Order::Use {
                slot: ItemSlot(0),
                target: Target::None
            }
        ),
        vec![RejectReason::OnCooldown]
    );
}
fn crozier_refresh_trial(with_heal: bool) -> (i32, u32) {
    use bota_proto::{EventKind, Inflictor, Target};
    let mut game = Match::new();
    give(&mut game, 0, "item_crellas_crozier");
    give(&mut game, 2, "item_mekansm");
    game.cheat(
        0,
        Cheat::SetPools {
            health: 10,
            mana: 100,
        },
    );
    game.cheat(
        2,
        Cheat::SetPools {
            health: 25,
            mana: 100,
        },
    );
    use_first(&mut game, 0);
    let started = game.game.tick();
    let period = source("item_crellas_crozier", "interval").secs().ticks();
    if with_heal {
        use_first(&mut game, 2);
    } else {
        game.order(
            2,
            Order::Move {
                target: Target::None,
            },
        );
    }
    game.advance(period / 2 - 1);
    game.cheat(0, Cheat::Refresh);
    game.cheat(
        0,
        Cheat::SetPools {
            health: 10,
            mana: 100,
        },
    );
    use_first(&mut game, 0);
    let receiver = game.hero(0).id;
    while game.game.tick() - started <= period {
        if let Some(amount) = game.facts[0].iter().find_map(|event| match event {
            EventKind::Healed {
                target,
                amount,
                inflictor: Some(Inflictor::Item(used)),
                ..
            } if *target == receiver && *used == item("item_crellas_crozier") => Some(*amount),
            _ => None,
        }) {
            return (amount, game.game.tick() - started);
        }
        game.advance(1);
    }
    panic!("refresh lost the original collection phase/pending restoration");
}

#[test]
fn crozier_recasting_preserves_the_pending_restoration_and_original_interval_phase() {
    let (control, control_at) = crozier_refresh_trial(false);
    let (healed, healed_at) = crozier_refresh_trial(true);
    let period = source("item_crellas_crozier", "interval").secs().ticks();
    assert_eq!(
        control_at, period,
        "native refresh leaves the scheduler phase untouched"
    );
    assert_eq!(healed_at, period);
    let pending = source("item_mekansm", "heal_amount").int()
        * source("item_crellas_crozier", "active_health_steal").int()
        / 100;
    assert!(
        (healed - control - pending).abs() <= 1,
        "native refresh preserves the existing pending pool"
    );
}

#[test]
fn crozier_recasting_does_not_run_an_extra_creation_movement_pulse() {
    let mut game = Match::new();
    give(&mut game, 0, "item_crellas_crozier");
    let def = bota_core::item_def(item("item_crellas_crozier")).expect("item");
    let steal = def
        .modifiers
        .iter()
        .find_map(|template| template.movement_steal)
        .expect("movement transfer");
    let debuff = def.modifiers[usize::from(steal.debuff)].effect;
    use_first(&mut game, 0);
    game.cheat(0, Cheat::Refresh);
    use_first(&mut game, 0);
    let receiver = game
        .hero(2)
        .effects
        .iter()
        .find(|held| held.id == debuff)
        .expect("receiver");
    assert_eq!(
        receiver.stacks,
        Some(1),
        "OnRefresh reloads parameters without another OnCreated pulse"
    );
}

fn crozier_expiry_trial(with_heal: bool, refreshed: bool) -> (i32, bool) {
    use bota_proto::Target;
    let mut game = Match::new();
    give(&mut game, 0, "item_crellas_crozier");
    give(&mut game, 2, "item_mekansm");
    game.cheat(
        0,
        Cheat::SetPools {
            health: 10,
            mana: 100,
        },
    );
    game.cheat(
        2,
        Cheat::SetPools {
            health: 25,
            mana: 100,
        },
    );
    use_first(&mut game, 0);
    let first = game.game.tick();
    let period = source("item_crellas_crozier", "interval").secs().ticks();
    if refreshed {
        game.advance(period / 2);
        game.cheat(0, Cheat::Refresh);
        game.cheat(
            0,
            Cheat::SetPools {
                health: 10,
                mana: 100,
            },
        );
        game.cheat(
            2,
            Cheat::SetPools {
                health: 25,
                mana: 100,
            },
        );
        use_first(&mut game, 0);
    }
    let duration = source("item_crellas_crozier", "duration").secs().ticks();
    let expiry = game.game.tick() + duration;
    assert_eq!(
        (expiry - first).is_multiple_of(period),
        !refreshed,
        "distinguish a due interval from a partial remainder"
    );
    game.advance(duration - 2);
    let before = game.hero(0).hp;
    if with_heal {
        use_first(&mut game, 2);
    } else {
        game.order(
            2,
            Order::Move {
                target: Target::None,
            },
        );
    }
    game.advance(1);
    assert_eq!(game.game.tick(), expiry);
    let active = bota_core::item_def(item("item_crellas_crozier"))
        .expect("item")
        .modifiers
        .iter()
        .find(|template| template.collection_period.is_some())
        .expect("collector")
        .effect;
    (
        game.hero(0).hp - before,
        game.hero(0).effects.iter().any(|held| held.id == active),
    )
}

#[test]
fn crozier_runs_a_due_collection_before_its_natural_expiry() {
    let (control, _) = crozier_expiry_trial(false, false);
    let (collected, active) = crozier_expiry_trial(true, false);
    assert!(
        !active,
        "the interval does not extend the active's lifetime"
    );
    let expected = source("item_mekansm", "heal_amount").int()
        * source("item_crellas_crozier", "active_health_steal").int()
        / 100;
    assert!(
        (collected - control - expected).abs() <= 1,
        "a due interval runs before expiry cleanup"
    );
}

#[test]
fn crozier_does_not_flush_a_partial_collection_interval_on_removal() {
    let (control, _) = crozier_expiry_trial(false, true);
    let (collected, active) = crozier_expiry_trial(true, true);
    assert!(!active);
    assert_eq!(
        collected, control,
        "native removal hooks do not pay an unfinished interval"
    );
}

#[test]
fn crozier_snapshots_its_resolved_active_radius_until_the_active_is_refreshed() {
    use bota_proto::{ItemSlot, Target};
    let mut game = Match::new();
    give(&mut game, 0, "item_crellas_crozier");
    give(&mut game, 0, "item_chasm_stone");
    let def = bota_core::item_def(item("item_crellas_crozier")).expect("item");
    let steal = def
        .modifiers
        .iter()
        .find_map(|template| template.movement_steal)
        .expect("movement transfer");
    let bonus = bota_core::item_def(item("item_chasm_stone"))
        .expect("stone")
        .carried
        .aoe_bonus;
    assert!(bonus > 2);
    let at = game.hero(0).pos + Vec2::from_ints(steal.radius + bonus / 2, 0);
    game.cheat(2, Cheat::Teleport { at });
    game.order(
        2,
        Order::Move {
            target: Target::None,
        },
    );
    assert_eq!(
        game.hero(2).pos,
        at,
        "annulus fixture must not be clamped by terrain"
    );
    let debuff = def.modifiers[usize::from(steal.debuff)].effect;
    let count = |game: &Match| {
        game.hero(2)
            .effects
            .iter()
            .find(|held| held.id == debuff)
            .and_then(|held| held.stacks)
    };
    use_first(&mut game, 0);
    assert_eq!(
        count(&game),
        Some(1),
        "the creation pulse uses the resolved widened radius"
    );
    game.order(
        0,
        Order::Swap {
            from: ItemSlot(1),
            to: ItemSlot(rules::INVENTORY_SLOTS as u8),
        },
    );
    game.advance(steal.period);
    assert_eq!(
        count(&game),
        Some(2),
        "the active reuses its resolved radius after the area bonus is backpacked"
    );
    game.cheat(0, Cheat::Refresh);
    use_first(&mut game, 0);
    game.advance(steal.period);
    assert_eq!(
        count(&game),
        Some(2),
        "refresh resolves the current smaller radius without an extra creation pulse"
    );
}

#[test]
fn crozier_activation_changes_an_enemy_heal_later_in_the_same_tick() {
    use bota_proto::{EventKind, Inflictor, ItemSlot, Target};
    let requested = source("item_mekansm", "heal_amount").int();
    let redirected = source("item_crellas_crozier", "active_health_steal").int();
    let expected = requested * (100 - redirected) / 100;
    let mut exercised = false;
    for idle in 0..8 {
        let mut game = Match::new();
        give(&mut game, 0, "item_crellas_crozier");
        give(&mut game, 2, "item_mekansm");
        game.cheat(
            2,
            Cheat::SetPools {
                health: 25,
                mana: 100,
            },
        );
        game.advance(idle);
        let tick = game.game.tick();
        let uses = [0, 2].map(|seat| {
            [OutgoingOrder {
                seq: tick + seat,
                seen: tick,
                unit: None,
                queue: false,
                order: Order::Use {
                    slot: ItemSlot(0),
                    target: Target::None,
                },
            }]
        });
        let streams = game
            .game
            .step(&[&uses[0], &[], &uses[1]])
            .expect("simultaneous uses");
        let facts = event_facts(&streams);
        let source_used = facts[0].iter().position(|fact| {
            matches!(fact,
            EventKind::ItemUsed { item: used, .. } if *used == item("item_crellas_crozier"))
        });
        let recovered = facts[0]
            .iter()
            .enumerate()
            .find_map(|(at, fact)| match fact {
                EventKind::Healed {
                    amount,
                    inflictor: Some(Inflictor::Item(used)),
                    ..
                } if *used == item("item_mekansm") => Some((at, *amount)),
                _ => None,
            });
        if let (Some(source_used), Some((healed_at, healed))) = (source_used, recovered)
            && source_used < healed_at
        {
            exercised = true;
            assert!(
                (healed - expected).abs() <= 1,
                "idle={idle}: activation precedes the heal in one tick; got={healed}, expected={expected}"
            );
        }
    }
    assert!(
        exercised,
        "the deterministic trials include activation before healing"
    );
}

#[test]
fn magical_item_effects_respect_immunity_and_blasts_leave_allies_untouched() {
    use bota_proto::{DamageKind, EventKind, Inflictor, Target};
    for key in ["item_radiance", "item_shivas_guard"] {
        let mut game = Match::new();
        give(&mut game, 0, key);
        for seat in [1, 2] {
            game.cheat(
                seat,
                Cheat::SetPools {
                    health: 50,
                    mana: 100,
                },
            );
        }
        let enemy = game.hero(2).id;
        let ally = game.hero(1).id;
        game.cheat(
            0,
            Cheat::SpellImmunity {
                target: Target::Unit(enemy),
                ticks: 1000,
            },
        );
        let before = game.hero(2).hp;
        let def = bota_core::item_def(item(key)).expect("item");
        let horizon = match def.effect {
            bota_core::UseSpec::Wave(wave) => {
                use_first(&mut game, 0);
                (wave.radius as u32 * rules::TICKS_PER_SECOND).div_ceil(wave.speed as u32) + 2
            }
            _ => {
                def.modifiers
                    .iter()
                    .find_map(|m| m.pulse)
                    .expect("magic pulse")
                    .period
                    * 2
            }
        };
        for _ in 0..horizon {
            for facts in &game.facts {
                assert!(!facts.iter().any(|fact| matches!(fact,
                    EventKind::Damaged { target, kind: DamageKind::Magical, inflictor: Some(Inflictor::Item(used)), .. }
                    if (*target == enemy || *target == ally) && *used == item(key))),
                    "{key}: no magical damage to an immune enemy or an ally");
            }
            game.advance(1);
        }
        assert!(game.hero(2).hp >= before, "{key}: immunity blocks damage");
    }
}

#[test]
fn a_blast_damages_an_enemy_but_never_an_ally_inside_the_same_radius() {
    use bota_proto::{EventKind, Inflictor};
    let mut game = Match::new();
    give(&mut game, 0, "item_shivas_guard");
    let enemy = game.hero(2).id;
    let ally = game.hero(1).id;
    let bota_core::UseSpec::Wave(wave) = bota_core::item_def(item("item_shivas_guard"))
        .expect("item")
        .effect
    else {
        panic!("wave")
    };
    use_first(&mut game, 0);
    let mut enemy_hit = false;
    for _ in 0..(wave.radius as u32 * rules::TICKS_PER_SECOND).div_ceil(wave.speed as u32) + 2 {
        for fact in &game.facts[0] {
            if let EventKind::Damaged {
                target,
                inflictor: Some(Inflictor::Item(used)),
                ..
            } = fact
                && *used == item("item_shivas_guard")
            {
                assert_ne!(*target, ally, "the blast never hits an ally");
                enemy_hit |= *target == enemy;
            }
        }
        game.advance(1);
    }
    assert!(enemy_hit, "the enemy control must be damaged");
}

#[test]
fn crozier_steals_from_a_normal_enemy_but_not_a_spell_immune_enemy() {
    use bota_proto::Target;
    for immune in [false, true] {
        let mut game = Match::new();
        give(&mut game, 0, "item_crellas_crozier");
        let def = bota_core::item_def(item("item_crellas_crozier")).expect("item");
        let steal = def
            .modifiers
            .iter()
            .find_map(|m| m.movement_steal)
            .expect("steal");
        let target = game.hero(2).id;
        if immune {
            game.cheat(
                0,
                Cheat::SpellImmunity {
                    target: Target::Unit(target),
                    ticks: 1000,
                },
            );
        }
        use_first(&mut game, 0);
        for _ in 0..=steal.period {
            let held = game
                .hero(2)
                .effects
                .iter()
                .find(|e| e.id == def.modifiers[usize::from(steal.debuff)].effect);
            assert_eq!(held.is_some(), !immune, "movement steal immune={immune}");
            game.advance(1);
        }
    }
}

#[test]
fn full_pool_support_never_emits_zero_health_and_zero_mana_restoration() {
    for key in ["item_mekansm", "item_arcane_boots", "item_guardian_greaves"] {
        let mut game = Match::new();
        give(&mut game, 0, key);
        use_first(&mut game, 0);
        assert!(
            game.facts.iter().flatten().all(|fact| !matches!(
                fact,
                bota_proto::EventKind::Healed {
                    amount: 0,
                    mana: 0,
                    ..
                }
            )),
            "{key}: empty restoration is not a fact"
        );
    }
}

#[test]
fn item_state_is_never_repeated_to_the_owner_or_a_viewer_in_fog() {
    let mut game = Match::new();
    give(&mut game, 0, "item_radiance");
    game.cheat(
        0,
        Cheat::Teleport {
            at: bota_core::fountain_pos(bota_core::map_of(MapId(2)), Team::Radiant),
        },
    );
    game.cheat(0, Cheat::AllVision { on: false });
    let carrier = game.game.hero_of(0).expect("carrier");
    let viewer = game
        .game
        .world()
        .team
        .get(game.game.hero_of(2).expect("viewer"))
        .copied()
        .expect("team");
    assert!(
        !game
            .game
            .world()
            .view(viewer)
            .units
            .iter()
            .any(|unit| unit.id == bota_server::game::wire_id(carrier))
    );
    for _ in 0..3 {
        game.advance(1);
        for facts in [&game.facts[0], &game.facts[2]] {
            assert!(
                !facts
                    .iter()
                    .any(|fact| matches!(fact, bota_proto::EventKind::RetiredItemState { .. })),
                "retired state events are not publication state, even for the owner"
            );
        }
    }
}
