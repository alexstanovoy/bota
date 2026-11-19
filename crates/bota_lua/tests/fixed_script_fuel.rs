//! The fuel a fixed script needs per tick, pinned exactly, on the first `Think` and on the ticks after it. The host's
//! API install ends with one collection run by the host (`LuaTeam::new`), so a script is not billed for tracing the
//! host's catalogue: these numbers do not depend on how many names the API publishes. If one moves, the collector's
//! schedule or a price changed; re-pin it here and in API.md, "Fuel and the catalogue".
mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};

/// The least fuel per tick on the first `Think` of an empty `Think`.
const FIRST_EMPTY_THINK: u64 = 16;
/// A hundred additions in a loop: no global, no allocation.
const FIRST_HUNDRED_ADDITIONS: u64 = 221;
/// 100 reads of a name the game does not define, then one move order.
const FIRST_HUNDRED_MISSES: u64 = 724;
/// 100 reads of a defined name, then one move order.
const FIRST_HUNDRED_HITS: u64 = 724;
/// The least fuel per tick for the ticks 2 to 6 of the additions script.
const STEADY_HUNDRED_ADDITIONS: u64 = 207;

const EMPTY: &[u8] = b"function Think() end";
const ADDITIONS: &[u8] = b"function Think() local n = 0 for i = 1, 100 do n = n + i end end";
const MISSES: &[u8] = br#"
function Think()
  local n = 0
  for i = 1, 100 do if BOT_NOT_A_CONSTANT_XYZ == nil then n = n + 1 end end
  GetBot():Action_MoveToLocation(Vector(n, 0, 0))
end
"#;
const HITS: &[u8] = br#"
function Think()
  local n = 0
  for i = 1, 100 do if TEAM_RADIANT == 2 then n = n + 1 end end
  GetBot():Action_MoveToLocation(Vector(n, 0, 0))
end
"#;

/// Whether each of the first `ticks` ticks of the script completes with `fuel_per_tick`.
fn verdicts(measured: bool, fuel_per_tick: u64, ticks: usize, source: &[u8]) -> Vec<bool> {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut team = LuaTeam::new_measured(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT.with_fuel_per_tick(fuel_per_tick),
        Keys {
            hash: [17, 19],
            random: 23,
        },
        measured,
    )
    .unwrap();
    team.add_script(SlotId(0), ScriptKind::Takeover, "fixed_script", source)
        .unwrap();
    let mut session = sessions.into_iter().next().unwrap();
    (0..ticks)
        .map(|_| {
            let mut view = session.current().expect("view").clone();
            view.tick += 1;
            let tick = view.tick;
            session
                .ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ())
                .expect("snapshot");
            session
                .ingest(
                    bota_proto::ServerMsg::Events {
                        tick,
                        events: Vec::new(),
                    },
                    &mut (),
                )
                .expect("events");
            team.think(&session).unwrap().failures.is_empty()
        })
        .collect()
}

fn completes(measured: bool, fuel_per_tick: u64, source: &[u8]) -> bool {
    verdicts(measured, fuel_per_tick, 1, source)[0]
}

/// `exact` is the least fuel per tick that lets the script finish: it finishes with `exact` and not with one less.
fn assert_exact_minimum(label: &str, source: &[u8], exact: u64) {
    for measured in [false, true] {
        assert!(
            completes(measured, exact, source),
            "{label} must finish with {exact} fuel per tick (measured={measured})"
        );
        assert!(
            !completes(measured, exact - 1, source),
            "{label} must not finish with {} fuel per tick (measured={measured})",
            exact - 1
        );
    }
}

#[test]
fn the_first_empty_think_needs_exactly_its_pinned_fuel_with_and_without_measurement() {
    assert_exact_minimum("empty Think", EMPTY, FIRST_EMPTY_THINK);
}

#[test]
fn the_first_think_of_a_hundred_additions_needs_exactly_its_pinned_fuel_with_and_without_measurement()
 {
    assert_exact_minimum("100 additions", ADDITIONS, FIRST_HUNDRED_ADDITIONS);
}

#[test]
fn the_first_think_of_a_hundred_undefined_reads_needs_exactly_its_pinned_fuel_with_and_without_measurement()
 {
    assert_exact_minimum("100 undefined reads", MISSES, FIRST_HUNDRED_MISSES);
}

#[test]
fn the_first_think_of_a_hundred_defined_reads_needs_exactly_its_pinned_fuel_with_and_without_measurement()
 {
    assert_exact_minimum("100 defined reads", HITS, FIRST_HUNDRED_HITS);
}

#[test]
fn the_later_ticks_of_a_hundred_additions_need_exactly_their_pinned_fuel_with_and_without_measurement()
 {
    for measured in [false, true] {
        let enough = verdicts(measured, STEADY_HUNDRED_ADDITIONS, 6, ADDITIONS);
        assert!(
            enough[1..].iter().all(|tick| *tick),
            "ticks 2 to 6 must finish with {STEADY_HUNDRED_ADDITIONS} fuel (measured={measured}): {enough:?}"
        );
        let short = verdicts(measured, STEADY_HUNDRED_ADDITIONS - 1, 6, ADDITIONS);
        assert!(
            short[1..].iter().any(|tick| !*tick),
            "ticks 2 to 6 must not all finish with {} fuel (measured={measured}): {short:?}",
            STEADY_HUNDRED_ADDITIONS - 1
        );
    }
}
