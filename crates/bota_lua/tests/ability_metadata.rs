mod support;

use bota_client_core::IssuedOrder;
use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Cheat, HeroId, ItemId, ItemSlot, Order, SlotId, Target, Team, Vec2};

#[test]
fn ability_metadata_identifies_talents_ultimates_and_training_after_real_learning() {
    let hero = bota_core::hero_def(HeroId::SHADOW_FIEND).expect("hero definition");
    let skill = bota_core::ability_def(hero.abilities[0])
        .expect("skill")
        .key;
    let ultimate = bota_core::ability_def(
        *hero
            .abilities
            .last()
            .expect("ultimate slot per hero catalogue contract"),
    )
    .expect("ultimate")
    .key;
    let talent = hero.talents.expect("tree").first().expect("talent").key();
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let slot = SlotId(index as u8);
        {
            let world = local.world_mut();
            world.cheats = true;
            world.cheat(slot, None, Cheat::Levels { count: 9 }, &mut Vec::new());
            world.cheat(
                slot,
                None,
                Cheat::Item {
                    item: ItemId::TANGO,
                },
                &mut Vec::new(),
            );
        }
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&[], &[]]).expect("setup"))
        {
            support::fold(session, stream);
        }
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
            local phase, skill, ultimate, talent, item = 0, nil, nil, nil, nil
            function Think()
                phase=phase+1
                local bot=GetBot()
                if phase==1 then
                    skill=bot:GetAbilityByName('__SKILL__')
                    ultimate=bot:GetAbilityByName('__ULT__')
                    talent=bot:GetAbilityByName('__TALENT__')
                    item=bot:GetItemInSlot(0)
                end
                assert(skill and ultimate and talent and item)
                assert(not skill:IsUltimate() and not skill:IsTalent())
                assert(ultimate:IsUltimate() and not ultimate:IsTalent() and not ultimate:IsTrained())
                assert(talent:IsTalent() and not talent:IsUltimate())
                assert(skill:IsTrained() == (phase~=1) and talent:IsTrained() == (phase~=1))
                for _, name in ipairs({'IsTrained','IsTalent','IsUltimate'}) do
                    assert(not pcall(skill[name],{}) and not pcall(skill[name],bot))
                    assert(not pcall(skill[name],setmetatable({}, {__index=skill})))
                end
                if phase==1 then
                    assert(item:IsTrained() and not item:IsTalent() and not item:IsUltimate())
                    bot:ActionImmediate_LevelAbility('__SKILL__')
                    bot:ActionImmediate_LevelAbility('__TALENT__')
                else
                    if phase==3 then
                        assert(item:IsNull())
                        for _, name in ipairs({'IsTrained','IsTalent','IsUltimate'}) do
                            local ok,reason=pcall(item[name],item)
                            assert(not ok and string.find(tostring(reason),'ability is no longer known',1,true))
                        end
                    end
                    bot:Action_MoveToLocation(Vector(100+phase,200,0))
                end
            end
        "#.replace("__SKILL__",skill).replace("__ULT__",ultimate).replace("__TALENT__",talent);
        lua.add_script(
            slot,
            ScriptKind::Takeover,
            "ability_metadata",
            source.as_bytes(),
        )
        .expect("script");
        for phase in 1..=3 {
            let turn = lua.think(&sessions[index]).expect("think");
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
            if phase == 1 {
                assert_eq!(
                    turn.bots[0]
                        .orders
                        .iter()
                        .map(|order| order.order)
                        .collect::<Vec<_>>(),
                    vec![
                        Order::Learn {
                            slot: bota_proto::AbilitySlot(0)
                        },
                        Order::LearnTalent {
                            talent: bota_proto::TalentSlot(0)
                        }
                    ]
                );
            } else {
                assert_eq!(
                    turn.bots[0].orders,
                    vec![IssuedOrder::hero(Order::Move {
                        target: Target::Pos(Vec2::from_ints(9316 + phase, 9416))
                    })]
                );
            }
            let mut outgoing: Vec<_> = turn.bots[0]
                .orders
                .iter()
                .map(|&order| sessions[index].issue(order).expect("ordinary order"))
                .collect();
            if phase == 2 {
                let position = sessions[index].own_hero().expect("hero").pos;
                outgoing.push(
                    sessions[index]
                        .issue(IssuedOrder::hero(Order::Put {
                            slot: ItemSlot(0),
                            target: Target::Pos(position),
                        }))
                        .expect("drop"),
                );
            }
            let mut by_seat = [Vec::new(), Vec::new()];
            by_seat[index] = outgoing;
            for (session, stream) in sessions.iter_mut().zip(
                local
                    .step(&[&by_seat[0], &by_seat[1]])
                    .expect("apply orders"),
            ) {
                assert!(
                    !stream.iter().any(|message| matches!(
                        message,
                        bota_proto::ServerMsg::OrderRejected { .. }
                    )),
                    "learning and dropping are accepted"
                );
                support::fold(session, stream);
            }
        }
    }
}
