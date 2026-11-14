#![allow(
    clippy::float_arithmetic,
    reason = "exact conversion of catalogue fixed point to the Lua coordinate convention"
)]
mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};
fn run(source: &[u8]) {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
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
        lua.add_script(slot, ScriptKind::Takeover, "catalogue_queries", source)
            .unwrap();
        let turn = lua.think(session).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    }
}
#[test]
fn item_slot_types_follow_the_installed_legacy_classes() {
    run(br#"function Think()
 local bot=GetBot()
 assert(bot:GetItemSlotType(0)==0 and bot:GetItemSlotType(5)==0)
 assert(bot:GetItemSlotType(6)==1 and bot:GetItemSlotType(8)==1)
 assert(bot:GetItemSlotType(9)==2 and bot:GetItemSlotType(14)==2)
 assert(bot:GetItemSlotType(-1)==-1 and bot:GetItemSlotType(15)==-1)
 end"#);
}
#[test]
fn the_day_fraction_matches_the_pregame_clock() {
    run(br#"function Think() assert(math.abs(GetTimeOfDay()-(0.25+DotaTime()/600))<0.000001) end"#);
}
#[test]
fn height_reads_published_ground_instead_of_vector_z() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let position = sessions[0].own_hero().unwrap().pos;
    let expected = sessions[0].statics().ground_tier(position);
    let dire = sessions[1]
        .statics()
        .ground_tier(sessions[1].own_hero().unwrap().pos);
    run(format!("function Think() local p=GetBot():GetLocation(); assert(GetHeightLevel(p)==(GetTeam()==2 and {expected} or {dire})); assert(GetHeightLevel(Vector(p.x,p.y,999))==GetHeightLevel(p)) end").as_bytes());
}
#[test]
fn stock_reads_the_receiving_sides_public_shop_count() {
    run(br#"function Think() assert(GetItemStockCount('item_branches')==-1); assert(GetItemStockCount('item_infused_raindrop')==0) end"#);
}
#[test]
fn location_visibility_reuses_the_shared_sight_model_for_both_sides() {
    run(b"function Think() assert(IsLocationVisible(GetBot():GetLocation())); assert(not IsLocationVisible(Vector(GetTeam()==TEAM_RADIANT and 8500 or -8500,GetTeam()==TEAM_RADIANT and 8500 or -8500,0))) end");
}
#[test]
fn tree_location_resolves_static_public_id_and_returns_zero_for_an_invalid_id() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let point = sessions[0].statics().trees()[0];
    run(format!("function Think() local p=GetTreeLocation(0); assert(p.x=={} and p.y=={}); local bad=GetTreeLocation(-1); assert(bad.x==0 and bad.y==0) end",f64::from(point.x.raw)/65536.0-9216.0,f64::from(point.y.raw)/65536.0-9216.0).as_bytes());
}
