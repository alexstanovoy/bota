# bota

A simplified Dota 2 in Rust for AI bots and humans. Architecture — `DESIGN.md`.

## Code conventions

### Documentation

A docstring answers "what is this", "what does it mean" and "when is it used". It never
answers "why was it decided this way".

- Rationale, alternatives, comparisons with rejected options and decision history live
  in `DESIGN.md`. Not in the code.
- One line is enough for a field. State units, range, invariants and what a zero or
  absent value means.
- Never write `deliberately`, `because`, "unlike option X", "this used to be".
- Do not record assumptions about how the other side works. If a fact has not been
  verified — it does not exist.
- A docstring that has grown large is almost always a sign that rationale ended up in
  the wrong place.
- English, ASCII only, rustdoc markup.

### lib.rs and mod.rs

`lib.rs` and `mod.rs` may contain **only** `mod` and `use`. No type definitions,
functions, constants, traits or `impl` — all of that lives in submodules.

Re-exports are predominantly wildcard:

```rust
// lib.rs
mod codec;
mod ids;
mod math;

pub use codec::*;
pub use ids::*;
pub use math::*;
```

The consequence to keep in mind: names must be unique across the crate, otherwise the
glob re-exports conflict. The conflict surfaces not at the declaration site but at the
use site, so type names are not picked carelessly.

### Tests

Tests are wired in only by declaring the module in `lib.rs` or `mod.rs`:

```rust
#[cfg(test)]
mod tests;
```

Careful: `test`, not `tests`. `#[cfg(tests)]` is a condition on a feature that does not
exist — the module never gets compiled, the tests silently vanish, and `cargo test`
reports "0 passed" without a single error.

- Few tests — a `tests.rs` file next to the module.
- Many — a `tests/` directory with a `mod.rs` and groups split by file. That `mod.rs`
  falls under the general rule: only `mod` and `use`.
- No integration-test directory at the crate root.

Fixtures build structs by listing **all** fields, without `..Default::default()`.
This is the guard on type composition: a new field in `UnitView` breaks test
compilation and demands a conscious decision about what to put in it.

### Crate boundaries

Four crates: `bota-proto` (shared vocabulary + codec), `bota-server` (simulation +
networking), `bota-client`, and `bota-bot`. All depend only on `proto`, and in particular
no bot depends on another: a second bot that leaned on the first would mean every future
one carrying every past one about with it. The wire and the match loop are short enough
to be worth writing twice.

The membership criterion for `bota-proto` is the single rule deciding where code lives:

> If it does not cross the wire and is not needed to read the wire — it is not in `proto`.

That is why `World`, `MatchRng`, the balance constants and the ability
implementations live in `bota-server/src/game/` and are unreachable from the client in
principle.

`bota-server` is split in two. `engine/` is how state is kept and walked — entity
handles, component tables, the fingerprint hasher — and knows nothing of Dota.
`game/` is the game: `world.rs` and the tick order, `systems/`, `components/`, and
`config/` for everything the game is made of — the map, the terrain, the forest, the
units, the items, the camps and the balance constants. `game` names what it takes from
`engine` in one place, `game/mod.rs`.

### bota-proto and bota-server

- No `f32`/`f64`: `#![deny(clippy::float_arithmetic)]`. Reason: `sin/cos/sqrt` from
  libm differ across platforms and break determinism. Float is allowed in `bota-client`
  (rendering) and in `bota-bot` (what gets recorded are the bot's orders, not its
  reasoning).
- No `HashMap`/`HashSet` in the simulation: iteration order is not guaranteed.
- No `std::time` in `game/`: time exists only as ticks.
- `game/` knows nothing about sockets or `PlayerId` — only `SlotId`.

### Dependencies

New external dependencies only after discussion. Allowed:

| Crate | Where | Purpose |
|---|---|---|
| `serde` (derive) | `bota-proto` | message serialization |
| `postcard` 1.1 | `bota-proto` | compact binary format |
| `macroquad` | `bota-client` | rendering |
| `resvg` | `bota-client` | rasterising the item art |
| `rand_chacha` 0.10 | `bota-server` | PRNG |
| `clap` (derive) | every binary | command line arguments |

Every binary parses its arguments with `clap` and its derive. There is no bar low enough
for a hand-rolled parser to be worth clearing: it costs a hundred lines, generates no
`--help`, validates nothing, and quietly lets a flag apply to the wrong subcommand.

The requirement for any primitive that affects the simulation: **value-stability**,
i.e. a guarantee of identical values across versions and platforms. Hence
`rand_chacha::ChaCha8Rng` rather than `rand::StdRng` (the latter is allowed to change
its algorithm in a minor release). For the same reason
`std::collections::hash_map::DefaultHasher` is not fit for `world.hash()` — that is
ten lines of FNV-1a.

### Versioning

Until the first release there is no versioning and no compatibility. The wire carries
no version field and no ruleset fingerprint; no protocol, replay or hash compatibility
is promised between pre-release commits. Mismatched builds are not detected — they are
simply not run against each other.

## Pre-commit checks

```
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo test --all --release
```

Release is run separately not for speed: `debug_assert!` is off there, and overflow
behavior switches from panicking to saturating. A test that covers only one half fails
in the other mode — and it fails exactly where we would notice it last.
