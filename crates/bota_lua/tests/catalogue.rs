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
    let actor = view.units.iter_mut().find(|unit| unit.id == own).expect("actor");
    let mut ability = actor.abilities[0];
    ability.id = id;
    ability.level = level;
    ability.cooldown_left = 0;
    ability.mana_cost = bota_core::ability_mana_cost(id, level);
    actor.abilities = vec![ability];
    actor.mana = actor.max_mana;
    let mut target = actor.clone();
    target.id = EntityId { idx: 100_000, generation: 0 };
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
    session.ingest(ServerMsg::Snapshot { view }, &mut ()).expect("catalogue ability view");
    session.ingest(ServerMsg::Events { tick, events: Vec::new() }, &mut ()).expect("events");
    session
}

fn assert_script(session: &ClientSession, source: &str) {
    let mut lua = LuaTeam::new(Team::Radiant, &[BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Limits::DEFAULT, Keys { hash: [17,19], random: 23 }).expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "catalogue", source.as_bytes()).expect("source");
    let turn = lua.think(session).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}

#[test]
fn a_neutral_stomp_exposes_its_core_damage_and_special_values() {
    let ability = bota_core::NeutralAbility::WarStomp;
    let bota_core::Spell::Cast { act: bota_core::Act::Stomp { damage, radius, .. }, .. } = ability.spell() else { panic!("stomp data") };
    let session = ability_session(ability.id(), 2);
    assert_script(&session, &format!("function Think() local a=GetBot():GetAbilityByName('{}') assert(a) assert(a:GetSpecialValueInt('damage')=={}, 'neutral special damage') assert(a:GetSpecialValueInt('radius')=={}, 'neutral special radius') assert(a:GetAbilityDamage()=={}, 'neutral ability damage') end", ability.entry().def.key, bota_core::at_level(damage,2), bota_core::at_level(radius,2), bota_core::at_level(damage,2)));
}

#[test]
fn estimates_include_a_neutral_stomps_authoritative_damage() {
    let ability = bota_core::NeutralAbility::WarStomp;
    let session = ability_session(ability.id(), 2);
    assert_script(&session, "function Think() local b=GetBot() local target=b:GetNearbyHeroes(1600,true,0)[1] assert(target) assert(b:GetEstimatedDamageToTarget(false,target,3,DAMAGE_TYPE_MAGICAL)>0, 'neutral spell estimate') end");
}

fn existing_damage_value(ability: &bota_core::AbilityDef, level: u8) -> i32 {
    if let Some(neutral) = bota_core::NeutralAbility::of(ability.id) {
        return match *neutral.spell() {
            bota_core::Spell::Cast { act: bota_core::Act::Stomp { damage, .. } | bota_core::Act::Chain { damage, .. } | bota_core::Act::Smash { damage, .. } | bota_core::Act::Spikes { damage, .. }, .. } => bota_core::at_level(&damage, level),
            bota_core::Spell::Cast { act: bota_core::Act::Bolt { hero_damage, .. }, .. } => hero_damage,
            bota_core::Spell::Cast { act: bota_core::Act::Roar(spec), .. } => spec.damage,
            bota_core::Spell::Cast { act: bota_core::Act::Grab(spec), .. } => spec.damage,
            bota_core::Spell::Storm { near_dps, .. } => near_dps,
            bota_core::Spell::Strike(bota_core::Strike::Bash { damage, .. }) => bota_core::at_level(&damage, level),
            bota_core::Spell::Strike(bota_core::Strike::Lay { dps, .. }) => bota_core::at_level(&dps, level),
            bota_core::Spell::Strike(bota_core::Strike::Petrify { dps, .. }) => dps,
            _ => 0,
        };
    }
    let rank = usize::from(level - 1);
    use bota_core::rules;
    match ability.id {
        AbilityId::RAZE_NEAR | AbilityId::RAZE_MID | AbilityId::RAZE_FAR => rules::RAZE_DAMAGE[rank],
        AbilityId::REQUIEM => rules::REQUIEM_LINE_DAMAGE[rank],
        AbilityId::MEAT_HOOK => rules::HOOK_DAMAGE[rank],
        AbilityId::ROT => rules::ROT_DAMAGE_PER_SECOND[rank],
        AbilityId::DISMEMBER => rules::DISMEMBER_DAMAGE_PER_SECOND[rank],
        _ => 0,
    }
}

#[test]
fn every_catalogued_ability_reads_its_rank_and_shared_core_metadata() {
    for ability in bota_core::ABILITIES.iter().filter(|ability| !ability.retired) {
        for level in 1..=ability.max_level.max(1) {
            let session = ability_session(ability.id, level);
            let source = format!("function Think() local a=GetBot():GetAbilityByName('{}') assert(a,'catalogue handle') assert(a:GetName()=='{}') assert(a:GetLevel()=={}) assert(a:GetSpecialValueInt('AbilityManaCost')=={}) assert(a:GetSpecialValueInt('AbilityCooldown')=={}) assert(a:GetSpecialValueInt('AbilityCastRange')=={}) assert(a:GetAbilityDamage()=={},'base damage') assert(a:GetSpecialValueInt('__unknown_catalogue_field')==0) end",
                ability.key, ability.key, level, bota_core::ability_mana_cost(ability.id,level), bota_core::ability_cooldown(ability.id,level)/bota_core::rules::TICKS_PER_SECOND, ability.range, existing_damage_value(ability, level));
            assert_script(&session, &source);
        }
    }
}

#[test]
fn every_damaging_neutral_cast_uses_its_typed_core_values_in_lua() {
    for entry in bota_core::NEUTRAL_ABILITIES {
        if !matches!(entry.spell, bota_core::Spell::Cast { .. }) { continue; }
        let level = entry.def.max_level.max(1);
        if existing_damage_value(&entry.def, level) == 0 { continue; }
        let session = ability_session(entry.def.id, level);
        assert_script(&session, "function Think() local b=GetBot() local t=b:GetNearbyHeroes(1600,true,0)[1] assert(t) assert(b:GetEstimatedDamageToTarget(false,t,10,DAMAGE_TYPE_ALL)>0,'catalogue cast contributes damage') end");
    }
}

fn item_session(id: bota_proto::ItemId, level: u8) -> ClientSession {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let buy = sessions[0].issue(bota_client_core::IssuedOrder::hero(bota_proto::Order::Buy { item: bota_proto::ItemId::HEALING_SALVE })).expect("fixture purchase");
    let streams = local.step(&[&[buy], &[]]).expect("step");
    for (session, stream) in sessions.iter_mut().zip(streams) { support::fold(session, stream); }
    let mut session = sessions.remove(0);
    let own = session.own_hero().expect("hero").id;
    let mut view = session.current().expect("view").clone();
    view.tick += 1;
    let unit = view.units.iter_mut().find(|unit| unit.id == own).expect("actor");
    let item = unit.items[0].as_mut().expect("fixture item");
    item.id = id; item.level = level;
    let tick=view.tick;
    session.ingest(ServerMsg::Snapshot { view }, &mut ()).expect("item view");
    session.ingest(ServerMsg::Events { tick, events: Vec::new() }, &mut ()).expect("events");
    session
}

#[test]
fn item_special_values_follow_the_received_rank_instead_of_the_first_rank() {
    let item = bota_core::item_by_key("item_roshans_banner").expect("ranked catalogue item");
    let session = item_session(item, 3);
    let expected = bota_core::item_def(item).expect("item").value("radius").expect("radius")[2].0 / 1000;
    assert_script(&session, &format!("function Think() local item=GetBot():GetItemInSlot(0) assert(item) assert(item:GetLevel()==3) assert(item:GetSpecialValueInt('radius')=={expected},'ranked item value') end"));
}

#[test]
fn every_item_numeric_field_is_read_from_the_catalogue_at_its_received_rank() {
    for item in bota_core::ITEMS.iter().filter(|item| !item.retired && !item.values.is_empty()) {
        let levels = item.values.iter().map(|(_,values)| values.len()).max().expect("fields").max(1);
        for level in 1..=levels {
            let mut source = String::from("function Think() local item=GetBot():GetItemInSlot(0) assert(item) ");
            for (key, values) in item.values {
                let value = values.get(level-1).or_else(|| values.last()).expect("nonempty catalogue field").0 / 1000;
                source.push_str(&format!("assert(item:GetSpecialValueInt('{key}')=={value},'{key}') "));
            }
            source.push_str("end");
            assert_script(&item_session(item.id, u8::try_from(level).expect("bounded item ranks")), &source);
        }
    }
}
