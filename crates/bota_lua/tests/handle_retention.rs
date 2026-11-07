mod support;

use bota_client_core::{ClientSession, DEFAULT_FORGET_AFTER_TICKS};
use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{EntityId, HeroId, ServerMsg, SlotId, Team};

fn team(source: &[u8], stress: GcStress) -> LuaTeam {
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
    lua.add_script(SlotId(0), ScriptKind::Takeover, "lifetimes", source)
        .expect("script");
    lua.set_gc_stress(stress);
    lua
}

fn wave(session: &mut ClientSession, first: u32, generation: u32, count: u32) {
    let mut view = session.current().expect("view").clone();
    view.tick += DEFAULT_FORGET_AFTER_TICKS + 1;
    let template = session.own_hero().expect("hero").clone();
    view.units.retain(|unit| unit.id.idx < 100_000);
    for index in 0..count {
        let mut unit = template.clone();
        unit.id = EntityId {
            idx: first + index,
            generation,
        };
        unit.team = Team::Dire;
        unit.owner = None;
        for ability in &mut unit.abilities {
            ability.learnable = None;
        }
        unit.items.clear();
        unit.town_portal = None;
        unit.effects.clear();
        unit.queue.clear();
        view.units.push(unit);
    }
    view.units.sort_by_key(|unit| unit.id);
    let tick = view.tick;
    session
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("wave");
    session
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    assert_eq!(
        session
            .entities()
            .iter()
            .filter(|track| track.id.idx >= 100_000)
            .count(),
        count as usize,
        "old waves really leave tracking"
    );
}

#[test]
fn forgotten_waves_do_not_exhaust_unit_or_ability_handles() {
    for stress in [GcStress::Off, GcStress::Full, GcStress::Incremental] {
        let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let mut lua = team(
            br#"
            function Think()
                local current=GetBot():GetNearbyHeroes(1600,true,BOT_MODE_NONE)
                assert(#current>0)
                for _,unit in ipairs(current) do
                    local ability=unit:GetAbilityByName('nevermore_shadowraze1')
                    assert(ability:GetName()=='nevermore_shadowraze1')
                end
            end
        "#,
            stress,
        );
        // Stress collections pay real fuel; keep each call small while still observing over 8192 identities.
        let count = match stress {
            GcStress::Off => 64,
            _ => 8,
        };
        for round in 0..8448 / count {
            wave(&mut sessions[0], 100_000 + round * count, 0, count);
            let turn = lua.think(&sessions[0]).expect("think");
            assert!(
                turn.failures.is_empty(),
                "wave={round}, stress={stress:?}: {:?}",
                turn.failures
            );
        }
    }
}

#[test]
fn script_retained_handles_stay_authenticated_and_reuse_identity_after_reobservation() {
    for stress in [GcStress::Full, GcStress::Incremental] {
        let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let mut lua = team(
            br#"
            local old,ability,turn
            turn=0
            function Think()
                turn=turn+1
                collectgarbage()
                local bot=GetBot()
                local current=bot:GetNearbyHeroes(1600,true,BOT_MODE_NONE)[1]
                local current_ability=current:GetAbilityByName('nevermore_shadowraze1')
                if turn==1 then
                    old=current ability=current_ability
                    old.saved=17 ability.saved=23
                elseif turn==3 then
                    assert(current==old and current_ability==ability)
                    assert(not old:IsNull() and not ability:IsNull())
                    assert(old.saved==17 and ability.saved==23)
                else
                    assert(old:IsNull() and ability:IsNull())
                    assert(old:GetHealth()==0 and ability:GetName()=='')
                    assert(current~=old and current_ability~=ability)
                    assert(getmetatable(old)==false and getmetatable(ability)==false)
                    assert(not pcall(setmetatable,old,{}))
                    local forged={}
                    for key,value in pairs(old) do forged[key]=value end
                    assert(not pcall(old.IsAlive,forged))
                    assert(not pcall(ability.GetName,forged))
                    assert(not pcall(bot.Action_UseAbility,bot,ability))
                end
            end
        "#,
            stress,
        );
        for (first, generation) in [(100_000, 0), (200_000, 0), (100_000, 0), (100_000, 1)] {
            wave(&mut sessions[0], first, generation, 64);
            let turn = lua.think(&sessions[0]).expect("think");
            assert!(
                turn.failures.is_empty(),
                "stress={stress:?}: {:?}",
                turn.failures
            );
        }
    }
}

#[test]
fn collected_forgotten_handles_release_their_host_roots() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(
        br#"
        local initial
        function Think()
            collectgarbage()
            local bytes=collectgarbage('count')
            if initial==nil then initial=bytes end
            assert(bytes<initial+1024, 'forgotten handle roots keep growing')
            for _,unit in ipairs(GetBot():GetNearbyHeroes(1600,true,BOT_MODE_NONE)) do
                assert(unit:GetAbilityByName('nevermore_shadowraze1')~=nil)
            end
        end
    "#,
        GcStress::Off,
    );
    for round in 0..48 {
        wave(&mut sessions[0], 100_000 + round * 64, 0, 64);
        let turn = lua.think(&sessions[0]).expect("think");
        assert!(
            turn.failures.is_empty(),
            "wave={round}: {:?}",
            turn.failures
        );
    }
}
