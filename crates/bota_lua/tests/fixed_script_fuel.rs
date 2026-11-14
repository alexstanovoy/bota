//! The fuel a fixed script needs per tick, pinned exactly. Publishing more API names (the constants of 4c-1 took the
//! global catalogue from 185 to 437 entries) moves these numbers by a fixed amount per tick (about 127) for every script
//! that runs more than an empty `Think`: a catalogue change that shifts them must re-pin them here and say so in
//! API.md, "Fuel and the catalogue".
mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};

/// The empty `Think` is the baseline: the catalogue does not touch its price.
const EMPTY_THINK: u64 = 284;
/// A hundred additions in a loop: no global, no allocation (before the constants: 596).
const HUNDRED_ADDITIONS: u64 = 723;
/// The empty `Think` plus 100 reads of a name the game does not define (before the constants: 969).
const HUNDRED_MISSES: u64 = 1096;
/// The empty `Think` plus 100 reads of a defined name (before the constants: 967).
const HUNDRED_HITS: u64 = 1095;

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

fn completes(measured: bool, fuel_per_tick: u64, source: &[u8]) -> bool {
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
    team.think(&sessions[0]).unwrap().failures.is_empty()
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
fn an_empty_think_needs_exactly_its_pinned_fuel_per_tick_with_and_without_measurement() {
    assert_exact_minimum("empty Think", EMPTY, EMPTY_THINK);
}

#[test]
fn a_hundred_additions_need_exactly_their_pinned_fuel_per_tick_with_and_without_measurement() {
    assert_exact_minimum("100 additions", ADDITIONS, HUNDRED_ADDITIONS);
}

#[test]
fn a_hundred_undefined_reads_need_exactly_their_pinned_fuel_per_tick_with_and_without_measurement()
{
    assert_exact_minimum("100 undefined reads", MISSES, HUNDRED_MISSES);
}

#[test]
fn a_hundred_defined_reads_need_exactly_their_pinned_fuel_per_tick_with_and_without_measurement() {
    assert_exact_minimum("100 defined reads", HITS, HUNDRED_HITS);
}
