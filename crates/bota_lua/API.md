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

`IsFullyCastable` uses shared admission readiness before choosing a target; it checks
learned state, cooldown, mana, charges, backpack inactivity and visible disables.
Owned illusions are never fully castable. Negative costs return false. Abilities whose
data permits casting while stunned retain that exception (currently a neutral ability,
not a hero-kit ability). A stored Bottle rune needs no charge. An item that requires charges
but has no received charge fact is not fully castable. Uncertain rounded mana (including a Mango
at its shown maximum) returns false. Unknown root/hidden-control state returns true
when the other readiness checks pass, so an unexposed hook followthrough does not
make every ability of its caster permanently unavailable. `CanAbilityBeUpgraded`
requires an affirmative own-side learnability fact; an absent fact returns false.
The [engine API dump](https://github.com/ModDota/API/blob/master/dump/script_help2.lua)
only specifies a boolean castability result; it does not specify partial-information
fallbacks. These fallbacks are explicit host approximations: identical filtered
views can correspond to different engine answers. Neither method promises that
an order will execute or have an effect.

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
Cooldowns are seconds; `IsCooldownReady` tests the cooldown alone, independently of mana
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
data over the requested duration. For allied casters, `available=true` uses the same
shared readiness and uncertainty fallback as `IsFullyCastable`, including known
disables and inventory copies, plus the remaining projected mana budget. An allied caster
that is stunned or feared has its cast abilities and items excluded, subject to an ability's
data-defined casting exception. Ordinary attacks and passive projections, including continuous
passive effects, do not require cast readiness. Enemy
availability is assumed, as specified by the API; withheld cooldown placeholders
are not used as evidence of readiness. Mitigation and spell piercing use core
rules, without positional assumptions.

Purchase results reserve gold, stock and placement for the team's current turn.
They use core recipe and stacking rules. A failed bot turn releases its own
reservations; subsequent snapshots replace projections with received facts. The
legacy `Action_PurchaseItem` and `Action_LevelAbility` names share the immediate
callbacks. Orders remain ordinary scheduler outbox entries.

`GetBuybackCost()`, `GetBuybackCooldown()` and `HasBuyback()` read what the bot's own
side is told of a teammate hero's buyback, as Dota's getters do. The cost is
`200 + net worth / 13`. `GetBuybackCooldown()` is the game time (on `GameTime()`'s
clock) buyback is available from, not the seconds left: 0 before any buyback, a past
time once the cooldown is over. `HasBuyback()` is the engine's own test, a cooldown
strictly past and gold strictly above the cost (the gold left after this turn's
purchases); it asks neither whether the hero lies dead nor whether a buyback given
exactly the cost would pass, which it would. For any unit that is no teammate hero
(another side's buyback is never received) the three answer -1, -1 and false; what
Dota returns besides its logged warning for the last two was not established.
`ActionImmediate_Buyback()` is for the active bot alone: it orders a buyback when the
hero lies dead and is not rising by a reincarnation, the cooldown is over and the gold
left covers the cost, once a turn, and otherwise does nothing, as the engine's buyback
checks silently.

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

Ability classification reads validate the current authenticated handle first.
`IsTrained` reports whether its received level is positive; `IsTalent` identifies
an actual talent handle; `IsUltimate` uses the public core ability classification.
Items and talents are not ultimate abilities. These methods do not inspect
cooldown readiness, and forgotten ability/item handles raise an ordinary error.

`FindItemSlot` accepts an item internal-name string and returns the first matching
observed slot in the supported bag, backpack, stash and dedicated scroll layout,
or -1 when there is no observed match. It uses the same slot mapping as
`GetItemInSlot` and public core item keys, with case-sensitive matching. It reads
received storage, not pending purchase projections. Enemy stash is not searched,
and a hidden purchase does not update remembered inventory. Owned dead kits and
stash remain searchable; an expired foreign inventory raises an ordinary error.

The unit/location distance globals use planar engine world units, with squared
queries returning squared world units. They share the `GetLocation` observation
policy: own remembered body positions remain usable during respawn, while expired
foreign positions error. Vector inputs use the normal checked coordinate
conversion. Differences are widened before squaring and summing, so opposite
protocol-coordinate boundaries neither overflow nor saturate the distance.

`GetGlyphCooldown` returns seconds from the viewer's received `ready_after` field;
it never uses the retired zero-valued `ready_in` field or the other side's timer.
Missing cooldown knowledge raises an ordinary error. `ActionImmediate_Glyph`
requires the active bot's authenticated hero handle and emits an ordinary Glyph
request. Game-rule validation remains with the simulation; the getter does not
predict or reserve future activations.

`GetCurrentActiveAbility` returns the authenticated ability named by the received
cast or channel record; no active record returns nil. Public roster death returns
nil for any hero, even while its retained body still carries a pre-death cast.
A known foreign unit that is not currently observed returns nil; a remembered
cast is not evidence of current activity. This includes foreign heroes whose
public roster identity remains known after their body track expires. Unknown
unit identities still raise `unit is no longer known`. Own-unit observation
behavior is unchanged.
The cast record withholds
the identity of an item use; that case, or an ability missing from the received
book and public innate definition, raises `active ability is not observed` rather
than fabricating a handle or claiming that no cast exists. The signature follows
the [generated bot API](https://docs.moddota.com/lua_bots/)'s
`GetCurrentActiveAbility(): handle` declaration.

### Observed animation activity

`GetAnimActivity()` and `GetAnimCycle()` read the shared client activity model.
The former returns the registered `ACTIVITY_*` id, the latter a cycle in `[0,1)`.
The identifiers and unseen sentinels were established from the installed game,
build 6975.
An authenticated foreign unit outside current sight returns `-1` / `-1.0`, even
while its frozen last observation remains in memory. A forgotten non-roster unit
errors with `unit is no longer known`; forged handles are rejected.

The model observes movement, received action-phase counters, casts, channels,
forced movement and stun. Attack and cast phases use public catalogue
and received timing; loop durations come from trusted local model metadata
supplied before execution. Missing timing metadata returns -1 for the affected getter and preserves the
bot turn, rather than inventing a duration. Native loading reads that metadata only during
setup from the user's install; no model data is distributed with the binding.
The model is a deterministic client observation, not a claim of identical
reference-engine sequence selection. In particular, rare-idle selection and
attack alternation follow the client model. Attack and cast phases read the
received body's target-independent phase kind and raw elapsed animation beats, including
a visible attacker winding up at a hidden target. No target identity or location
is recovered. The clock freezes when the body leaves sight. Idle bodies omit the record. The server copies existing action progress only:
it does not read target properties, item definitions, durations or deadlines.
Clients derive display time from the public timebase. This observation does not
change simulation timing or orders. Channels use local loop metadata; an unknown
loop duration returns -1 instead of predicting a target-dependent end. Recovery
has no current ability identity and never inherits an earlier spell.

Each query charges eight fuel units plus handle validation and two units per
received body and roster row searched, above `bare_native_call_fuel()`.
The binding alone maps semantic activities to engine constants: idle, rare idle,
run, attack, alternate attack, flail, disabled, six cast and six channel slots,
death and spawn. Death/spawn/teleport transitions require observed signals; no
private controller state is read. Casts and channels without an observed ability
slot use the model's generic first cast/channel activity; this does not create an
ability handle or assert a reference-engine sequence selection.

### Incoming tracking projectiles

`GetIncomingTrackingProjectiles()` returns a fresh array of currently received
homing missiles whose target is this visible unit. The result fields were
established from the installed game, build 6975. Rows contain `location`,
`caster`, `playerid`, `ability`, `is_attack` and `is_dodgeable`. An unseen foreign
receiver returns nil; an unknown non-roster receiver errors. Only current filtered
records participate. A concealed caster stays nil and has playerid -1; it is never
recovered from remembered launch events. Ability/item handles are supplied only
when their current authenticated owner and slot are observed.

Attack classification follows the projectile's received inflictor fact. In the
current simulation, attacks are disjointable and spell missiles keep homing, so
`is_dodgeable` follows that rule; this is a simulation contract rather than a
claim that every reference spell shares the same disjoint behavior.

The query charges 16 host fuel plus two per body, roster row and projectile
searched; each result row adds 24 plus body/roster search work (two per row for an attack,
12 for an ability inflictor, 36 for an item inflictor). Tables,
vectors and authenticated handles pay their ordinary allocation charges. Empty
results still allocate and charge their result table.

Illusion attack progress is projected at its worn hero sheet's rate for both
wind-up and backswing; the real hero's current action is never copied. Cast
progress already uses the same unscaled timebase on both bodies. Turning has no
phase/progress field on this wire.


`CDOTA_Bot_Script` and `CDOTABaseAbility_BotScript` expose the shared unit and
ability/item method tables. Existing handles look up missing fields through those same tables:
patching a class method changes subsequent calls, while an instance field takes precedence.
These class names are established from the installed game, build 6975. Handles retain their
host-maintained identity checks; a class table or copied method is not a unit/ability identity.
Class access and script overrides spend normal interpreter table/instruction fuel; native calls
retain their existing explicit charges.


| Query | Observations and cost above the interpreter's native-call price |
|---|---|
| `GetShopLocation(team, shop)` | Public configured map geometry. `SHOP_HOME=0`, `SHOP_SIDE=1`, `SHOP_SECRET=2`, second side/secret ids 4/5; absent shop returns `Vector(0,0,0)`. Secret ids select the two neutral shops, independently of team. Home selects the team's fountain. 8 fuel. |
| `DistanceFromSideShop()` | Current maps have no side shop: the installed integer-distance sentinel is -2147483648. A forgotten identity errors. Checked receiver plus 8 fuel. |
| `GetNumCouriers()` / `GetCourier(index)` | Own side only; zero-based current courier roster, including fallen bodies. Out-of-range index returns nil; stable ordinary unit handles. 8 plus one per received/remembered body scanned, plus handle construction for a result. |
| `GetGameState()` | Selection=2, pregame=4, game in progress=5; uses the observed game clock, including clock cheats. The post-game constant is 6; no think runs after match end. 8 fuel. |
| `IsItemPurchasedFromSideShop(name)` | The current installed item data has no active SideShop flags, so false, including unknown names. 8 plus checked-string work. |

These enum values and absent-shop behavior were established from the installed game, build 6975;
locations and the current roster use the configured map and the receiving side's observations.


`GetNearbyTrees(radius)` returns a one-based Lua array of opaque tree ids from the client's
known forest, within the integer radius (0..32767). Static ids are match-tree indices; observed
planted positions receive stable ids after that prefix (at most 8192 retained planted identities).
Ids never reveal whether a tree outside sight has secretly been cut or planted. A foreign receiver
outside the current view returns nil; a forgotten owned receiver errors unless its body position
is retained. The call shape and tree query are established from the installed game, build 6975.
Host ids translate the configured match's forest; they are not native entity indexes.
Fuel: checked receiver, 8 plus received bodies scanned, each static tree and its binary-search
work in remembered felled indices, each planted tree scanned, each retained identity examined,
and each result, plus interpreter table/write charges. The scan and retained storage are bounded.


`GetLaneFrontAmount(team, lane, ignoreTowers)` returns progress in the viewer's orientation:
allied fronts advance upward from 0, enemy fronts downward from 1. It considers observed living
lane creeps within 1000 units of their lane and, unless ignored, observed towers of that team.
Those extrema, defaults, orientation and tower inclusion were established from the installed game,
build 6975. This host supports the single-wave-lane maps: their creep lane is public from the
configured wave lanes. Other maps error rather than infer a creep's unobserved assigned lane.
Canonical configured lane geometry supplies the projection. Cost: 32 + 8 per lane segment, plus
per received body 4 + 8 per segment + the team's map tower rows. Hidden units never supply a front.
`NumQueuedActions()` counts the active bot's waiting queue, excluding its running action;
foreign/non-active receivers return -1. A publicly dead owned hero has no queued actions.
This queue distinction and sentinel were established from the installed game, build 6975.
Cost: checked receiver plus 8; no scan over the returned queue length.

