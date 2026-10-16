# bota

bota is a deterministic simulator of Dota 2 at patch 7.41f, written in Rust: a
game server, the wire protocol, the rules shared by both ends of the wire, and a
client library. It is built for bots and humans to play the same game over the
same protocol.

Current scope: 1v1, the Shadow Fiend and Pudge heroes, the Dota map, and a
mid-only ruleset on that map, plus replays. Everything a 1v1 mid lane can touch
(creeps, neutrals, runes, the shop, towers, couriers, talents) follows the
7.41f numbers; what 1v1 mid never uses is not modelled.

The simulation is authoritative on the server. A client receives what its side
can see and sends orders; it cannot run the game itself.

## Crates

```
bota_proto            wire vocabulary and codec           deps: serde, postcard
bota_core             rules a player may know             deps: bota_proto
bota_server           simulation, network, lobby, CLI     deps: bota_proto, bota_core
bota_client_core      logic every client shares           deps: bota_proto, bota_core
```

Dependencies point one way: `bota_proto` <- `bota_core` <- `bota_server` and
`bota_client_core`. The server and the client library do not depend on each
other (the server's tests and examples use the client library).

- `bota_proto`: ids, `Fixed` (Q16.16 integers), `Vec2`, `Order`, `EventKind`,
  `WorldView`, `ClientMsg`, `ServerMsg`, `ReplayRecord`, length-prefixed frames
  encoded with postcard. A type is here only if it crosses the wire or is needed
  to read the wire.
- `bota_core`: hero, ability, item and unit tables, map definitions, and pure
  integer functions over them (damage mitigation, regeneration, prices, camp and
  wave composition). Hidden quantities enter these functions only as parameters.
  No world, no random stream, no clock, no I/O.
- `bota_server`: the `World`, its systems (movement, combat, vision, economy,
  creeps, items, abilities), the random streams, the lobby, the TCP loop, the
  replay writer, and `LocalMatch`, a lockstep match run inside the caller's
  process. The binary is `bota_server`.
- `bota_client_core`: connection and seating (`Link`, `Net`, `join`), and
  `ClientSession`, the client's picture of the match. The `ui` feature adds the
  float-using, drawing-side parts, including the replay player. The graphical
  viewer, dbota, is a separate project built on this crate.

## Build and run the server

```
cargo build --release
cargo run --release -p bota_server -- --map 2 --mode lockstep --seed 1 --replay match.brp
```

The server hosts one lobby and one match, then exits. It listens on `--port`
(default 4455) and starts when every seat has joined and is ready.

| Flag | Meaning |
|---|---|
| `--port N` | TCP port, default 4455 |
| `--map N` | `0` the Dota map played to the Ancient (default); `1` a small hero demo map; `2` the mid-only ruleset on the Dota map: a side loses on its second hero death or its first lost tower, and the match is a draw after fifteen game minutes |
| `--mode realtime\|lockstep` | `realtime` runs at `--tick-rate` on the wall clock (default); `lockstep` waits for every seat to acknowledge each tick |
| `--tick-rate N` | ticks per second, default 30 |
| `--players N` | seats, default 2 |
| `--seed N` | seed of the match randomness, for a match that must play again the same; without it the key is drawn from the operating system and known to no one |
| `--match-id N` | the public label of the match in its start message and replay; drawn when absent, and nothing random comes from it |
| `--seat KEY_HEX:NAME,...` | reserve the seats, in slot order, for the holders of 64-hex-digit keys (also `BOTA_SEATS`, which keeps the keys out of process lists); a key taken again replaces its holder, also mid-match |
| `--referee-key KEY_HEX` | admit a referee with this key (also `BOTA_REFEREE_KEY`): told the lobby, the start, departures and the end, never a tick |
| `--start-after-secs N` | with reserved seats, start this long after listening at the latest; a seat without a pick gets a random hero |
| `--max-connections N` | most connections held at once |
| `--wire-fingerprint` | print the fingerprint of the wire registries this build speaks, and exit |
| `--replay FILE` | write a replay |
| `--ack-timeout-ticks N` | lockstep: tick-lengths to wait for a straggler, default 150 |
| `--cheats` | honour cheat orders (gold, levels, items, refreshes) |
| `--immortal-structures`, `--immortal-heroes` | structures or heroes take damage but never fall, for practice |
| `--dummy-seat` | the last seat holds a hero that never acts, so one client can practise alone |
| `--order-delay-ticks`, `--order-jitter-ticks` | delay every order, one value or one per seat; jitter is drawn from a hidden stream of the seed |

## Write a bot

A bot is a client like any other and sees the same `WorldView` a human does.

1. Connect and take a seat: `Link::join(address, name, Some(HeroId::SHADOW_FIEND))`
   returns the connection and a `Seated`. `Link` blocks; `Net` runs the socket
   on threads and never blocks the caller. Both implement the same framing over
   the `Transport` trait.
2. When `ServerMsg::MatchStart { info }` arrives, build a
   `ClientSession::start(&seated, &info, options)`.
3. Feed every server message to `session.ingest(message, &mut ())`. It folds
   snapshots and events into the session and sends nothing. The session keeps
   the current and previous view, tracks of units seen, projectiles, recent
   events, standing orders and refusals.
4. Decide an order and call `session.issue(IssuedOrder { unit, order })`. It
   numbers the order and returns an `OutgoingOrder`; `outgoing.message()` is the
   `ClientMsg` to send. `unit: None` drives the hero, `Some(id)` the courier.
5. In lockstep, call `session.end_turn()` after deciding; it returns the tick to
   acknowledge once (`ClientMsg::Ack { tick }`).

`tests/client.rs` in `bota_server` does this over a real socket.

### In process: `LocalMatch`

`bota_server::local::LocalMatch` runs a lockstep match in your process with no
sockets. `step` takes at most one order per seat, advances one tick, and returns
the messages each seat would have been sent, through the same fog and event
filters as the network loop. The complete program below is
`crates/bota_server/examples/local_match.rs`; it plays two scripted heroes to
the end of a match:

```
cargo run --release -p bota_server --example local_match
```

```rust
let (mut local, opening) = LocalMatch::new(&cfg, &[OrderDelay::NONE])?;
// ...a ClientSession per seat, built from each stream's MatchStart...
let streams = local.step(&[Some(order_for_seat_0), Some(order_for_seat_1)])?;
for (session, stream) in sessions.iter_mut().zip(streams) {
    for message in stream {
        session.ingest(message, &mut ())?;
    }
}
```

## Protocol rule: append-only

Ids and wire positions are append-only, like protobuf field numbers. This covers
hero, ability, item and effect ids, the variant index of every wire enum
(`UnitKind`, `Order`, `EventKind`, `RejectReason`, ...), and the field order of
every wire struct, because postcard encodes variants and fields by position.

- A new id takes the next number of its space; a new variant or field goes last.
- Removing something retires its id for good: it is never renumbered or reused,
  and tables indexed by id keep a placeholder.
- Two committed registries in `crates/bota_proto/registry/` record what has been
  assigned: `ids.txt` (every id, with name and `live` or `retired`) and
  `wire.txt` (every variant index and field position). They only grow. Tests
  fail on a wire position that changed or disappeared, on one not yet registered,
  and on a hero, ability, item or effect table that disagrees with `ids.txt`.

There is no version negotiation and no compatibility between revisions.

## Determinism and replays

The same seed and the same orders give the same match, bit for bit, on any
platform. The rules that keep it so:

- No floating point in the simulation or the protocol (`clippy::float_arithmetic`
  is denied workspace-wide). Scalars are `Fixed`, Q16.16 in an `i32` with
  multiplication through `i64`; angles are `u16` "brads"; there is no
  trigonometry; distances are compared as squares, with an integer square root
  where a length is needed. Damage, gold and experience are integers.
- No wall clock: time is ticks (30 per second).
- No iteration over hash maps; `HashMap` and `HashSet` are denied by clippy in
  the rule and client crates. Entities are visited in ascending slot order. Hash
  maps in the server are used for lookup only, keyed by integers.
- At most one pending order per seat; a tick's commands are applied in slot order.
- Only value-stable external primitives: randomness is `ChaCha8Rng`
  (never `StdRng`), and the state hash is FNV-1a, not `DefaultHasher`.
- Randomness is split into streams by purpose, so a new draw in one place does
  not shift the draws elsewhere. Evasion and pierce use exact per-block counts
  with a hidden order inside each block.
- `World::hash()` covers the whole state, hidden state included, so a divergence
  in randomness consumption shows on the tick it happens.

A match is reproducible from `(master_key, stream)` in `MatchConfig`, or from
`--seed` on the command line. The key comes out of the seed through ChaCha8, and
the public `match_id` is a block of another of the key's streams, so neither the
label nor the play gives the seed or the key away. Hidden state (the key,
random streams, pending random outcomes) is never put on the wire; the types in
`bota_proto` cannot express it.

A replay (`--replay FILE`) is a stream of framed `ReplayRecord`s: the fogless
spectator messages plus the orders accepted on each tick. It plays back without
a server; the client reads frames from the file instead of a socket. It contains
no seed and is not re-simulated, so it survives later rule changes. The player is
in `bota_client_core` behind the `ui` feature.

## Tests and benchmarks

```
cargo test --all
cargo test --all --release
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
cargo bench -p bota_server --bench dummy
```

`bota_server/src/game/tests/determinism.rs` pins whole-match fingerprints (world
hash and the encoded view and event streams every 1500 ticks) on every map; a
deliberate rule change re-records them with the ignored `print_pins` test.
`bota_server/tests/client.rs` plays a lockstep match through a real socket.

## License

MIT, see [LICENSE](LICENSE).
