mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Cheat, HeroId, ItemId, SlotId, Team};
fn run(source: &[u8]) {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    for index in 0..2 {
        for item in [ItemId::POWER_TREADS, ItemId::TANGO] {
            local.world_mut().cheat(
                SlotId(index as u8),
                None,
                Cheat::Item { item },
                &mut Vec::new(),
            );
        }
    }
    for (session, stream) in sessions.iter_mut().zip(local.step(&[&[], &[]]).unwrap()) {
        support::fold(session, stream);
    }
    for (index, session) in sessions.iter().enumerate() {
        let slot = SlotId(index as u8);
        let mut lua = LuaTeam::new(
            session.team(),
            &[BotSeat {
                slot,
                hero: HeroId::SHADOW_FIEND,
            }],
            Limits::DEFAULT,
            Keys {
                hash: [17, 19],
                random: 23,
            },
        )
        .unwrap();
        lua.add_script(slot, ScriptKind::Takeover, "ability_extra", source)
            .unwrap();
        let turn = lua.think(session).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    }
}
#[test]
fn channel_time_is_the_catalogue_channel_and_not_a_buff_duration() {
    run(b"function Think() local bot=GetBot(); assert(bot:GetAbilityByName('nevermore_shadowraze1'):GetChannelTime()==0); assert(bot:GetItemInSlot(1):GetChannelTime()==0) end");
}
#[test]
fn ability_phase_is_false_outside_a_visible_cast_point() {
    run(b"function Think() local bot=GetBot(); assert(not bot:GetAbilityByName('nevermore_shadowraze1'):IsInAbilityPhase()); assert(not bot:GetItemInSlot(0):IsInAbilityPhase()) end");
}
#[test]
fn power_treads_reports_the_received_selected_attribute_and_non_treads_sentinel() {
    run(b"function Think() local bot=GetBot(); assert(bot:GetItemInSlot(0):GetPowerTreadsStat()==ATTRIBUTE_STRENGTH); assert(bot:GetItemInSlot(1):GetPowerTreadsStat()==-1) end");
}
#[test]
fn power_treads_attribute_changes_after_real_item_uses() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{ItemSlot, Order, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    local.world_mut().cheat(
        SlotId(0),
        None,
        Cheat::Item {
            item: ItemId::POWER_TREADS,
        },
        &mut Vec::new(),
    );
    for (session, stream) in sessions.iter_mut().zip(local.step(&[&[], &[]]).unwrap()) {
        support::fold(session, stream);
    }
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
    .unwrap();
    lua.add_script(SlotId(0),ScriptKind::Takeover,"treads_modes",b"local n=0 function Think() local expected={ATTRIBUTE_STRENGTH,ATTRIBUTE_INTELLECT,ATTRIBUTE_AGILITY}; n=n+1; assert(GetBot():GetItemInSlot(0):GetPowerTreadsStat()==expected[n]) end").unwrap();
    for phase in 0..3 {
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(
            turn.failures.is_empty(),
            "phase={phase}: {:?}",
            turn.failures
        );
        if phase < 2 {
            let order = sessions[0]
                .issue(IssuedOrder::hero(Order::Use {
                    slot: ItemSlot(0),
                    target: Target::None,
                }))
                .unwrap();
            for (session, stream) in sessions
                .iter_mut()
                .zip(local.step(&[&[order], &[]]).unwrap())
            {
                support::fold(session, stream);
            }
        }
    }
}
#[test]
fn the_specific_spell_in_a_received_cast_point_is_in_ability_phase() {
    use bota_proto::{AbilityId, CastView, ServerMsg};
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut view = sessions[0].current().unwrap().clone();
    view.tick += 1;
    let own = sessions[0].own_hero().unwrap().id;
    let unit = view.units.iter_mut().find(|unit| unit.id == own).unwrap();
    unit.cast = Some(CastView {
        ability: Some(AbilityId::RAZE_NEAR),
        goes_off_in: 2,
    });
    unit.statuses.bits |= bota_proto::StatusFlags::CASTING;
    let tick = view.tick;
    sessions[0]
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .unwrap();
    sessions[0]
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .unwrap();
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
    .unwrap();
    lua.add_script(SlotId(0),ScriptKind::Takeover,"cast_phase",b"function Think() local bot=GetBot(); assert(bot:GetAbilityByName('nevermore_shadowraze1'):IsInAbilityPhase()); assert(not bot:GetAbilityByName('nevermore_shadowraze2'):IsInAbilityPhase()) end").unwrap();
    let turn = lua.think(&sessions[0]).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}

#[test]
fn a_channelled_spell_reports_its_nonzero_base_channel() {
    let (_, sessions) = support::local_with_modifiers(
        &[
            (Team::Radiant, HeroId::PUDGE),
            (Team::Dire, HeroId::SHADOW_FIEND),
        ],
        Vec::new(),
    );
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::PUDGE,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .unwrap();
    lua.add_script(SlotId(0),ScriptKind::Takeover,"channel_base",b"function Think() assert(GetBot():GetAbilityByName('pudge_dismember'):GetChannelTime()==2.75) end").unwrap();
    let turn = lua.think(&sessions[0]).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}
