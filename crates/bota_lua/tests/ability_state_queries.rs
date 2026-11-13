mod support;

use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Cheat, HeroId, ItemId, Order, ServerMsg, SlotId, Target, Team, Vec2};

#[test]
fn ability_and_item_reads_follow_received_mana_cooldowns_charges_and_item_removal() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for slot in [SlotId(0), SlotId(1)] {
        let world = local.world_mut();
        world.cheats = true;
        world.cheat(
            slot,
            None,
            Cheat::Item {
                item: ItemId::TANGO,
            },
            &mut Vec::new(),
        );
    }
    let streams = local.step(&[&[], &[]]).expect("received items");
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let slot = SlotId(index as u8);
        let mut view = sessions[index].current().expect("view").clone();
        let own = sessions[index].own_hero().expect("hero").id;
        let key = bota_core::ability_def(sessions[index].own_hero().expect("hero").abilities[0].id)
            .expect("definition")
            .key;
        let mut lua = LuaTeam::new(
            side,
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
        .expect("team");
        lua.set_gc_stress(GcStress::Full);
        let source = r#"
            local phase, ability, item = 0, nil, nil
            function Think()
                phase = phase + 1
                if phase == 1 then
                    ability = GetBot():GetAbilityByName('__ABILITY__')
                    item = GetBot():GetItemInSlot(0)
                end
                assert(ability and item)
                for _, name in ipairs({'GetManaCost','GetCooldownTimeRemaining','IsCooldownReady','GetCurrentCharges'}) do
                    assert(type(ability[name]) == 'function')
                    assert(not pcall(ability[name],{}))
                    assert(not pcall(ability[name],setmetatable({}, {__index=ability})))
                end
                assert(ability:GetManaCost() == 37 and ability:GetCurrentCharges() == 0)
                assert(ability:GetCooldownTimeRemaining() == (phase == 1 and 2.5 or 0))
                assert(ability:IsCooldownReady() == (phase ~= 1))
                if phase <= 2 then
                    assert(item:GetManaCost() == 19)
                    assert(item:GetCurrentCharges() == (phase == 1 and 7 or 2))
                    assert(item:GetCooldownTimeRemaining() == (phase == 1 and 1.5 or 0))
                    assert(item:IsCooldownReady() == (phase == 2))
                else
                    assert(GetBot():GetItemInSlot(0) == nil and item:IsNull())
                    for _, name in ipairs({'GetManaCost','GetCooldownTimeRemaining','IsCooldownReady','GetCurrentCharges'}) do
                        local ok, reason = pcall(item[name],item)
                        assert(not ok and string.find(tostring(reason),'ability is no longer known',1,true))
                    end
                end
                GetBot():Action_MoveToLocation(Vector(100+phase,200,0))
            end
        "#.replace("__ABILITY__",key);
        lua.add_script(
            slot,
            ScriptKind::Takeover,
            "ability_state_queries",
            source.as_bytes(),
        )
        .expect("script");
        for phase in 1..=4 {
            view.tick += if phase == 4 { 481 } else { 1 };
            let unit = view
                .units
                .iter_mut()
                .find(|unit| unit.id == own)
                .expect("hero");
            unit.abilities[0].level = 1;
            unit.abilities[0].mana_cost = 37;
            unit.abilities[0].cooldown_left = if phase == 1 { 75 } else { 0 };
            if phase < 3 {
                let item = unit.items[0].as_mut().expect("tango");
                item.mana_cost = 19;
                item.cooldown_left = if phase == 1 { 45 } else { 0 };
                item.charges = Some(if phase == 1 { 7 } else { 2 });
            } else {
                unit.items[0] = None;
            }
            if phase == 4 {
                let kit = bota_proto::Kit {
                    abilities: unit.abilities.clone(),
                    items: unit.items.clone(),
                    town_portal: unit.town_portal,
                    sellable_slots: 0,
                    reincarnating: false,
                };
                let player = view
                    .players
                    .iter_mut()
                    .find(|player| player.slot == slot)
                    .expect("player");
                player.kit = Some(kit);
                player.respawn_left = 600;
                player.deaths = 1;
                view.units.retain(|unit| unit.id != own);
                view.aggro.clear();
            }
            let tick = view.tick;
            sessions[index]
                .ingest(ServerMsg::Snapshot { view: view.clone() }, &mut ())
                .expect("received ability state");
            sessions[index]
                .ingest(
                    ServerMsg::Events {
                        tick,
                        events: Vec::new(),
                    },
                    &mut (),
                )
                .expect("events");
            if phase == 4 {
                assert!(sessions[index].entity(own).is_none());
            }
            let turn = lua.think(&sessions[index]).expect("think");
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
            assert_eq!(turn.bots[0].orders.len(), 1);
            assert_eq!(
                turn.bots[0].orders[0].order,
                Order::Move {
                    target: Target::Pos(Vec2::from_ints(9316 + phase, 9416))
                }
            );
        }
    }
}
