mod support;

use bota_client_core::{ClientSession, MatchStatic, SessionOptions};
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Fixed, HeroId, ItemId, SlotId, Team};
use bota_server::game::{HeldRune, ItemStack};

fn castable(item: ItemId, charges: u8, rune: bool, mana: Fixed, expected: bool) {
    castable_at_rate(item, charges, rune, mana, 10_000, expected);
}

fn castable_at_rate(item: ItemId, charges: u8, rune: bool, mana: Fixed, rate: i32, expected: bool) {
    let (mut local, opening) = support::opening(&[Team::Radiant, Team::Dire]);
    let hero = local.hero_of(0).expect("hero");
    let mut stack = ItemStack::bought(item, SlotId(0), 0).expect("item");
    stack.charges = charges;
    if rune {
        stack.rune = Some(HeldRune {
            kind: bota_proto::RuneKind::Haste,
            ticks_left: 100,
            gold: 0,
            was_full: false,
        });
    }
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[0] = Some(stack);
    local.world_mut().seats[0].item_clocks.clear();
    local.world_mut().mana.get_mut(hero).expect("mana").mana = mana;
    local
        .world_mut()
        .stats
        .get_mut(hero)
        .expect("stats")
        .mana_cost_rate_bp = rate;
    let session = session_from(&mut local, &opening[0]);
    if rate != 10_000 {
        let shown = session.own_hero().expect("own hero");
        assert_eq!(
            shown.items[0].as_ref().expect("scroll").cooldown_left,
            0,
            "no shared startup cooldown"
        );
        assert_eq!(shown.mana, 1, "fixture shows a rounded fractional pool");
        assert_eq!(
            shown.items[0].as_ref().expect("scroll").mana_cost,
            1,
            "fixture must exercise a one-point cost"
        );
    }
    let source = format!(
        "function Think() assert(GetBot():GetItemInSlot(0):IsFullyCastable() == {expected}, 'castability contract') end"
    );
    assert_lua(&session, HeroId::SHADOW_FIEND, source.as_bytes());
}

#[test]
fn fully_castable_rejects_a_zero_charge_stick() {
    castable(ItemId::MAGIC_STICK, 0, false, Fixed::from_int(100), false);
}

#[test]
fn fully_castable_accepts_one_charge_and_a_stored_rune_without_charges() {
    castable(ItemId::MAGIC_STICK, 1, false, Fixed::from_int(100), true);
    castable(ItemId::BOTTLE, 0, true, Fixed::from_int(100), true);
}

#[test]
fn fully_castable_rejects_mango_at_full_mana() {
    // The fixture hero's full pool is supplied directly, without a tick of regeneration.
    let (local, _) = support::opening(&[Team::Radiant, Team::Dire]);
    let hero = local.hero_of(0).expect("hero");
    let maximum = local.world().stats.get(hero).expect("stats").max_mana;
    castable(ItemId::MANGO, 1, false, maximum, false);
}

#[test]
fn fully_castable_does_not_prove_one_mana_from_a_rounded_fraction() {
    castable_at_rate(
        ItemId::TOWN_PORTAL_SCROLL,
        1,
        false,
        Fixed::from_ratio(1, 2),
        200,
        false,
    );
}

#[test]
fn fully_castable_keeps_a_ready_ability_available_when_hidden_control_is_unknown() {
    let (mut local, mut sessions) = support::local_with_modifiers(
        &[
            (Team::Radiant, HeroId::PUDGE),
            (Team::Dire, HeroId::SHADOW_FIEND),
        ],
        Vec::new(),
    );
    let hero = local.hero_of(0).expect("hero");
    local
        .world_mut()
        .abilities
        .get_mut(hero)
        .expect("book")
        .slots[1]
        .level = 1;
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[], &[]]).expect("step"))
    {
        support::fold(session, stream);
    }
    assert_lua(&sessions[0], HeroId::PUDGE, br#"
        function Think()
            assert(GetBot():GetAbilityByName('pudge_rot'):IsFullyCastable(), 'unknown body control is not a permanent disable')
        end
    "#);
}

#[test]
fn fully_castable_rejects_illusion_abilities_and_copied_inventory() {
    let (mut local, opening) = support::opening(&[Team::Radiant, Team::Dire]);
    let hero = local.hero_of(0).expect("hero");
    local
        .world_mut()
        .abilities
        .get_mut(hero)
        .expect("book")
        .slots[0]
        .level = 1;
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[0] = Some(ItemStack::granted(ItemId::MAGIC_STICK, SlotId(0), 0, 1));
    local.world_mut().put_modifier(
        hero,
        bota_server::game::Modifier::new(bota_core::ModifierKind::Illusion, None, None, Some(1000)),
    );
    let session = session_from(&mut local, &opening[0]);
    assert_lua(&session, HeroId::SHADOW_FIEND, br#"
        function Think()
            local bot = GetBot()
            assert(not bot:GetAbilityByName('nevermore_shadowraze1'):IsFullyCastable(), 'illusion ability')
            assert(not bot:GetItemInSlot(0):IsFullyCastable(), 'copied inventory')
        end
    "#);
}

fn session_from(
    local: &mut bota_server::local::LocalMatch,
    opening: &[bota_proto::ServerMsg],
) -> ClientSession {
    let bota_proto::ServerMsg::MatchStart { info } = &opening[0] else {
        panic!("start")
    };
    let mut session = ClientSession::seat(
        MatchStatic::new(info).expect("statics"),
        SlotId(0),
        Some(HeroId::SHADOW_FIEND),
        SessionOptions::default(),
    )
    .expect("session");
    for message in local.restate().remove(0) {
        session.ingest(message, &mut ()).expect("snapshot");
    }
    session
}

fn assert_lua(session: &ClientSession, hero: HeroId, source: &[u8]) {
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "castability", source)
        .expect("script");
    let turn = lua.think(session).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}

fn session_with_view(mut change: impl FnMut(&mut bota_proto::UnitView)) -> ClientSession {
    let (mut local, opening) = support::opening(&[Team::Radiant, Team::Dire]);
    let hero = local.hero_of(0).expect("hero");
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[0] = ItemStack::bought(ItemId::BLOOD_GRENADE, SlotId(0), 0);
    let mut session = session_from(&mut local, &opening[0]);
    let id = session.own_hero().expect("hero").id;
    let mut view = session.current().expect("view").clone();
    view.tick += 1;
    change(
        view.units
            .iter_mut()
            .find(|unit| unit.id == id)
            .expect("own body"),
    );
    session
        .ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ())
        .expect("view ingestion");
    session
}

#[test]
fn fully_castable_rejects_negative_mana_costs_from_a_server_view() {
    let session = session_with_view(|unit| {
        unit.abilities[0].level = 1;
        unit.abilities[0].cooldown_left = 0;
        unit.abilities[0].mana_cost = -1;
        unit.items[0].as_mut().expect("grenade").mana_cost = -1;
    });
    assert_lua(
        &session,
        HeroId::SHADOW_FIEND,
        br#"
        function Think()
            assert(not GetBot():GetAbilityByName('nevermore_shadowraze1'):IsFullyCastable())
            assert(not GetBot():GetItemInSlot(0):IsFullyCastable())
        end
    "#,
    );
}

#[test]
fn fully_castable_respects_an_abilitys_cast_while_stunned_exception() {
    let session = session_with_view(|unit| {
        let ability = &mut unit.abilities[0];
        ability.id = bota_proto::AbilityId(85);
        ability.level = 1;
        ability.cooldown_left = 0;
        ability.mana_cost = 0;
        unit.statuses.bits |= bota_proto::StatusFlags::STUNNED;
    });
    assert_lua(
        &session,
        HeroId::SHADOW_FIEND,
        br#"
        function Think()
            assert(GetBot():GetAbilityByName('roshan_revengeroar'):IsFullyCastable())
        end
    "#,
    );
}

#[test]
fn available_damage_refuses_unproven_resources_and_disabled_casters() {
    for item in [false, true] {
        for state in 0..4 {
            let session = session_with_view(|unit| {
                unit.mana = if state == 0 { 1 } else { 100 };
                if state == 1 {
                    unit.statuses.bits |= bota_proto::StatusFlags::STUNNED;
                }
                if state == 3 {
                    unit.effects.push(bota_proto::EffectView {
                        id: bota_proto::EffectId::ILLUSION,
                        ticks_left: Some(100),
                        ticks_total: Some(100),
                        stacks: None,
                        caster: None,
                        inflictor: None,
                    });
                }
                let cost = if state == 2 { -1 } else { 1 };
                for ability in &mut unit.abilities {
                    ability.level = 0;
                }
                if item {
                    let mut grenade = unit.items[0].expect("catalogue grenade");
                    grenade.id = ItemId::BLOOD_GRENADE;
                    grenade.aim = Some(bota_proto::Aim::Point);
                    grenade.charges = Some(1);
                    grenade.cooldown_left = 0;
                    grenade.mute_left = 0;
                    grenade.mana_cost = cost;
                    unit.items[0] = Some(grenade);
                } else {
                    unit.items.fill(None);
                    unit.abilities[0].level = 1;
                    unit.abilities[0].cooldown_left = 0;
                    unit.abilities[0].mana_cost = cost;
                }
            });
            assert_lua(
                &session,
                HeroId::SHADOW_FIEND,
                br#"
            function Think()
                local bot = GetBot()
                assert(bot:GetEstimatedDamageToTarget(true, bot, 2, DAMAGE_TYPE_MAGICAL) == 0)
                assert(bot:GetEstimatedDamageToTarget(false, bot, 2, DAMAGE_TYPE_MAGICAL) > 0)
            end
        "#,
            );
        }
    }
}

#[test]
fn available_damage_includes_passive_continuous_projections_even_when_the_caster_is_disabled() {
    use bota_core::NeutralAbility;
    use bota_proto::StatusFlags;
    // Envenomed Weapon level 2: 20 magical DPS for 2 seconds. Petrify level 1:
    // 100 physical DPS for 1.2 seconds below 90% HP. No mitigation in this fixture.
    for (id, level, mask, expected) in [
        (NeutralAbility::EnvenomedWeapon.id(), 2, 2, 40),
        (NeutralAbility::Petrify.id(), 1, 1, 120),
    ] {
        for disabled in [0, StatusFlags::STUNNED, StatusFlags::FEARED] {
            let session = session_with_view(|unit| {
                let mut ability = unit.abilities[0];
                ability.id = id;
                ability.level = level;
                ability.passive = true;
                ability.cooldown_left = 0;
                ability.mana_cost = 0;
                unit.abilities = vec![ability];
                unit.items.fill(None);
                unit.effects.clear();
                unit.hp = 50;
                unit.max_hp = 100;
                unit.mana = 0;
                unit.attack_damage = 0;
                unit.armor = Fixed::from_int(0);
                unit.magic_resist = Fixed::from_int(0);
                unit.statuses.bits = disabled;
            });
            let source = format!(
                r#"
                function Think()
                    local bot = GetBot()
                    assert(bot:GetEstimatedDamageToTarget(false, bot, 2, {mask}) == {expected}, 'passive fixture projection')
                    assert(bot:GetEstimatedDamageToTarget(true, bot, 2, {mask}) == {expected}, 'passive damage must not require cast readiness')
                end
            "#
            );
            assert_lua(&session, HeroId::SHADOW_FIEND, source.as_bytes());
        }
    }
}

#[test]
fn available_damage_includes_a_ready_grenade_but_excludes_disables_and_missing_charges() {
    use bota_proto::StatusFlags;
    for (disabled, charges, expected) in [
        (0, Some(1), 65),
        (StatusFlags::STUNNED, Some(1), 0),
        (StatusFlags::FEARED, Some(1), 0),
        (0, None, 0),
    ] {
        let session = session_with_view(|unit| {
            let mut grenade = unit.items[0].expect("grenade");
            grenade.charges = charges;
            grenade.cooldown_left = 0;
            grenade.mute_left = 0;
            grenade.mana_cost = 0;
            unit.items.fill(None);
            unit.items[0] = Some(grenade);
            unit.abilities.clear();
            unit.effects.clear();
            unit.magic_resist = Fixed::from_int(0);
            unit.statuses.bits = disabled;
        });
        // At 2 seconds, the 0.2-second cast leaves one burn tick: 50 + 15 damage.
        let source = format!(
            r#"
            function Think()
                local bot = GetBot()
                assert(bot:GetEstimatedDamageToTarget(false, bot, 2, DAMAGE_TYPE_MAGICAL) == 65, 'grenade damage fixture')
                assert(bot:GetEstimatedDamageToTarget(true, bot, 2, DAMAGE_TYPE_MAGICAL) == {expected}, 'grenade availability')
                assert(bot:GetItemInSlot(0):IsFullyCastable() == {castable}, 'grenade readiness')
            end
        "#,
            castable = expected != 0
        );
        assert_lua(&session, HeroId::SHADOW_FIEND, source.as_bytes());
    }
}
