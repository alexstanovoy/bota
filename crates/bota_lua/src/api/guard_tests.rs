//! Bounds, fog filters and argument strictness of the shared-query calls, each pinned so that
//! removing the guard fails a test. The snapshot is edited directly: these are host contracts
//! over what the seat was told, not simulation scenarios.
use super::*;
use crate::{
    BotSeat, Keys, Limits,
    state::{KnownUnit, Snapshot},
};
use bota_proto::{EntityId, HeroId, SlotId, UnitKind, Vec2};

mod support {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs"));
}

pub(super) struct Rig {
    pub(super) lua: Lua,
    pub(super) state: Shared,
    pub(super) vectors: Vectors,
}

impl Rig {
    /// A rig over the opening snapshot of a duel, after `edit` told the seat more.
    pub(super) fn new(edit: impl FnOnce(&mut Snapshot, EntityId)) -> Self {
        let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let own = sessions[0].own_hero().expect("own hero").id;
        let mut snapshot = Snapshot::capture(&sessions[0]).expect("snapshot");
        edit(&mut snapshot, own);
        let state = Rc::new(RefCell::new(State::new(
            Team::Radiant,
            &[BotSeat {
                slot: SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            }],
        )));
        state.borrow_mut().statistics = Some(crate::statistics::collector());
        state.borrow_mut().prepare_sight(&snapshot);
        state.borrow_mut().snapshot = Some(snapshot);
        let mut lua = Lua::new(
            Limits::DEFAULT.with_fuel_per_call(Limits::DEFAULT.fuel_per_tick),
            Keys {
                hash: [17, 19],
                random: 23,
            },
        )
        .expect("interpreter");
        let vectors = vector::install(&mut lua).expect("vectors");
        install(&mut lua, state.clone(), vectors).expect("API");
        lua.collect().expect("setup collection");
        Self {
            lua,
            state,
            vectors,
        }
    }

    /// Runs a chunk; its first result is returned.
    pub(super) fn run(&mut self, source: &str) -> Result<Value, luagee::Error> {
        let program = self.lua.load("guard", source.as_bytes()).expect("chunk");
        self.lua.begin_tick();
        let result = self.lua.call(program, &[]);
        self.state.borrow_mut().flush_actions(0, false);
        let results = result?;
        Ok(results.first().copied().unwrap_or(Value::Nil))
    }

    /// The error text of a chunk that must fail.
    fn error(&mut self, source: &str) -> String {
        match self.run(source) {
            Err(error) => error.to_string(),
            Ok(value) => panic!("expected an error from {source}, got {value:?}"),
        }
    }

    fn number(&mut self, source: &str) -> f64 {
        match self.run(source) {
            Ok(Value::Number(value)) => value,
            other => panic!("expected a number from {source}, got {other:?}"),
        }
    }

    pub(super) fn declined(&self) -> Vec<(String, u64)> {
        self.state
            .borrow()
            .statistics
            .as_ref()
            .expect("measured")
            .borrow()
            .declined_api_calls
            .names
            .iter()
            .map(|entry| (entry.name.clone(), entry.reads))
            .collect()
    }

    /// The seat stops seeing the body, as when it walks into the fog; scripts keep their handles.
    pub(super) fn hide(&mut self, idx: u32) {
        self.state
            .borrow_mut()
            .snapshot
            .as_mut()
            .expect("snapshot")
            .units
            .iter_mut()
            .find(|known| known.unit.id.idx == idx)
            .expect("told body")
            .visible = false;
    }

    fn orders(&self) -> usize {
        self.state.borrow().orders.len()
    }
}

/// Power Treads told with the given selected attribute (none: the seat was not told it).
pub(super) fn treads_item(mode: Option<bota_proto::Attribute>) -> bota_proto::ItemView {
    bota_proto::ItemView {
        id: bota_proto::ItemId::POWER_TREADS,
        charges: None,
        cooldown_left: 0,
        mute_left: 0,
        mode,
        mana_cost: 0,
        range: 0,
        aim: None,
        for_sale: false,
        owner: SlotId(1),
        selling: None,
        dispenser: None,
        locked: None,
        disassembly: None,
        rune: None,
        expires_in: None,
        level: 0,
    }
}

/// A body told to the seat beside the hero, `by` units to the east of it.
pub(super) fn add_body(
    snapshot: &mut Snapshot,
    own: EntityId,
    id: u32,
    kind: UnitKind,
    team: Team,
    by: i32,
) {
    let hero = snapshot
        .units
        .iter()
        .find(|known| known.unit.id == own)
        .expect("own hero");
    let mut unit = hero.unit.clone();
    unit.id = EntityId {
        idx: id,
        generation: 1,
    };
    unit.kind = kind;
    unit.team = team;
    unit.hero = None;
    unit.owner = None;
    unit.abilities.clear();
    unit.items.clear();
    unit.effects.clear();
    unit.pos = hero.unit.pos + Vec2::from_ints(by, 0);
    let known = KnownUnit {
        damage_dealt_at: None,
        direction_history: Default::default(),
        last_visible: (snapshot.tick, unit.pos),
        unit,
        visible: true,
        velocity: None,
        activity: Default::default(),
        recent_damage: Default::default(),
        attack_target: None,
    };
    snapshot.units.push(known);
    // Handles look bodies up by binary search, as a captured snapshot is sorted.
    snapshot.units.sort_by_key(|known| known.unit.id);
}

/// Every key `declined_api_calls` can hold, sorted; API.md lists the same ones.
pub(super) const DECLINED_KEYS: [&str; 12] = [
    "GetActualIncomingDamage_target_not_seen",
    "GetAttackTarget_target_not_seen",
    "GetNearbyLaneCreeps_not_a_bot",
    "GetNearbyLaneCreeps_not_seen",
    "GetNearbyNeutralCreeps_not_a_bot",
    "GetNearbyNeutralCreeps_not_seen",
    "GetPowerTreadsStat_item_not_seen",
    "GetPowerTreadsStat_mode_not_received",
    "GetStashValue_not_a_bot_teammate",
    "GetStashValue_stash_not_received",
    "WasRecentlyDamagedByCreep_not_a_bot_teammate",
    "WasRecentlyDamagedByHero_not_a_bot_teammate",
];

pub(super) const LANE: UnitKind = UnitKind::CreepMelee;

#[test]
fn nearby_creeps_never_reach_beyond_sixteen_hundred_units_whatever_radius_is_asked() {
    let mut rig = Rig::new(|snapshot, own| {
        for (id, by) in [(70_001, 100), (70_002, 1600), (70_003, 1601)] {
            add_body(snapshot, own, id, LANE, Team::Dire, by);
        }
        for (id, by) in [(70_011, 100), (70_012, 1600), (70_013, 1601)] {
            add_body(snapshot, own, id, UnitKind::CreepNeutral, Team::Neutral, by);
        }
    });
    for radius in ["1600", "5000", "1e300"] {
        let lane = format!("return #GetBot():GetNearbyLaneCreeps({radius}, true)");
        assert_eq!(
            rig.number(&lane),
            2.0,
            "lane creeps within 1600 for {radius}"
        );
        let neutral = format!("return #GetBot():GetNearbyNeutralCreeps({radius})");
        assert_eq!(
            rig.number(&neutral),
            2.0,
            "neutrals within 1600 for {radius}"
        );
    }
    assert_eq!(
        rig.number("return #GetBot():GetNearbyLaneCreeps(1599, true)"),
        1.0
    );
    assert_eq!(
        rig.number("return #GetBot():GetNearbyLaneCreeps(-5, true)"),
        0.0
    );
    assert_eq!(
        rig.number("return #GetBot():GetNearbyLaneCreeps(99, true)"),
        0.0
    );
}

#[test]
fn nearby_creeps_skip_bodies_the_seat_no_longer_sees_and_refuse_a_receiver_out_of_view() {
    let mut rig = Rig::new(|snapshot, own| {
        add_body(snapshot, own, 70_001, LANE, Team::Dire, 100);
        add_body(snapshot, own, 70_002, LANE, Team::Dire, 200);
        add_body(
            snapshot,
            own,
            70_011,
            UnitKind::CreepNeutral,
            Team::Neutral,
            100,
        );
        add_body(
            snapshot,
            own,
            70_012,
            UnitKind::CreepNeutral,
            Team::Neutral,
            200,
        );
    });
    rig.run("foe = GetUnitList(UNIT_LIST_ENEMY_CREEPS)[1]")
        .unwrap();
    rig.hide(70_002);
    rig.hide(70_012);
    assert_eq!(
        rig.number("return #GetBot():GetNearbyLaneCreeps(1000, true)"),
        1.0
    );
    assert_eq!(
        rig.number("return #GetBot():GetNearbyNeutralCreeps(1000)"),
        1.0
    );
    assert_eq!(rig.declined(), []);
    rig.hide(70_001);
    assert_eq!(
        rig.number("return #foe:GetNearbyLaneCreeps(1000, true)"),
        0.0,
        "a receiver out of view answers the reference's empty table"
    );
    assert_eq!(
        rig.declined(),
        [("GetNearbyLaneCreeps_not_seen".to_owned(), 1)]
    );
}

#[test]
fn nearby_creeps_answer_the_empty_table_for_a_receiver_that_is_not_a_bot() {
    let mut rig = Rig::new(|snapshot, own| {
        add_body(snapshot, own, 70_001, LANE, Team::Dire, 100);
        add_body(
            snapshot,
            own,
            70_011,
            UnitKind::CreepNeutral,
            Team::Neutral,
            100,
        );
        add_body(snapshot, own, 70_021, LANE, Team::Radiant, 50);
    });
    rig.run(
        "ally = GetUnitList(UNIT_LIST_ALLIED_CREEPS)[1]; foe = GetUnitList(UNIT_LIST_ENEMY_CREEPS)[1]; jungler = GetUnitList(UNIT_LIST_NEUTRAL_CREEPS)[1]",
    )
    .unwrap();
    for receiver in ["ally", "foe", "jungler"] {
        for call in [
            "GetNearbyLaneCreeps(1000, true)",
            "GetNearbyNeutralCreeps(1000)",
        ] {
            assert_eq!(
                rig.number(&format!("return #{receiver}:{call}")),
                0.0,
                "{receiver}:{call}"
            );
        }
    }
    assert_eq!(
        rig.declined(),
        [
            ("GetNearbyLaneCreeps_not_a_bot".to_owned(), 3),
            ("GetNearbyNeutralCreeps_not_a_bot".to_owned(), 3),
        ]
    );
    assert!(rig.number("return #GetBot():GetNearbyLaneCreeps(1000, true)") >= 1.0);
}

#[test]
fn attack_target_extrapolation_and_secret_shop_distance_need_a_body_the_seat_sees() {
    let mut rig = Rig::new(|snapshot, own| {
        add_body(snapshot, own, 70_001, LANE, Team::Dire, 100);
        add_body(snapshot, own, 70_002, LANE, Team::Dire, 200);
        let target = snapshot
            .units
            .iter()
            .find(|known| known.unit.id.idx == 70_002)
            .unwrap()
            .unit
            .id;
        snapshot
            .units
            .iter_mut()
            .find(|known| known.unit.id.idx == 70_001)
            .unwrap()
            .attack_target = Some(target);
    });
    rig.run("foe = GetUnitList(UNIT_LIST_ENEMY_CREEPS)[1]")
        .unwrap();
    for (call, what) in [
        ("foe:GetAttackTarget()", "attack target"),
        ("foe:GetExtrapolatedLocation(1)", "extrapolation"),
        ("foe:DistanceFromSecretShop()", "secret shop distance"),
    ] {
        let source = format!("return {call} ~= nil");
        assert_eq!(
            rig.run(&source),
            Ok(Value::Boolean(true)),
            "{what} of a seen body"
        );
    }
    rig.hide(70_001);
    for (call, what) in [
        ("foe:GetAttackTarget()", "attack target"),
        ("foe:GetExtrapolatedLocation(1)", "extrapolation"),
        ("foe:DistanceFromSecretShop()", "secret shop distance"),
    ] {
        let source = format!("return {call} ~= nil");
        assert_eq!(
            rig.run(&source),
            Ok(Value::Boolean(false)),
            "{what} of an unseen body"
        );
    }
}

#[test]
fn incoming_damage_and_body_definitions_of_a_foreign_unit_need_it_to_be_seen() {
    let mut rig = Rig::new(|snapshot, own| {
        add_body(snapshot, own, 70_001, LANE, Team::Dire, 100);
    });
    rig.run("foe = GetUnitList(UNIT_LIST_ENEMY_CREEPS)[1]")
        .unwrap();
    let calls = [
        "foe:GetActualIncomingDamage(100, DAMAGE_TYPE_PURE)",
        "foe:GetAttackProjectileSpeed()",
        "foe:GetBaseDamageVariance()",
    ];
    for call in calls {
        assert_ne!(
            rig.number(&format!("return {call}")),
            -1.0,
            "{call} of a seen body"
        );
    }
    assert_eq!(rig.declined(), []);
    rig.hide(70_001);
    for call in calls {
        assert_eq!(
            rig.number(&format!("return {call}")),
            -1.0,
            "{call} of an unseen body"
        );
    }
    assert_eq!(
        rig.declined(),
        [("GetActualIncomingDamage_target_not_seen".to_owned(), 1)]
    );
}

#[test]
fn location_visibility_counts_the_vision_of_bodies_in_view_only() {
    let mut rig = Rig::new(|snapshot, own| {
        snapshot.units.retain(|known| known.unit.id == own);
        add_body(snapshot, own, 70_001, LANE, Team::Radiant, 3000);
        let hero_y = snapshot
            .units
            .iter()
            .find(|known| known.unit.id == own)
            .unwrap()
            .unit
            .pos
            .y;
        let creep = snapshot
            .units
            .iter_mut()
            .find(|known| known.unit.id.idx == 70_001)
            .unwrap();
        creep.unit.vision_radius = bota_proto::Fixed::from_int(800);
        creep.unit.pos.y = hero_y + bota_proto::Fixed::from_int(3000);
    });
    let at = "local p = GetBot():GetLocation(); local q = Vector(p.x + 3000, p.y + 3000, 0); return IsLocationVisible(q)";
    assert_eq!(
        rig.run(at),
        Ok(Value::Boolean(true)),
        "an allied body in view sees its surroundings"
    );
    rig.hide(70_001);
    assert_eq!(
        rig.run(at),
        Ok(Value::Boolean(false)),
        "a body remembered from before it left the view no longer sees anything"
    );
}

#[test]
fn power_treads_of_an_unseen_foreign_unit_are_minus_one_and_counted() {
    let mut rig = Rig::new(|snapshot, own| {
        add_body(snapshot, own, 70_001, UnitKind::Hero, Team::Dire, 100);
        let hero = snapshot
            .units
            .iter_mut()
            .find(|known| known.unit.id.idx == 70_001)
            .unwrap();
        hero.unit.owner = Some(SlotId(1));
        hero.unit.hero = Some(HeroId::SHADOW_FIEND);
        hero.unit.items = vec![Some(treads_item(Some(bota_proto::Attribute::Agility)))];
    });
    rig.run("treads = GetUnitList(UNIT_LIST_ENEMY_HEROES)[1]:GetItemInSlot(0)")
        .unwrap();
    assert_eq!(
        rig.number("return treads:GetPowerTreadsStat()"),
        1.0,
        "a seen enemy shows its mode"
    );
    assert_eq!(rig.declined(), []);
    rig.hide(70_001);
    assert_eq!(rig.number("return treads:GetPowerTreadsStat()"), -1.0);
    assert_eq!(
        rig.declined(),
        [("GetPowerTreadsStat_item_not_seen".to_owned(), 1)]
    );
}

#[test]
fn integer_arguments_of_the_shared_queries_reject_fractions_with_the_argument_error() {
    let mut rig = Rig::new(|_, _| {});
    for (call, argument) in [
        ("GetBot():GetActualIncomingDamage(100, 1.5)", 3),
        ("GetBot():GetItemSlotType(1.5)", 2),
        ("GetTreeLocation(1.5)", 1),
        (
            "GetBot():Action_UseAbilityOnTree(GetBot():GetAbilityByName(\"nevermore_shadowraze1\"), 1.5)",
            3,
        ),
    ] {
        let error = rig.error(&format!("return {call}"));
        assert!(
            error.contains(&format!("#{argument}")) && error.contains("integer expected"),
            "{call}: {error}"
        );
    }
    assert_eq!(rig.orders(), 0, "a rejected argument publishes nothing");
}

#[test]
fn tree_location_of_an_unknown_id_is_the_zero_vector_the_reference_returns() {
    let mut rig = Rig::new(|_, _| {});
    let zero = "local v = GetTreeLocation(%) return v.x == 0 and v.y == 0 and v.z == 0";
    for id in ["-1", "100000000", "9007199254740992"] {
        assert_eq!(
            rig.run(&zero.replace('%', id)),
            Ok(Value::Boolean(true)),
            "tree id {id}"
        );
    }
    assert_eq!(
        rig.run(&zero.replace('%', "0")),
        Ok(Value::Boolean(false)),
        "a published tree has a place"
    );
}

#[test]
fn a_movement_path_with_a_hole_is_an_error_and_publishes_nothing() {
    let mut rig = Rig::new(|_, _| {});
    for table in [
        "{Vector(1, 1, 0), nil, Vector(3, 3, 0)}",
        "{nil, Vector(2, 2, 0)}",
        "(function() local t = {Vector(1, 1, 0)}; t[33] = Vector(9, 9, 0); return t end)()",
    ] {
        let error = rig.error(&format!("GetBot():Action_MovePath({table})"));
        assert!(
            error.contains("waypoint table has a hole"),
            "{table}: {error}"
        );
        assert_eq!(rig.orders(), 0, "{table} left a partial path");
    }
    let error = rig.error(
        "local t = {} for i = 1, 33 do t[i] = Vector(i, i, 0) end GetBot():Action_MovePath(t)",
    );
    assert!(error.contains("waypoint limit"), "{error}");
    assert_eq!(rig.orders(), 0);
    rig.run("GetBot():Action_MovePath({Vector(1, 1, 0), Vector(2, 2, 0)})")
        .unwrap();
    assert_eq!(rig.orders(), 1, "only the active path head is published");
    assert_eq!(
        rig.state.borrow().bots[0].actions[0].queued_len(),
        1,
        "the remaining waypoint stays in the client queue"
    );
    assert_eq!(
        rig.state.borrow().bots[0].actions[0].queued_type(0),
        Some(bota_client_core::bot_actions::BotActionType::MoveTo)
    );
}

#[test]
fn an_own_stash_the_seat_never_received_is_nil_and_counted_not_minus_one() {
    let mut rig = Rig::new(|snapshot, own| {
        snapshot
            .players
            .iter_mut()
            .find(|player| player.unit == Some(own))
            .expect("own player")
            .stash = None;
    });
    assert_eq!(rig.run("return GetBot():GetStashValue()"), Ok(Value::Nil));
    assert_eq!(
        rig.declined(),
        [("GetStashValue_stash_not_received".to_owned(), 1)]
    );
}

#[test]
fn ability_points_of_a_hero_the_seat_no_longer_sees_come_from_its_player_record() {
    let mut rig = Rig::new(|snapshot, own| {
        let level = 6;
        snapshot
            .players
            .iter_mut()
            .find(|player| player.unit == Some(own))
            .expect("own player")
            .level = level;
        let known = snapshot
            .units
            .iter_mut()
            .find(|known| known.unit.id == own)
            .unwrap();
        known.unit.level = 1;
        known.visible = false;
    });
    assert_eq!(
        rig.number("return GetBot():GetAbilityPoints()"),
        6.0,
        "six levels and no ability learned: the retained pre-death body says one"
    );
}

#[test]
fn damage_and_definition_queries_agree_that_a_forgotten_handle_is_an_error() {
    let mut rig = Rig::new(|snapshot, own| {
        add_body(snapshot, own, 70_001, LANE, Team::Dire, 100);
    });
    rig.run("foe = GetUnitList(UNIT_LIST_ENEMY_CREEPS)[1]")
        .unwrap();
    rig.state
        .borrow_mut()
        .snapshot
        .as_mut()
        .unwrap()
        .units
        .retain(|known| known.unit.id.idx != 70_001);
    for call in [
        "foe:GetAttackProjectileSpeed()",
        "foe:GetBaseDamageVariance()",
    ] {
        let error = rig.error(&format!("return {call}"));
        assert!(error.contains("unit is no longer known"), "{call}: {error}");
    }
}

#[test]
fn hero_damage_needs_a_current_view_even_for_an_attacker_that_is_not_a_hero() {
    let mut rig = Rig::new(|snapshot, own| {
        add_body(snapshot, own, 70_001, LANE, Team::Dire, 100);
    });
    rig.run("me = GetBot(); foe = GetUnitList(UNIT_LIST_ENEMY_CREEPS)[1]")
        .unwrap();
    rig.state.borrow_mut().snapshot = None;
    let error = rig.error("return me:WasRecentlyDamagedByHero(foe, 5)");
    assert!(error.contains("a current view is required"), "{error}");
}

#[test]
fn an_allied_body_out_of_view_may_still_be_asked_about_but_an_enemy_may_not() {
    let mut rig = Rig::new(|snapshot, own| {
        add_body(snapshot, own, 70_001, LANE, Team::Dire, 100);
        add_body(snapshot, own, 70_021, LANE, Team::Radiant, 50);
    });
    rig.run("ally = GetUnitList(UNIT_LIST_ALLIED_CREEPS)[1]; foe = GetUnitList(UNIT_LIST_ENEMY_CREEPS)[1]")
        .unwrap();
    rig.hide(70_001);
    rig.hide(70_021);
    for call in [
        "GetActualIncomingDamage(100, DAMAGE_TYPE_PURE)",
        "GetAttackProjectileSpeed()",
        "GetBaseDamageVariance()",
    ] {
        assert_ne!(
            rig.number(&format!("return ally:{call}")),
            -1.0,
            "allied body: {call}"
        );
        assert_eq!(
            rig.number(&format!("return foe:{call}")),
            -1.0,
            "enemy body: {call}"
        );
    }
    assert_eq!(rig.run("return ally:GetAttackTarget()"), Ok(Value::Nil));
    assert_eq!(rig.run("return foe:GetAttackTarget()"), Ok(Value::Nil));
    assert_eq!(
        rig.declined(),
        [
            ("GetActualIncomingDamage_target_not_seen".to_owned(), 1),
            ("GetAttackTarget_target_not_seen".to_owned(), 1),
        ],
        "only the enemy body is refused; an allied one is merely not in view"
    );
    assert_eq!(
        rig.number("return #ally:GetNearbyLaneCreeps(1000, true)"),
        0.0
    );
    assert!(
        rig.declined()
            .contains(&("GetNearbyLaneCreeps_not_a_bot".to_owned(), 1)),
        "an allied creep out of view is seen by the gate and refused as a non-bot"
    );
}
