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
`GetHeroLastSeenInfo` returns an array with the last received `location` and
`time_since_seen`, or an empty array for a hero never observed. It uses the
session's remembered body even after short-lived entity tracking expires.

`IsPlayerBot` is a host-declared fact. Configured scripted seats always answer
true. The trusted launcher may additionally set `NativeConfig.declared_bot_slots`
or call `LuaTeam::set_bot_declarations` before execution. These declarations use
wire seat ids, contain at most ten distinct entries in 0–9, and default to empty.
Every other seat answers false, conservatively treating an undeclared controller
as human. This does not disclose controller information from the wire.

New bounded integer arguments use `api/globals::bounded_integer`: finite whole
numbers must fall within the specified bounds before conversion. The older
general numeric helper retains its existing callers' conversion behavior.

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
