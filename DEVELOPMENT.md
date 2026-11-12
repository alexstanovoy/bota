# Developing bota

## Crates

```
bota_proto        wire vocabulary and codec                deps: serde, postcard
bota_core         rules a player may know                  deps: bota_proto
bota_server       simulation, network, lobby, CLI          deps: bota_proto, bota_core
bota_client_core  logic every client shares                deps: bota_proto, bota_core
bota_replay       the match metadata format                deps: bota_proto, bota_core
bota_radovid      optional matchmaking helper              deps: bota_client_core, bota_proto, radovid
bota_gym          in-process training and evaluation       deps: bota_server, bota_client_core, bota_replay, bota_core, bota_proto
bota_lua          optional bot-script host (outside the workspace)  deps: bota_client_core, bota_core, bota_proto, luagee
```

Dependencies point one way: `bota_proto` <- `bota_core` <- `bota_server` and `bota_client_core`. The server and the
client library don't depend on each other (the server's tests and examples use the client library).

- `bota_proto`: ids, `Fixed` (Q16.16), `Vec2`, `Order`, `EventKind`, `WorldView`, `ClientMsg`, `ServerMsg`,
  `ReplayRecord`, length-prefixed postcard frames. A type is here only if it crosses the wire or is needed to read it.
  The rustdoc of `bota_proto::contract` is the server's promise to clients.
- `bota_core`: hero, ability, item and unit tables, maps, and pure integer functions over them (mitigation,
  regeneration, prices, camps, waves). Hidden quantities enter only as parameters. No world, no randomness, no clock,
  no I/O.
- `bota_server`: the `World` and its systems, the random streams, the lobby, the network loop, the replay writer, and
  `LocalMatch`. The binary is `bota_server`.
- `bota_client_core`: the bot loop (`Link::play`, `play`, `Seat`), connection and seating (`Link`, `Net`, `join`), and
  `ClientSession`, the client's picture of the match. The `ui` feature adds the drawing-side parts and the replay
  player; `web` adds `WebSocketLink` for browsers.
- `bota_radovid`: finds games through a radovid service (matchmaking, lobbies, tickets). Nothing else depends on it;
  it is outside the default members and behind its `radovid` feature. Its end-to-end tests
  (`cargo test -p bota_radovid --features e2e`) need a radovid checkout beside this one.
- `bota_gym`: many in-process matches on a worker pool for training and evaluating bots: `Game` (a lockstep match,
  the hidden state only behind `Game<Privileged>`), the caller's per-game `Episode` run on `Workers`, `VecEnv` and
  `Tournament` drivers, seeding, opponent scheduling, statistics. Outside the default members; its crate docs are
  the guide (`cargo doc -p bota_gym --open`).
- `bota_lua`: the host for Lua bots (binary `bota_botscript`). It builds on `luagee`, a repository of its own, and is a
  workspace of its own (`crates/bota_lua/Cargo.toml`, with its own `Cargo.lock` and `target/`), not a member of this
  one: Cargo loads the manifest of every path dependency, an optional one included, so a member would stop every build
  of bota whenever the luagee checkout is absent. Nothing in the bota workspace depends on it. Build and test it with
  the commands in [its README](crates/bota_lua/README.md).

## Running a server

`bota_server --help` is the reference for every option. It serves one lobby and one match, then exits. The defaults
are the safe ones: loopback only, no cheats, no keyless spectators, every connection bounded. A `--bind` that other
machines reach needs a certificate (`--tls-cert`, `--tls-key`; the WebSocket port then speaks TLS 1.3 and plain TCP
stays on loopback) or an explicit `--insecure-plaintext`.

Keys never come from arguments, which every process listing shows, only from the environment: `BOTA_SEATS` reserves
the seats (`key_hex:team:name[:kind],...`) and `BOTA_REFEREE_KEY` admits a referee. The key is what admits a client;
the WebSocket path and `Origin` are not checked.

Connections that have not said Hello yet are bounded per source, so that one host cannot keep the players out:

- TLS and WebSocket handshakes under way share one pool of 64 for every peer; a peer that finds it full is let go at
  once, so the kernel's backlog never fills. A source holds 16 of them at most per IPv4 address, 16 per IPv6 /64 and
  64 per IPv6 /48.
- Connections without a Hello have the same bounds per source, and take a quarter of `--max-connections` at most. A
  newcomer past that (or past `--max-connections`) takes the place of another source's stranger: a lingering one
  first, then the oldest of the source holding the most; one that could only take its own source's is refused. A peer
  let go while a stranger (refused at its Hello, say) is read on for a while and counts meanwhile as a stranger of its
  source.
- Watchers without a seat's key are never let go for room, so they are bounded too: 4 per IPv4 address, IPv6 /64 and
  IPv6 /48, and a quarter of `--max-connections` in all; one past either is refused as `NotAdmitted`.
- Players that said Hello have no bound per source and are never let go for room: players behind one address all play.

Behind a reverse proxy every peer counts as the proxy's address: `X-Forwarded-For`, which any peer could forge, is not
read.

### Launcher contract

For a service that starts one server per match:

- stdout carries the `bota_server listening on ...` lines, then `READY` once players may connect, and at the end of a
  played match one line `RESULT winner=0|1|draw duration_ms=N report=HEX [left=SLOT] [replay=truncated]` with exit
  code 0. `HEX` is a postcard `bota_proto::MatchReport`; `left=SLOT` names a seat whose player left for good (the
  other team won); `replay=truncated` says the replay was closed early by its size cap. Fields after `report` are
  `key=value`, and readers skip keys they don't know. Version-2 fields are printed only to a launcher that asks
  (`--result-version 2` or `RADOVID_RESULT_VERSION`), so the two can be updated in any order.
- Exit code 3 without `RESULT`: nobody played (`--abandon-unjoined`, `--empty-after-secs`, or every seat empty with
  none to come back). Any other exit is a failure; a refused command line exits before `READY`.
- Neither stdout nor stderr ever carries a key.

## The client library

`Link::play` hands your closure the `ClientSession` every tick: the current and previous view, tracks of units seen,
projectiles, recent events, and the orders standing after the server's answers. It returns an `Option` or a `Vec` of
orders; an order that repeats the standing one is not sent again. While the hero is dead, orders that need its body
are held back and counted (shopping still goes out, into the stash); `IssuedOrder::courier` drives the courier. The
outcome reports the seat, the winner (`Team::Neutral` for a draw), the last tick and the refused and held-back orders.

Start bots in lockstep (`--mode lockstep`): in realtime the server skips ticks a slow client misses. The socket
transports answer pings and redial on a broken connection (`Redial`); the server gives the seat back to the same key
once the old connection is gone, and standing move or attack orders are sent again. A player who leaves is absent,
not beaten.

For more control, the parts compose: `join(transport, name, hero, key)` takes a seat over any `Transport`;
`play(transport, seated, observer, decide)` drives it with an `Observer` on every snapshot and event batch; `Seat`
turns the lobby and `MatchStart` into a session for a loop of your own.

`bota_server::local::LocalMatch` runs a lockstep match inside your process with no sockets: `step` takes each seat's
orders, advances one tick and returns what each seat would have been sent, through the same fog and event filters as
the network. `crates/bota_server/examples/local_match.rs` plays two scripted heroes to the end
(`-- --replay match.brp` records it).

On `wasm32` the socket transports are absent; `WebSocketLink` (feature `web`) is the browser's WebSocket. On desktop
`Link` and `Net` dial `host:port`, or `ws://` and `wss://` with the `websocket` feature.

## Adding a hero, an ability or an item

Every hero lives in `crates/bota_core/src/heroes/<hero>/`: numbers, body, abilities, talents and the pure functions
clients read. The checklists are rustdoc, next to the code they describe: `bota_core::heroes` for a hero or ability
(registry lines, the module, the `HEROES` and `ABILITIES` rows, the server's hook and system, tests), `bota_core::ITEMS`
for an item (registry line, table row, catalogue agreement, new stats, actives or buffs, and end-to-end tests). A unit kind is a
`UnitKind` variant plus its `UNIT_KINDS` row. Registry lines name ids by Dota's internal names (`item_blink`,
`npc_dota_hero_nevermore`), which each table row carries as its `key`.

## Protocol: append-only

Ids and wire positions are append-only, like protobuf field numbers, because postcard encodes variants and fields by
position. This covers hero, ability, item and effect ids, the variant index of every wire enum and the field order of
every wire struct.

- A new id takes the next number; a new variant or field goes last.
- A removed id is retired for good, never reused; tables indexed by id keep a placeholder.
- `crates/bota_proto/registry/ids.txt` (every id, its Dota key, live or retired) and `wire.txt` (every variant index
  and field position) only grow. Tests fail on a position that changed, disappeared or isn't registered yet, and on a
  table that disagrees with `ids.txt`.

There is no version negotiation. A client sends `bota_core::WIRE_FINGERPRINT` in its `Hello`: a hash of both
registries and of `bota_core::RULES_IDENTITY`, a content-derived `u64` computed by `bota_core/build.rs`. A server of another fingerprint answers
`WireMismatch` and closes, so two builds that disagree, a balance change included, fail at the door instead of in
mid-match.

`RULES_IDENTITY` is an identity, never an ordered counter. No author bumps it, checks in generated values, or
renumbers it after merging another rule change. The build follows production module declarations from
`bota_core/src/lib.rs`, `bota_server/src/game/mod.rs`, `bota_server/src/engine/mod.rs`, and the simulation helpers
exported by `bota_proto/src/lib.rs`. It excludes the core's item-data, compatibility and text-only modules;
server transport, lobby, chat, CLI, profiling and spectator plumbing; and the protocol's message, framing,
credential and codec modules. New protocol root modules must be classified explicitly as simulation helpers or
wire plumbing. Decision helpers in the server belong under `game` or `engine`.

The graph includes order validation, per-side visibility/event mapping, map/configuration decisions, and literal
`include!`, `include_str!` and `include_bytes!` inputs, including qualified standard-library macros. Module bindings
bring newly declared files into the graph; unreferenced files and `cfg(test)` modules are ignored. Production
`#[path]` modules are refused explicitly, including conditional path attributes that could be active in production.
Rust includes may contain inline declarations, but external modules belong in the normal module tree.

Canonicalization uses exact `syn`, `proc-macro2` and `prettyplease` versions. It removes comments, documentation,
test-only declarations and layout; normalizes optional trailing commas and single-expression match-arm braces;
and sorts imports. Tuple commas, literal contents, macro punctuation and matchers remain significant. Opaque
macro fragments are retained conservatively instead of guessing where an item ends. Other production refactors
can change the identity: this is a source contract, not a proof of semantic equivalence. If parsing fails, the build
warns and hashes CRLF-normalized raw source under a separate tag, leaving rustc to report the actual source error.
The real-tree source test requires zero fallback inputs, so new syntax must be supported by the pinned parser
before it can pass the gate.

The whole private `bota_core/src/data/` directory belongs to `Catalogue` prefix agreement. Its Rust sources are
parsed and watched as validation inputs, without entering the rules hash. Its `mod.rs` must re-export only
`ITEM_ROWS`. Impl blocks, extern code, macro declarations/exports and non-builtin macros are refused, including
nested declarations; use ordinary private constant/function factories. Adding a data helper and an item row
preserves the identity and older catalogue prefixes. Each row also binds its associated `ItemId` names and literal numeric values,
found at every AST depth: changing an existing binding cannot bypass agreement, and a constant for an appended id
leaves older prefixes unchanged. Keep these declarations as literal `ItemId(u16)` values. Match metadata keeps the
frozen `rules_version: u32` header field, zero for content identities, and appends `MetaRecord::RulesIdentity { identity }`.

Hash inputs have sorted repository-relative UTF-8 paths, framed lengths, explicit little-endian encoding and a
versioned SHA-256 domain. Machine paths, Git history, timestamps, compiler debug output and target settings do not
enter it. The first eight digest bytes form the identity; the existing 64-bit wire has the same finite
hash-collision limitation and neither is authentication. Compiler/dependency behavior is checked by determinism tests.

The build script reads bounded, regular, repository-contained inputs, follows no symlinks, performs no network
requests and launches no processes. Dynamic includes are refused. It writes only in Cargo's output directory and
only if output changes. Cargo watches each traced file individually, the private data validation files, and the build
helper itself. Editing a test file under `bota_server/src/**/tests` does not rerun the identity builder. Changes to
an inline test in a traced production file can rerun it while preserving the identity. There are no directory
watches: adding a production module changes its watched parent declaration; deleting a traced file invalidates its
watch. A gate-only fence also keeps new manual view impls (such as `ClockShift` behavior) out of the wire view
module; its existing client read helpers stay wire-only, with no extra build watches. A complete repository
checkout is required, including the server rule sources even for client builds.
Packaging individual crates for crates.io is not supported by this build arrangement; publication policy is separate.

## Determinism

The same seed and orders give the same match, bit for bit, on any platform:

- No floating point in the simulation or the protocol (`clippy::float_arithmetic` is denied). Scalars are `Fixed`
  (Q16.16 in an `i32`, products through `i64`), angles are `u16` brads, distances compare as squares. The one float,
  inside `isqrt64`, is corrected by integer steps to the exact root.
- No wall clock: time is ticks, 30 a second.
- No iteration over hash maps; `HashMap`/`HashSet` are denied in the rule and client crates. Entities are visited by
  slot.
- No side wins a race by spawn order: systems that act in turn go up or down the slots by a coin drawn every tick,
  blows landing in one tick are felt in a freshly drawn order, and ties go by a rank hashed from the seed and slots.
- Orders apply seat by seat in slot order, each seat's in the order given; a unit takes at most one unqueued order a
  tick, and queued orders follow as Dota's Shift-queue does.
- Only value-stable primitives: `ChaCha8Rng` (never `StdRng`), FNV-1a (never `DefaultHasher`). Randomness is split
  into streams by purpose, so a new draw in one place doesn't shift others. Evasion and pierce use Dota's
  pseudo-random distribution, one stream per source.
- `World::hash()` covers the whole state, hidden state included: `world_tables!` declares every field of the world
  once, and a test changes each hashed part in turn and checks that the hash moves.

A match is reproducible from `(master_key, stream)` in `MatchConfig` or from `--seed`. The public `match_id` comes from
another stream of the key, so it gives neither the seed nor the key away. Hidden state is never put on the wire: the
types in `bota_proto` cannot express it.

## Replays, metadata and chat

A replay (`--replay FILE`) is one Zstandard stream of framed `ReplayRecord`s (some 4.5 MB for 20 minutes): a header
with the wire fingerprint, then the fogless spectator messages, the accepted orders and the chat. It plays without a
server and is not re-simulated; a player refuses a replay of another fingerprint, as Dota's replays go stale with a
patch. The player (`bota_client_core`, feature `ui`) decodes in pure Rust, in the browser too, and bounds what a file
can make it hold.

The metadata file (`--meta FILE`), like Dota's `.meta`, is a small log of the match (what each seat held and earned
every minute, kills, purchases, levels, buildings, final items) in the `bota_replay` format, outside the wire
fingerprint. Its records are append-only (`crates/bota_replay/registry/meta.txt`), and readers skip what they don't
know, so every later build reads every earlier file. `bota_client_core::meta_of_replay` rebuilds the same bytes from a
replay. `--meta-trailer` appends it to the replay as a Zstandard skippable frame; it stays off by default because
older players refuse bytes after the stream.

Chat never enters the simulation or a tick's hash. A client may ask for its connection to be compressed
(`ClientMsg::Compress`, one DEFLATE stream). `bota_proto::contract` has the rules of both.

## Tests

`tools/precheck.sh` is the quick loop: clippy and the tests the change since the merge-base with `origin/main` can
affect. It stamps a green tree. `tools/gate.sh` is the merge gate; its header lists the steps: clippy, the wasm checks,
the tests in the `test-release` profile, the bota_radovid end-to-end tests, the bench digest checks, `cargo deny check`,
the item importer (`tools/import_items`, which checks the committed item table against a Dota 2 install) and fmt. The
bot-script host `bota_lua` (clippy, the wasm check, its fuzz target, `cargo deny`, fmt and the `lua` test group) is
covered where the luagee checkout is. Where it is not, the precheck and the gate say it is not covered, and refuse a change
to its files (a change that cannot be checked must not look green); `TEST_GROUPS_REQUIRE_ALL=1` makes a missing
checkout a failure even for other changes, for machines that must cover it. It
stops at the first failure, prints `GATE-OK`, and fails when a step can't run. Both run what the change can affect;
`GATE_FULL=1` (`PRECHECK_ALL=1`) runs everything, for releases and after a batch of changes.

Every test is in exactly one of the groups of `tools/test_groups.toml`, which follow the crates and the game's
subsystems, and every file says which groups a change to it runs. `tools/test_groups.py check` proves both from
libtest's own test lists (a change checks the crates it can touch; `--all` checks every crate and build),
`plan` turns a change into groups and `run GROUP...` runs them, test binaries side by side while their test threads fit
the CPUs the run may use (`TEST_GROUPS_CPUS` sets that share).

What the main suites cover:

- `bota_server/src/game/tests/determinism.rs` pins whole-match fingerprints (world hash and encoded view and event
  streams every 1500 ticks) on every map, and replays them with all-seeing drivers to prove that what a side is shown
  never changes the simulation. A deliberate rule change re-records them with the ignored `print_pins` and
  `print_blind_pins` tests; `cargo bench -p bota_server --bench dummy -- --test` checks the bench digests.
  Rule identities never enter these state/output digests. Keep the golden expectations: a changed source hash
  must not automatically approve changed output. A merge needs new pins only when its actual output changes;
  hashes of two changed matches cannot predict the output of their combined rules.
- `bota_server/tests/client.rs` plays a lockstep match over a real socket; `tests/bad_links.rs` through a proxy that
  delays, jitters, stalls and resets the line; `tests/malformed.rs` against hostile peers. With
  `--features websocket` they also run over `ws://` and `wss://` (test certificate in `bota_server/tests/tls`).

The `phase-profile` feature of `bota_server` times the regions of a tick.

## Matchmaking integration

The game-specific rulesets and local launch guide live beside `bota_radovid`:
[matchmaking.md](crates/bota_radovid/matchmaking.md) and
[radovid.example.toml](crates/bota_radovid/radovid.example.toml). Protocol 14 requires rebuilt control clients;
this is independent of the game's own wire fingerprint.
