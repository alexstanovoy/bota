//! What the reference game answers for receivers and arguments it rejects, and the gaps this host
//! declares instead of inventing a value. Every case runs a script against a seat that was told
//! the scenario (edited views folded through the session, as a server would send them).
mod support;
use bota_client_core::ClientSession;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind, TeamTurn};
use bota_proto::{
    DamageKind, EntityId, EventKind, HeroId, ServerMsg, SlotId, Team, UnitKind, UnitView, Vec2,
    WorldView,
};

/// Seat 0 is the bot; seat 1 is an ally; seat 2 is the enemy. All start at their fountains.
fn trio() -> Vec<ClientSession> {
    support::local(&[Team::Radiant, Team::Radiant, Team::Dire]).1
}

/// The trio plus a second enemy seat, for tests that need two enemy players.
fn quartet() -> Vec<ClientSession> {
    support::local(&[Team::Radiant, Team::Radiant, Team::Dire, Team::Dire]).1
}

fn run(session: &ClientSession, declared: &[SlotId], source: &[u8]) -> TeamTurn {
    let mut team = LuaTeam::new_measured(
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
        true,
    )
    .unwrap();
    team.set_bot_declarations(declared).unwrap();
    team.add_script(SlotId(0), ScriptKind::Takeover, "reference_gates", source)
        .unwrap();
    team.think(session).unwrap()
}

fn passes(session: &ClientSession, declared: &[SlotId], source: &[u8]) -> TeamTurn {
    let turn = run(session, declared, source);
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    turn
}

fn declined(turn: &TeamTurn) -> Vec<(String, u64)> {
    turn.statistics
        .as_ref()
        .expect("measured")
        .declined_api_calls
        .names
        .iter()
        .map(|entry| (entry.name.clone(), entry.reads))
        .collect()
}

/// Folds one more tick: the given edit of the last view, then the events of that tick.
fn tick(session: &mut ClientSession, edit: impl FnOnce(&mut WorldView), events: Vec<EventKind>) {
    let mut view = session.current().expect("view").clone();
    view.tick += 1;
    edit(&mut view);
    view.units.sort_by_key(|unit| unit.id);
    let tick = view.tick;
    session
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("snapshot");
    session
        .ingest(ServerMsg::Events { tick, events }, &mut ())
        .expect("events");
}

fn body_like(model: &UnitView, id: u32, kind: UnitKind, team: Team, at: Vec2) -> UnitView {
    let mut unit = model.clone();
    unit.id = EntityId {
        idx: id,
        generation: 1,
    };
    unit.kind = kind;
    unit.team = team;
    unit.pos = at;
    if team != model.team {
        for ability in &mut unit.abilities {
            ability.learnable = None;
        }
        for item in unit.items.iter_mut().flatten().chain(&mut unit.town_portal) {
            item.selling = None;
        }
    }
    if kind == UnitKind::Hero && team != model.team {
        unit.owner = Some(SlotId(2));
    }
    if kind != UnitKind::Hero {
        unit.hero = None;
        unit.owner = None;
        unit.abilities.clear();
        unit.items.clear();
        unit.effects.clear();
    }
    unit
}

fn damage(source: EntityId, target: EntityId) -> EventKind {
    EventKind::Damaged {
        source: Some(source),
        target,
        amount: 40,
        kind: DamageKind::Physical,
        inflictor: None,
        removal: false,
    }
}

fn own_body(session: &ClientSession) -> UnitView {
    session.own_hero().expect("own hero").clone()
}

fn nearby(at: Vec2, by: i32) -> Vec2 {
    at + Vec2::from_ints(by, 0)
}

#[test]
fn damage_queries_decline_a_receiver_that_is_not_a_bot_teammate_instead_of_answering_silently() {
    let mut sessions = trio();
    let session = &mut sessions[0];
    let me = own_body(session);
    let enemy = body_like(&me, 70_001, UnitKind::Hero, Team::Dire, nearby(me.pos, 200));
    let enemy_creep = body_like(
        &me,
        70_002,
        UnitKind::CreepMelee,
        Team::Dire,
        nearby(me.pos, 300),
    );
    let allied_creep = body_like(
        &me,
        70_003,
        UnitKind::CreepMelee,
        Team::Radiant,
        nearby(me.pos, 400),
    );
    let creep_id = enemy_creep.id;
    tick(
        session,
        |view| view.units.extend([enemy, enemy_creep, allied_creep]),
        vec![damage(creep_id, me.id)],
    );
    let turn = passes(
        session,
        &[],
        b"function Think()
  local me = GetBot()
  assert(me:WasRecentlyDamagedByCreep(5) == true, 'the bot itself is answered from its journal')
  local enemy = GetUnitList(UNIT_LIST_ENEMY_HEROES)[1]
  local foe_creep = GetUnitList(UNIT_LIST_ENEMY_CREEPS)[1]
  local ally_creep = GetUnitList(UNIT_LIST_ALLIED_CREEPS)[1]
  assert(enemy and foe_creep and ally_creep)
  assert(enemy:WasRecentlyDamagedByCreep(5) == false)
  assert(enemy:WasRecentlyDamagedByHero(me, 5) == false)
  assert(foe_creep:WasRecentlyDamagedByCreep(5) == false)
  assert(ally_creep:WasRecentlyDamagedByHero(enemy, 5) == false)
end",
    );
    assert_eq!(
        declined(&turn),
        [
            ("WasRecentlyDamagedByCreep_not_a_bot_teammate".to_owned(), 2),
            ("WasRecentlyDamagedByHero_not_a_bot_teammate".to_owned(), 2),
        ],
        "each rejected receiver is counted under its API and reason"
    );
}

#[test]
fn an_allied_hero_is_answered_from_the_journal_only_when_it_is_declared_a_bot() {
    let mut sessions = trio();
    let session = &mut sessions[0];
    let me = own_body(session);
    let ally = session
        .current()
        .unwrap()
        .units
        .iter()
        .find(|unit| unit.kind == UnitKind::Hero && unit.team == Team::Radiant && unit.id != me.id)
        .expect("the allied hero is told to its side")
        .clone();
    let creep = body_like(
        &me,
        70_010,
        UnitKind::CreepMelee,
        Team::Dire,
        nearby(ally.pos, 100),
    );
    let creep_id = creep.id;
    tick(
        session,
        |view| view.units.push(creep),
        vec![damage(creep_id, ally.id)],
    );
    let declared = passes(
        session,
        &[SlotId(1)],
        &b"function Think()
  local ally = GetUnitList(UNIT_LIST_ALLIED_HEROES)[2]
  assert(ally and ally ~= GetBot())
  assert(ally:WasRecentlyDamagedByCreep(5) == true, 'a declared bot ally has a damage journal')
  assert(ally:WasRecentlyDamagedByCreep(0) == false, 'a zero interval never covers the damage')
end"[..],
    );
    assert_eq!(
        declined(&declared),
        [],
        "a declared bot is a valid receiver"
    );
    let undeclared = passes(
        session,
        &[],
        &b"function Think()
  local ally = GetUnitList(UNIT_LIST_ALLIED_HEROES)[2]
  assert(ally and ally ~= GetBot())
  assert(ally:WasRecentlyDamagedByCreep(5) == false, 'the reference has no history for a hero that is not a bot')
end"[..],
    );
    assert_eq!(
        declined(&undeclared),
        [("WasRecentlyDamagedByCreep_not_a_bot_teammate".to_owned(), 1)]
    );
}

#[test]
fn hero_damage_counts_for_the_attacking_seat_whichever_of_its_bodies_struck() {
    let mut sessions = quartet();
    let session = &mut sessions[0];
    let me = own_body(session);
    let hero = body_like(&me, 70_020, UnitKind::Hero, Team::Dire, nearby(me.pos, 300));
    let mut other = body_like(&me, 70_021, UnitKind::Hero, Team::Dire, nearby(me.pos, 500));
    let mut illusion = body_like(&me, 70_022, UnitKind::Hero, Team::Dire, nearby(me.pos, 700));
    let owner = SlotId(2);
    let mut hero = hero;
    hero.owner = Some(owner);
    illusion.owner = Some(owner);
    other.owner = Some(SlotId(3));
    let illusion_id = illusion.id;
    tick(
        session,
        |view| view.units.extend([hero, other, illusion]),
        vec![damage(illusion_id, me.id)],
    );
    let turn = passes(
        session,
        &[],
        &b"function Think()
  local me = GetBot()
  local seen = {}
  for _, h in ipairs(GetUnitList(UNIT_LIST_ENEMY_HEROES)) do
    seen[#seen + 1] = {h, me:WasRecentlyDamagedByHero(h, 5)}
  end
  assert(#seen == 3, 'three enemy bodies are visible: ' .. #seen)
  local hits = 0
  for _, entry in ipairs(seen) do if entry[2] then hits = hits + 1 end end
  assert(hits == 2, 'the hero and its illusion share the seat that struck: ' .. hits)
end"[..],
    );
    assert_eq!(declined(&turn), []);
}

#[test]
fn the_damage_window_excludes_the_age_equal_to_the_interval() {
    let mut sessions = trio();
    let session = &mut sessions[0];
    let me = own_body(session);
    let creep = body_like(
        &me,
        70_030,
        UnitKind::CreepMelee,
        Team::Dire,
        nearby(me.pos, 100),
    );
    let creep_id = creep.id;
    tick(
        session,
        |view| view.units.push(creep),
        vec![damage(creep_id, me.id)],
    );
    let struck = session.current().unwrap().tick;
    for _ in 0..29 {
        tick(session, |_| {}, Vec::new());
    }
    assert_eq!(session.current().unwrap().tick, struck + 29);
    passes(
        session,
        &[],
        &b"function Think() assert(GetBot():WasRecentlyDamagedByCreep(1) == true, 'age 29 ticks is inside one second') end"[..],
    );
    tick(session, |_| {}, Vec::new());
    passes(
        session,
        &[],
        &b"function Think()
  assert(GetBot():WasRecentlyDamagedByCreep(1) == false, 'age 30 ticks is not inside one second')
  assert(GetBot():WasRecentlyDamagedByCreep(1.01) == true)
  assert(GetBot():WasRecentlyDamagedByCreep(0) == false)
end"[..],
    );
}

#[test]
fn stash_value_is_minus_one_for_a_non_bot_and_nil_for_a_stash_the_seat_never_received() {
    let mut sessions = trio();
    let session = &mut sessions[0];
    let turn = passes(
        session,
        &[],
        &b"function Think()
  local ally = GetUnitList(UNIT_LIST_ALLIED_HEROES)[2]
  assert(ally and ally ~= GetBot())
  assert(GetBot():GetStashValue() == 0, 'an empty own stash is worth nothing')
  assert(ally:GetStashValue() == -1, 'an ally that is not a declared bot has no readable stash')
end"[..],
    );
    assert_eq!(
        declined(&turn),
        [("GetStashValue_not_a_bot_teammate".to_owned(), 1)]
    );
}

#[test]
fn a_hero_whose_body_was_forgotten_still_counts_through_its_player_record() {
    let mut sessions = trio();
    let session = &mut sessions[0];
    let me = own_body(session);
    let scoreboard = session
        .current()
        .unwrap()
        .players
        .iter()
        .find(|player| player.slot == SlotId(2))
        .and_then(|player| player.unit)
        .expect("the enemy seat names its hero on the scoreboard");
    let mut enemy = body_like(&me, 70_040, UnitKind::Hero, Team::Dire, nearby(me.pos, 300));
    enemy.id = scoreboard;
    tick(
        session,
        |view| view.units.push(enemy),
        vec![damage(scoreboard, me.id)],
    );
    let mut team = LuaTeam::new(
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
    team.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "forgotten_hero",
        b"local foe
function Think()
  if not foe then foe = GetUnitList(UNIT_LIST_ENEMY_HEROES)[1] assert(foe) return end
  assert(GetBot():WasRecentlyDamagedByHero(foe, 20) == true, 'the hero struck 500 ticks ago')
end",
    )
    .unwrap();
    let first = team.think(session).unwrap();
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    tick(
        session,
        |view| view.units.retain(|unit| unit.id != scoreboard),
        Vec::new(),
    );
    for _ in 0..485 {
        tick(session, |_| {}, Vec::new());
    }
    assert!(
        session.entity(scoreboard).is_none(),
        "the enemy's track expired after 480 ticks in the fog"
    );
    let later = team.think(session).unwrap();
    assert!(later.failures.is_empty(), "{:?}", later.failures);
}
