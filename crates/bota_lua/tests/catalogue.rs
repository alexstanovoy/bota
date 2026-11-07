mod support;

use bota_client_core::ClientSession;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{AbilityId, EntityId, HeroId, ServerMsg, SlotId, Team};

fn ability_session(id: AbilityId, level: u8) -> ClientSession {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut session = sessions.remove(0);
    let mut view = session.current().expect("view").clone();
    view.tick += 1;
    let own = session.own_hero().expect("hero").id;
    let actor = view
        .units
        .iter_mut()
        .find(|unit| unit.id == own)
        .expect("actor");
    let mut ability = actor.abilities[0];
    ability.id = id;
    ability.level = level;
    ability.cooldown_left = 0;
    ability.mana_cost = bota_core::ability_mana_cost(id, level);
    actor.abilities = vec![ability];
    actor.mana = actor.max_mana;
    actor.attack_damage = 0;
    let mut target = actor.clone();
    target.id = EntityId {
        idx: 100_000,
        generation: 0,
    };
    target.team = Team::Dire;
    target.owner = None;
    target.abilities.clear();
    target.items.clear();
    target.town_portal = None;
    target.effects.clear();
    target.queue.clear();
    view.units.push(target);
    view.units.sort_by_key(|unit| unit.id);
    let tick = view.tick;
    session
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("catalogue ability view");
    session
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    session
}

fn assert_script(session: &ClientSession, source: &str) {
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
        "catalogue",
        source.as_bytes(),
    )
    .expect("source");
    let turn = lua.think(session).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}

#[test]
fn a_neutral_stomp_exposes_its_core_damage_and_special_values() {
    let ability = bota_core::NeutralAbility::WarStomp;
    let bota_core::Spell::Cast {
        act: bota_core::Act::Stomp { damage, radius, .. },
        ..
    } = ability.spell()
    else {
        panic!("stomp data")
    };
    let session = ability_session(ability.id(), 2);
    assert_script(
        &session,
        &format!(
            "function Think() local a=GetBot():GetAbilityByName('{}') assert(a) assert(a:GetSpecialValueInt('damage')=={}, 'neutral special damage') assert(a:GetSpecialValueInt('radius')=={}, 'neutral special radius') assert(a:GetAbilityDamage()=={}, 'neutral ability damage') end",
            ability.entry().def.key,
            bota_core::at_level(damage, 2),
            bota_core::at_level(radius, 2),
            bota_core::at_level(damage, 2)
        ),
    );
}

#[test]
fn estimates_include_a_neutral_stomps_authoritative_damage() {
    let ability = bota_core::NeutralAbility::WarStomp;
    let session = ability_session(ability.id(), 2);
    assert_script(
        &session,
        "function Think() local b=GetBot() local target=b:GetNearbyHeroes(1600,true,0)[1] assert(target) assert(b:GetEstimatedDamageToTarget(false,target,3,DAMAGE_TYPE_MAGICAL)>0, 'neutral spell estimate') end",
    );
}

fn existing_damage_value(ability: &bota_core::AbilityDef, level: u8) -> i32 {
    if let Some(neutral) = bota_core::NeutralAbility::of(ability.id) {
        return match *neutral.spell() {
            bota_core::Spell::Cast {
                act:
                    bota_core::Act::Stomp { damage, .. }
                    | bota_core::Act::Chain { damage, .. }
                    | bota_core::Act::Smash { damage, .. }
                    | bota_core::Act::Spikes { damage, .. },
                ..
            } => bota_core::at_level(&damage, level),
            bota_core::Spell::Cast {
                act: bota_core::Act::Bolt { hero_damage, .. },
                ..
            } => hero_damage,
            bota_core::Spell::Cast {
                act: bota_core::Act::Roar(spec),
                ..
            } => spec.damage,
            bota_core::Spell::Cast {
                act: bota_core::Act::Grab(spec),
                ..
            } => spec.damage,
            bota_core::Spell::Storm { near_dps, .. } => near_dps,
            bota_core::Spell::Strike(bota_core::Strike::Bash { damage, .. }) => {
                bota_core::at_level(&damage, level)
            }
            bota_core::Spell::Strike(bota_core::Strike::Lay { dps, .. }) => {
                bota_core::at_level(&dps, level)
            }
            bota_core::Spell::Strike(bota_core::Strike::Petrify { dps, .. }) => dps,
            _ => 0,
        };
    }
    let rank = usize::from(level - 1);
    use bota_core::rules;
    match ability.id {
        AbilityId::RAZE_NEAR | AbilityId::RAZE_MID | AbilityId::RAZE_FAR => {
            rules::RAZE_DAMAGE[rank]
        }
        AbilityId::REQUIEM => rules::REQUIEM_LINE_DAMAGE[rank],
        AbilityId::MEAT_HOOK => rules::HOOK_DAMAGE[rank],
        AbilityId::ROT => rules::ROT_DAMAGE_PER_SECOND[rank],
        AbilityId::DISMEMBER => rules::DISMEMBER_DAMAGE_PER_SECOND[rank],
        _ => 0,
    }
}

#[test]
fn every_catalogued_ability_reads_its_rank_and_shared_core_metadata() {
    for ability in bota_core::ABILITIES
        .iter()
        .filter(|ability| !ability.retired)
    {
        for level in 1..=ability.max_level.max(1) {
            let session = ability_session(ability.id, level);
            let source = format!(
                "function Think() local a=GetBot():GetAbilityByName('{}') assert(a,'catalogue handle') assert(a:GetName()=='{}') assert(a:GetLevel()=={}) assert(a:GetSpecialValueInt('AbilityManaCost')=={}) assert(a:GetSpecialValueInt('AbilityCooldown')=={}) assert(a:GetSpecialValueInt('AbilityCastRange')=={}) assert(a:GetAbilityDamage()=={},'base damage') assert(a:GetSpecialValueInt('__unknown_catalogue_field')==0) end",
                ability.key,
                ability.key,
                level,
                bota_core::ability_mana_cost(ability.id, level),
                bota_core::ability_cooldown(ability.id, level) / bota_core::rules::TICKS_PER_SECOND,
                ability.range,
                existing_damage_value(ability, level)
            );
            assert_script(&session, &source);
        }
    }
}

#[test]
fn every_damaging_neutral_cast_uses_its_typed_core_values_in_lua() {
    for entry in bota_core::NEUTRAL_ABILITIES {
        if !matches!(entry.spell, bota_core::Spell::Cast { .. }) {
            continue;
        }
        let level = entry.def.max_level.max(1);
        if existing_damage_value(&entry.def, level) == 0 {
            continue;
        }
        let session = ability_session(entry.def.id, level);
        assert_script(
            &session,
            "function Think() local b=GetBot() local t=b:GetNearbyHeroes(1600,true,0)[1] assert(t) assert(b:GetEstimatedDamageToTarget(false,t,10,DAMAGE_TYPE_ALL)>0,'catalogue cast contributes damage') end",
        );
    }
}

fn item_session(id: bota_proto::ItemId, level: u8) -> ClientSession {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let buy = sessions[0]
        .issue(bota_client_core::IssuedOrder::hero(
            bota_proto::Order::Buy {
                item: bota_proto::ItemId::HEALING_SALVE,
            },
        ))
        .expect("fixture purchase");
    let streams = local.step(&[&[buy], &[]]).expect("step");
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
    let mut session = sessions.remove(0);
    let own = session.own_hero().expect("hero").id;
    let mut view = session.current().expect("view").clone();
    view.tick += 1;
    let unit = view
        .units
        .iter_mut()
        .find(|unit| unit.id == own)
        .expect("actor");
    let item = unit.items[0].as_mut().expect("fixture item");
    item.id = id;
    item.level = level;
    let tick = view.tick;
    session
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("item view");
    session
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    session
}

#[test]
fn item_special_values_follow_the_received_rank_instead_of_the_first_rank() {
    let item = bota_core::item_by_key("item_roshans_banner").expect("ranked catalogue item");
    let session = item_session(item, 3);
    let expected = bota_core::item_def(item)
        .expect("item")
        .value("radius")
        .expect("radius")[2]
        .0
        / 1000;
    assert_script(
        &session,
        &format!(
            "function Think() local item=GetBot():GetItemInSlot(0) assert(item) assert(item:GetLevel()==3) assert(item:GetSpecialValueInt('radius')=={expected},'ranked item value') end"
        ),
    );
}

#[test]
fn every_item_numeric_field_is_read_from_the_catalogue_at_its_received_rank() {
    for item in bota_core::ITEMS
        .iter()
        .filter(|item| !item.retired && !item.values.is_empty())
    {
        let levels = item
            .values
            .iter()
            .map(|(_, values)| values.len())
            .max()
            .expect("fields")
            .max(1);
        for level in 1..=levels {
            let mut source =
                String::from("function Think() local item=GetBot():GetItemInSlot(0) assert(item) ");
            for (key, values) in item.values {
                let value = values
                    .get(level - 1)
                    .or_else(|| values.last())
                    .expect("nonempty catalogue field")
                    .0
                    / 1000;
                source.push_str(&format!(
                    "assert(item:GetSpecialValueInt('{key}')=={value},'{key}') "
                ));
            }
            source.push_str("end");
            assert_script(
                &item_session(item.id, u8::try_from(level).expect("bounded item ranks")),
                &source,
            );
        }
    }
}

#[test]
fn utility_abilities_expose_core_effect_values_separately_from_channel_time() {
    let seconds = |ticks: u32| ticks / bota_core::rules::TICKS_PER_SECOND;
    let cases = [
        (
            AbilityId::BURST,
            "duration",
            seconds(bota_core::rules::COURIER_BURST_TICKS) as i32,
        ),
        (
            AbilityId::BURST,
            "bonus_movement_speed",
            bota_core::rules::COURIER_BURST_PCT,
        ),
        (
            AbilityId::SHIELD,
            "duration",
            seconds(bota_core::rules::COURIER_SHIELD_TICKS) as i32,
        ),
        (
            AbilityId::CAPTURE,
            "base_capture_time",
            seconds(bota_core::CAPTURE_TICKS) as i32,
        ),
        (
            AbilityId::LAMP_USE,
            "active_duration",
            seconds(bota_core::LAMP_LIT_TICKS) as i32,
        ),
        (
            AbilityId::LAMP_USE,
            "inactive_duration",
            seconds(bota_core::LAMP_OFF_TICKS) as i32,
        ),
    ];
    for (id, key, expected) in cases {
        let definition = bota_core::ability_def(id).expect("utility ability");
        assert_script(
            &ability_session(id, 1),
            &format!(
                "function Think() local a=GetBot():GetAbilityByName('{}') assert(a) assert(a:GetSpecialValueInt('{key}')=={expected},'{key}') assert(a:GetSpecialValueInt('AbilityChannelTime')=={}) end",
                definition.key,
                seconds(definition.duration)
            ),
        );
    }
}

fn changed_view(
    mut session: ClientSession,
    change: impl FnOnce(&mut bota_proto::WorldView, EntityId),
) -> ClientSession {
    let own = session.own_hero().expect("actor").id;
    let mut view = session.current().expect("view").clone();
    view.tick += 1;
    change(&mut view, own);
    let tick = view.tick;
    session
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("changed observed view");
    session
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    session
}

#[test]
fn petrify_projects_physical_damage_without_a_magical_contribution() {
    let session = changed_view(
        ability_session(bota_core::NeutralAbility::Petrify.id(), 2),
        |view, own| {
            let actor = view
                .units
                .iter_mut()
                .find(|unit| unit.id == own)
                .expect("actor");
            actor.hp = 1;
            actor.attack_damage = 0;
        },
    );
    assert_script(
        &session,
        "function Think() local b=GetBot() local t=b:GetNearbyHeroes(1600,true,0)[1] assert(t) assert(b:GetEstimatedDamageToTarget(false,t,2,DAMAGE_TYPE_PHYSICAL)>0,'physical petrify burn') assert(b:GetEstimatedDamageToTarget(false,t,2,DAMAGE_TYPE_MAGICAL)==0,'no magical petrify burn') end",
    );
}

#[test]
fn a_grab_estimate_waits_for_its_channel_and_landing() {
    let ability = bota_core::NeutralAbility::GrabAndThrow;
    let bota_core::Spell::Cast {
        act: bota_core::Act::Grab(spec),
        ..
    } = ability.spell()
    else {
        panic!("grab data")
    };
    let before_landing_ms =
        ability.entry().def.point + spec.duration * 1000 / bota_core::rules::TICKS_PER_SECOND / 2;
    assert_script(
        &ability_session(ability.id(), 1),
        &format!(
            "function Think() local b=GetBot() local t=b:GetNearbyHeroes(1600,true,0)[1] assert(t) assert(b:GetEstimatedDamageToTarget(false,t,{before_landing_ms}/1000,DAMAGE_TYPE_ALL)==0,'no damage before the throw lands') end"
        ),
    );
}

#[test]
fn a_grab_estimate_rejects_creep_targets() {
    let session = changed_view(
        ability_session(bota_core::NeutralAbility::GrabAndThrow.id(), 1),
        |view, _| {
            let target = view
                .units
                .iter_mut()
                .find(|unit| unit.id.idx == 100_000)
                .expect("target");
            target.kind = bota_proto::UnitKind::CreepMelee;
            target.hero = None;
        },
    );
    assert_script(
        &session,
        "function Think() local b=GetBot() local t=b:GetNearbyCreeps(1600,true)[1] assert(t) assert(b:GetEstimatedDamageToTarget(false,t,10,DAMAGE_TYPE_ALL)==0,'creeps cannot be grabbed') end",
    );
}

#[test]
fn growing_spells_use_the_received_game_clock_instead_of_the_transport_tick() {
    let ability = bota_core::NeutralAbility::RoshanSlam;
    let session = changed_view(ability_session(ability.id(), 1), |view, _| {
        view.clock = Some(bota_proto::ClockShift {
            game: 10 * 60 * bota_core::rules::TICKS_PER_SECOND,
            day: 0,
        });
    });
    let view = session.current().expect("view");
    let target = view
        .units
        .iter()
        .find(|unit| unit.id.idx == 100_000)
        .expect("target");
    let bota_core::Spell::Cast {
        act:
            bota_core::Act::Stomp {
                damage,
                growth: Some(growth),
                kind,
                ..
            },
        ..
    } = ability.spell()
    else {
        panic!("growing stomp")
    };
    let amount = growth.amount(
        bota_core::at_level(damage, 1),
        1,
        view.game_tick(),
        bota_core::rules::Schedule::of(session.map().pregame_ticks),
    );
    let ratio = bota_core::mitigate(
        1_000_000,
        *kind,
        target.armor,
        bota_core::magic_resist_bp(target.magic_resist),
    );
    let expected = i64::from(amount) * i64::from(ratio) / 1_000_000;
    assert_script(
        &session,
        &format!(
            "function Think() local b=GetBot() local t=b:GetNearbyHeroes(1600,true,0)[1] assert(t) assert(b:GetEstimatedDamageToTarget(false,t,1,DAMAGE_TYPE_MAGICAL)=={expected},'received shifted game clock') end"
        ),
    );
}

fn buff_field(stat: bota_core::Stat) -> (&'static str, i32) {
    use bota_core::Stat;
    match stat {
        Stat::MovePct => ("movement_speed_pct", 1),
        Stat::AttackSpeed => ("bonus_attack_speed", 1),
        Stat::Armor => ("bonus_armor", 1),
        Stat::BaseArmorPct => ("base_armor_pct", 1),
        Stat::MagicResistPct => ("magic_resistance", 1),
        Stat::MagicResistCreepPct => ("magic_resistance_creeps", 1),
        Stat::DamagePct => ("damage_pct", 1),
        Stat::Damage => ("bonus_damage", 1),
        Stat::OutgoingPct => ("outgoing_damage_pct", 1),
        Stat::HpRegen => ("health_regen", 10),
        Stat::ManaRegen => ("mana_regen", 100),
        Stat::RegenCutPct => ("regen_reduction_pct", 1),
        Stat::LifestealPct => ("lifesteal_pct", 1),
        Stat::MaxHpPct => ("bonus_health_pct", 1),
        Stat::MagicTakenPct => ("magic_damage_taken_pct", 1),
        Stat::CooldownPct => ("cooldown_reduction_pct", 1),
        Stat::AccuracyPct => ("accuracy_pct", 1),
        Stat::SlowResistPct => ("slow_resistance", 1),
        Stat::StatusResistPct => ("status_resistance", 1),
        Stat::IllusionAttackReductionPct => ("illusion_attack_reduction_pct", 1),
        Stat::IncomingDamagePct => ("incoming_damage_pct", 1),
    }
}

#[test]
fn every_catalogued_neutral_modifier_exposes_its_existing_numeric_stats() {
    use bota_core::{Act, Spell, Strike, Then, Throe};
    for entry in bota_core::NEUTRAL_ABILITIES {
        let buff = match entry.spell {
            Spell::Aura(aura) => Some(aura.buff),
            Spell::Storm { buff, .. }
            | Spell::Intrinsic { buff, .. }
            | Spell::Block { buff, .. } => Some(buff),
            Spell::Strike(Strike::Lay { buff, .. })
            | Spell::Throe(Throe::Rouse { buff, .. })
            | Spell::Cast {
                act: Act::Enrage { buff, .. },
                ..
            }
            | Spell::Cast {
                act:
                    Act::Stomp {
                        then: Then::Wear { buff, .. },
                        ..
                    },
                ..
            } => Some(buff),
            _ => None,
        };
        let Some(buff) = buff else { continue };
        let level = entry.def.max_level.max(1);
        let mut source = format!(
            "function Think() local a=GetBot():GetAbilityByName('{}') assert(a) ",
            entry.def.key
        );
        for (stat, _) in buff.stats {
            let (key, scale) = buff_field(stat);
            let expected = buff.get(stat, level) / scale;
            source.push_str(&format!(
                "assert(a:GetSpecialValueInt('{key}')=={expected},'{key}') "
            ));
        }
        source.push_str("end");
        assert_script(&ability_session(entry.def.id, level), &source);
    }
}
