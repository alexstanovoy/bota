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
reported. A team-hook error discards orders only for the first configured Lua seat, which
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
