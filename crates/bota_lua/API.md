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
