# API family extension contract

`api/context.rs` pins the internal interface for adding API families. The scheduler
and worker own the context; callbacks implement engine API behavior only.

```rust,ignore
fn get_something(context: &ApiContext, vm: &mut Vm, args: Args)
    -> Result<usize, LuaError>;
```

Register globals with `register_global`, unit methods with `register_unit`, and
ability/item methods with `register_method` and their installed method `TableId`.
Callbacks return the result count from `vm.results(&[Value, ...])`, or `Ok(0)`.
Arguments are zero-based. A method's argument 0 is its receiver, argument 1 its
first explicit argument. Values, strings, tables and userdata belong to luagee;
never construct interpreter handles or inspect interpreter internals.

Resolve unit arguments through `context.receiver` / `context.unit_id`. Units are
stable, pinned Lua tables, with identity held in the host map. Script fields and
metatables do not establish identity. Ability/item identities are resolved through
`api/abilities.rs`; validate against the current slot before issuing a cast/use.
New identity kinds or host facts must be coordinated with the host owner.

`context.state` is `Rc<RefCell<State>>`, local to one interpreter. `State.snapshot`
is a bounded copy of fog-filtered `ClientSession` facts, never a `World` or a full
server view. `State.current_slot` selects the active bot; `snapshot.players` is the
stable unit-to-seat authority even when a unit track expires. `State.unit` reads
remembered unit data; `KnownUnit.visible` distinguishes current sight. Nearby
queries must filter visible records. Do not infer an enemy's hidden fields.

Borrow state briefly, copy needed facts, and release the borrow before any Lua
reentry. No binding callback presently reenters Lua. Charge work with
`vm.charge_fuel` before scans, sorting, allocation or quadratic/cubic geometry.
Propagate `LuaError::Limit` unchanged. Required numeric arguments must be finite;
use `vector::number` and `api/globals::integer/string`, which validate/charge input.
Positions use `vector::position` and `context.vectors.location`, preserving the
coordinate shift and nearest-even fixed-point conversion. All retained storage
must fit the host reserve or a rooted charged userdata allocation.

`context.issue_hero` checks active-bot ownership and appends an ordinary pending
`IssuedOrder`. `context.issue_move` additionally accepts that seat's currently
visible, living courier and emits an ordinary courier movement order. It checks
current kind, owner and side; a remembered handle alone grants no control.
The scheduler commits only after all callbacks for that bot succeed;
on error or fuel exhaustion it discards the bot's entire outbox. The native worker
discards late outputs. Actions must not change simulation state or call a `World`.

Three complete patterns are in `api/patterns.rs`:

- `GetBot`: global → active seat identity → stable unit table.
- `GetHealth`: receiver → filtered snapshot → Lua number, including dead heroes.
- `Action_MoveToLocation`: receiver + typed Vector → ordinary Move order.

They run through the same installation path as other families. Tests `match.rs`,
`api.rs` and `identity.rs` exercise their real `LocalMatch` order/state path;
`hostile.rs` checks forged receivers and malformed numeric/vector arguments.

## Engine values and observations

Signatures and result categories follow the [bot API dump](https://docs.moddota.com/lua_bots/)
and [engine enums](https://docs.moddota.com/lua_bots_enums/). Lua numbers represent
integer results too. Times are seconds; positions are world units. Engine XY
coordinates are protocol XY minus 9,216, with nearest-even Q16.16 conversion on
orders. Engine lane IDs TOP=1, MID=2, BOT=3 are translated explicitly to core lanes.
Vectors expose both `x/y/z` and the one-based numeric components `1/2/3`.

`GetOrigin` is an explicit legacy entity-position compatibility extension, rather
than a canonical bot-dump method. The lower-level engine
[CBaseEntity declaration](https://docs.moddota.com/lua_server/#CBaseEntity) lists
`GetOrigin(): vector`. Map units have no parent transform here, so it shares the
authenticated `GetLocation` callback, coordinate conversion and remembered position.
Owned heroes retain their last received body position after death and track
expiry, using the session's body memory. Forgotten foreign units raise
`unit is no longer known`; their remembered body memory is not a position fallback.
This changes the pre-existing `GetLocation` zero-vector fallback as well as
`GetOrigin`, preventing forgotten targets from being mapped to the world origin.
It introduces no additional facts.

`GetGameMode` maps the configured Solo Mid map to engine mode 21; a map without
an explicit engine-mode mapping raises an ordinary Lua error. `GetAssignedLane`
returns engine MID (2) for roster heroes in Solo Mid. Other units, and maps with
no host lane assignment, return NONE (0). Assignment does not depend on a unit's
current position or on hidden facts. These queries require a received view.

`GetBarracks` and `GetAncient` resolve building positions from the public map,
then return an authenticated handle only for a living building in the received
side's current sight. A remembered enemy building does not disclose its current
existence or health through these lookups. Missing or unseen buildings return nil.
The current barracks mapping is UNSOURCED: provisional indices are TOP
melee/ranged (0/1), MID (2/3), BOT (4/5). A primary engine constant declaration or
live engine observation must verify this order before claiming parity. The tests
check identities against the public map catalogue, not this provisional table.
Other nonnegative engine integers return nil. Invalid teams, negative/fractional indices and values
outside the engine integer domain raise ordinary Lua errors.

`Min` and `Max` compare two supplied Lua numbers. Both operands must be finite
and within the engine float range; negative values are valid. Malformed or
out-of-range operands raise ordinary Lua errors before comparison.

Player ids are engine ids: Radiant 0–4 and Dire 5–9. Active-view queries derive
each side's rank from its public roster's wire seat order; wire seat 1 can therefore
be engine player 5. The host continues to use wire seats internally. Team arrays
contain occupied roster entries only. Out-of-range or fractional player/team ids
raise ordinary Lua errors; an unoccupied player id supplies no fabricated row.
Before a roster is received, a selected hero name is supplied only when the
configured seats prove its rank without assuming any missing lower seat's side.
Other names remain empty, and unproven ids do not select a seat. `SelectHero`
and `GetSelectedHeroName` use the same engine-id resolver. This does not
reconstruct a selection-phase roster.
Roster queries require a received view; this host's fixed selection mode skips
pack selection scripts. In an active view, an unoccupied hero-name lookup returns
an empty string; queries requiring a player row report its absence as an error.
`GetHeroLastSeenInfo` returns an array with the last received `location` and
`time_since_seen`, or an empty array when no corresponding observation remains. It uses the
session's remembered body even after short-lived entity tracking expires.
When the public roster withholds the real hero's identity, last-seen results list
received hero-body candidates instead of identifying a real hero or an illusion.

`IsPlayerBot` is a host-declared fact. Configured scripted seats always answer
true. The trusted launcher may additionally set `NativeConfig.declared_bot_slots`
or call `LuaTeam::set_bot_declarations` before execution. These declarations use
wire seat ids, contain at most ten distinct entries in 0–9, and default to empty.
Every other seat answers false, conservatively treating an undeclared controller
as human. This does not disclose controller information from the wire.

New bounded integer arguments use `api/globals::bounded_integer`: finite whole
numbers must fall within the specified bounds before conversion. The older
general numeric helper itself is unchanged; migrated calls validate their own bounds.

## Team lane desires

`GetPushLaneDesire(lane)` and `GetDefendLaneDesire(lane)` read this Lua team's
validated cache. Lane ids must be actual finite whole numbers; numeric strings
are rejected. Arguments use engine TOP=1, MID=2, BOT=3; they are translated
through the typed lane interface, not used as core indices (MID=0, TOP=1, BOT=2).
An absent override returns the native baseline zero.

`UpdatePushLaneDesires` and `UpdateDefendLaneDesires` are both attempted. Their
pair publishes immediately when both succeed, independently of errors in
`TeamThink`, farm, roam or Roshan hooks. Either lane-hook failure retains the
previous pair; with no previous pair both queries return zero. A missing or nil
return resets its own override when the pair succeeds. Table results require
exactly numeric keys 1–3, with finite values in [0,1]. Sparse trailing entries
and other extra fields are rejected. The decoder reads values raw and checks
cardinality with at most four calls to a captured original `next`, under the
same fuel budget. It does not invoke table metamethods, and normalizes negative
zero to positive zero.

Every team hook is attempted even after another fails; the first error is
reported. Later errors increment `HookActivity::suppressed_team_errors`; native
warnings include that count and match `script_failures` counts all errors.
A team-hook error discards orders only for the first configured Lua seat, which
need not have wire slot zero; other configured seats still run. The desire cache
is script decision state and is not rewound with rejected or late command
continuations. Queries expose no other team's cache or server-only state.

Unknown ability names and empty or invalid item slots return nil. Ability names
come from core definitions, including the hero's separate innate and talent tree.
Unlearned abilities retain handles at level zero. Innates have their granted level
and no cast or learning slot. Owned kit and talent views preserve handles while a
hero is dead. Item slots are inventory 0–5, backpack 6–8, stash 9–14 and TP 15;
backpack and stash items are not fully castable.

Nearby and AoE queries consider only current visible units. AoE delay extrapolates
the observed displacement between visible samples; it reads no standing enemy
order. Remembered observations stop updating in fog. Private inventory, gold and
talent eligibility are copied only for the receiving side. Stock and catalogue
information come from the received view and announced match catalogue.

`GetUnitList` returns fresh arrays of currently visible living units, using the
`UNIT_LIST_*` categories. Remembered enemy handles do not keep a hidden enemy in
these arrays. Creeps and buildings use the public core kind metadata; enemy-side
categories do not classify neutral units as members of the opposing team.

Unit statistics read the received whole-point mana and damage fields without
narrowing them through fixed-point conversion. Floating-point regeneration,
armor, evasion and bounding radius retain Q16.16 precision. Integer range, speed,
attribute and facing reads return whole units; facing uses engine degrees rather
than protocol brads. Attack intervals are returned in seconds.
`GetAttackPoint` and `GetAttackSpeed` use INFERRED compatibility semantics,
pending live engine verification: the point is base seconds before attack-speed
scaling, and speed is a multiplier with 1.0 at 100 attack speed. The point comes
from the public core unit definition; the speed comes from the received raw
attack speed. The speed-adjusted wire point is not multiplied back, avoiding
millisecond-rounding drift. The purchase test pins this chosen contract, not a
verified engine convention. Unknown base-body metadata raises a normal Lua error.
Owned scalar statistics remain available from the session's received body memory
while a dead hero's short-lived entity track is absent. They remain last-observed
values until a new body observation arrives; this does not refresh hidden data.

Modifier indices are zero-based positions in the received effect list. Names come
from the public effect catalogue. An absent named modifier returns index -1;
uncounted stacks return zero and a permanent duration returns -1. Condition reads
use received status flags or effect ids. In particular, `IsIllusion` tests only
the reported illusion effect; it does not identify an enemy real hero from
information withheld by the side's view.

Ability and item mana cost, remaining cooldown and current charges come from
the received ability book or inventory, including an owned dead hero's kit.
Cooldowns are seconds; readiness tests the cooldown alone, independently of mana
or status effects. Items without charges return zero. `AbilityView` has no charge
field: `GetCurrentCharges` currently returns zero for every ability, including
charged abilities. That value does not establish that an ability is exhausted.
The new
state reads reject forgotten ability/item handles. An active innate without a
received cooldown slot reports that the cooldown is unobserved; passive innates
and talents have no running cooldown.
Enemy item cooldowns are withheld by the producer, even when the item is visible.
`GetCooldownTimeRemaining`, `IsCooldownReady` and `IsFullyCastable` raise
`cooldown is not observed` for those items instead of treating the placeholder
zero as readiness. This also changes the older item `IsFullyCastable` behavior.
The carrier's side determines observation, rather than the item's original buyer;
own-side items and owned dead-hero kits retain their received cooldowns.

Estimated damage is an integer projection of observed attack stats and core spell
data over the requested duration. The availability flag restricts allied spells
and items by the known mana and cooldowns; it does not disable ordinary attacks.
Enemy availability is assumed, as specified by the API. Mitigation and spell
piercing use core rules, without positional assumptions.

Purchase results reserve gold, stock and placement for the team's current turn.
They use core recipe and stacking rules. A failed bot turn releases its own
reservations; subsequent snapshots replace projections with received facts. The
legacy `Action_PurchaseItem` and `Action_LevelAbility` names share the immediate
callbacks. Orders remain ordinary scheduler outbox entries.

Once attacks complete from `AttackLaunched` facts whose envelope tick is later
than the request, retained in the client's bounded entity memory rather than its
recent-event ring. The completion pass queues Stop before the next script hooks.
Only retained launch facts complete a request; projectile and impact observations
are not used to infer completion.

Tracked unit and ability tables are pinned only while their received identity remains
tracked (player identities and owned death kits remain stable). Historical tables
kept by Lua retain authenticated identity and fields, report null/none values while
forgotten, and are reused if that identity is observed again. Opaque witnesses in
protected metatables carry weak host-index entries charged to interpreter memory;
collection removes those entries and shrinks their capacity. Copying script fields
or borrowing a method cannot create an engine identity.

The README's “Native reconstruction evidence” table classifies the native
policy constants and inferred engine player/mode/lane ids. These bindings do not
claim a live-engine verification of those mappings. Explicit script attack-once
requests have no native policy lease; only native last-hit requests expire after
the bounded, windup-aware lease. A rejected or lost native order can therefore
recover without requiring a launch event or a gap-free stream of snapshots.

## Compatibility and input behavior

Unless a family specifies a stricter type requirement, numeric arguments retain
Lua's C API `checknumber` coercion: numeric strings such
as `GetHeroLevel("0")`, `GetBarracks(2,"0")` and `Min("1",2)` are accepted. Finite,
integer and range checks still apply after conversion; NaN, infinities and
out-of-domain values raise the same errors as their numeric equivalents.

`SelectHero` and `GetSelectedHeroName` now use engine player ids 0..9 and reject
values outside that range. This replaces the older acceptance of arbitrary
integers and wire-seat interpretation. In particular, a Dire wire seat 1 can be
engine player 5; scripts must use the engine id. Configured seats and received
rosters with more than five players on either side are rejected explicitly rather
than dropping unmappable players from roster results.

Constant-work utility and position queries explicitly charge eight fuel units
for their host work. Families that already charge for reads or scans retain their
own cost models; registration adds no implicit surcharge. Allocations and
retained handles continue to pay their additional charges before work.
