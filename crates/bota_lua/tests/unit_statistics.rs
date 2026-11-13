mod support;

use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Attribute, Fixed, HeroId, Order, ServerMsg, SlotId, Target, Team, Vec2};

fn assert_script_ran(turn: &bota_lua::TeamTurn) {
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
    assert_eq!(
        turn.bots[0].orders[0].order,
        Order::Move {
            target: Target::Pos(Vec2::from_ints(9339, 9672))
        }
    );
}

#[test]
fn unit_statistics_preserve_received_whole_points_fractions_and_time_units() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let slot = SlotId(index as u8);
        let mut view = sessions[index].current().expect("view").clone();
        view.tick += 1;
        let unit = view
            .units
            .iter_mut()
            .find(|unit| unit.owner == Some(slot) && unit.hero.is_some())
            .expect("hero");
        unit.mana = 2_000_000;
        unit.max_mana = i32::MAX;
        unit.hp_regen = Fixed::from_ratio(5, 4);
        unit.mana_regen = Fixed::from_ratio(9, 4);
        unit.armor = Fixed::from_ratio(-5, 2);
        unit.attack_damage = i32::MAX;
        unit.attack_range = Fixed::from_ratio(1203, 2);
        unit.attack_time = 1250;
        unit.move_speed = Fixed::from_ratio(701, 2);
        unit.bound = Fixed::from_ratio(145, 4);
        unit.evasion = Fixed::from_ratio(1, 4);
        unit.facing = bota_proto::Angle { brads: 16584 };
        unit.attributes.strength = Fixed::from_ratio(39, 2);
        unit.attributes.agility = Fixed::from_ratio(93, 4);
        unit.attributes.intelligence = Fixed::from_ratio(75, 4);
        unit.primary = Some(Attribute::Agility);
        let tick = view.tick;
        sessions[index]
            .ingest(ServerMsg::Snapshot { view }, &mut ())
            .expect("received statistics");
        sessions[index]
            .ingest(
                ServerMsg::Events {
                    tick,
                    events: Vec::new(),
                },
                &mut (),
            )
            .expect("events");
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
        lua.add_script(slot, ScriptKind::Takeover, "received_statistics", br#"
            function Think()
                local unit = GetBot()
                assert(unit:GetMana() == 2000000 and unit:GetMaxMana() == 2147483647)
                assert(unit:GetHealthRegen() == 1.25 and unit:GetManaRegen() == 2.25)
                assert(unit:GetArmor() == -2.5 and unit:GetAttackDamage() == 2147483647)
                assert(unit:GetAttackRange() == 601)
                assert(unit:GetSecondsPerAttack() == 1.25)
                assert(unit:GetCurrentMovementSpeed() == 350 and unit:GetBoundingRadius() == 36.25)
                assert(unit:GetEvasion() == 0.25 and unit:GetFacing() == 91)
                assert(unit:GetPrimaryAttribute() == ATTRIBUTE_AGILITY)
                assert(ATTRIBUTE_STRENGTH == 0 and ATTRIBUTE_AGILITY == 1 and ATTRIBUTE_INTELLECT == 2)
                assert(unit:GetAttributeValue(ATTRIBUTE_STRENGTH) == 19)
                assert(unit:GetAttributeValue(ATTRIBUTE_AGILITY) == 23)
                assert(unit:GetAttributeValue(ATTRIBUTE_INTELLECT) == 18)
                unit:Action_MoveToLocation(Vector(123,456,0))
            end
        "#).expect("script");
        let turn = lua.think(&sessions[index]).expect("think");
        assert_script_ran(&turn);
    }
}

#[test]
fn unit_statistics_reject_forged_receivers_and_invalid_attribute_indices() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
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
    lua.add_script(SlotId(0), ScriptKind::Takeover, "statistic_boundaries", br#"
        function Think()
            local unit = GetBot()
            for _, name in ipairs({'GetMana','GetMaxMana','GetHealthRegen','GetManaRegen','GetArmor',
                'GetAttackDamage','GetAttackRange','GetSecondsPerAttack',
                'GetCurrentMovementSpeed','GetBoundingRadius','GetEvasion','GetFacing',
                'GetPrimaryAttribute','GetAttributeValue'}) do
                local method = unit[name]
                assert(type(method) == 'function')
                assert(not pcall(method, {}, 0))
            end
            for _, value in ipairs({-1,3,0.5,1e300,-1e300,0/0,math.huge,-math.huge}) do
                assert(not pcall(unit.GetAttributeValue, unit, value))
            end
            local ok, reason = pcall(unit.GetAttributeValue, unit, -1)
            assert(not ok and string.find(tostring(reason), 'integer within bounds expected', 1, true))
            ok, reason = pcall(unit.GetAttributeValue, unit, math.huge)
            assert(not ok and string.find(tostring(reason), 'finite number expected', 1, true))
            assert(unit:GetMana() >= 0)
            unit:Action_MoveToLocation(Vector(123,456,0))
        end
    "#).expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert_script_ran(&turn);
}
