mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Audience, Body, HeroId, SlotId, Team};
fn host(source: &str) -> LuaTeam {
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [1, 2],
            random: 3,
        },
    )
    .unwrap();
    lua.add_script(SlotId(0), ScriptKind::Takeover, "chat", source.as_bytes())
        .unwrap();
    lua
}
fn rate(name: &str) {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = host(&format!(
        "function Think() GetBot():{name}(' hello ',true); GetBot():{name}('second',false) end"
    ));
    for turn in 0..31 {
        let result = lua.think(&sessions[0]).unwrap();
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert_eq!(
            result.bots[0].chat.len(),
            usize::from(turn == 0 || turn == 30)
        );
        if let Some(line) = result.bots[0].chat.first() {
            assert_eq!(line.to, Audience::All);
            assert_eq!(line.body, Body::Text("hello".to_owned()));
        }
        for (session, stream) in sessions.iter_mut().zip(local.step(&[&[], &[]]).unwrap()) {
            support::fold(session, stream);
        }
    }
}
#[test]
fn immediate_chat_is_bounded_and_rate_limited_on_simulation_ticks() {
    rate("ActionImmediate_Chat");
}
#[test]
fn chat_alias_has_the_same_bounded_rate_contract() {
    rate("Action_Chat");
}
#[test]
fn hostile_text_is_rejected_and_a_failed_turn_does_not_consume_the_chat_slot() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = host(
        r#"local failed=false
 function Think()
  local b=GetBot()
  for _,text in ipairs({'\255','\27[31m','\nforged',string.rep('x',513)}) do assert(not pcall(b.ActionImmediate_Chat,b,text,false)) end
  b:ActionImmediate_Chat('safe',false)
  if not failed then failed=true;error('discard this turn') end
 end"#,
    );
    let result = lua.think(&sessions[0]).unwrap();
    assert_eq!(result.failures.len(), 1);
    assert!(result.bots[0].chat.is_empty());
    for (session, stream) in sessions.iter_mut().zip(local.step(&[&[], &[]]).unwrap()) {
        support::fold(session, stream);
    }
    let result = lua.think(&sessions[0]).unwrap();
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    assert_eq!(result.bots[0].chat.len(), 1);
    assert_eq!(result.bots[0].chat[0].to, Audience::Team);
}
