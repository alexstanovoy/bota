# bota

bota is a deterministic simulator of Dota 2, written in Rust: a game server,
the wire protocol, the rules shared by both ends of the wire, and a client
library. It is built for bots and humans to play the same game over the same
protocol.

Current scope: 1v1, the Shadow Fiend and Pudge heroes, the Dota map, and a
mid-only ruleset on that map, plus replays.

- At Dota 7.41's numbers: lane creeps and their waves, every neutral creep the
  game spawns, with its abilities and the conditions the neutral AI casts them
  on, and the camps that draw them (the flooded camps' amphibians evolving with
  the clock), towers, day and night (every unit's day and night vision, neutral camps
  asleep at night), and Shadow Fiend (his abilities, innate, talents and
  Attribute Bonus) wherever the engine has the mechanic; the economy (passive
  gold, hero kill, First Blood and assist gold, the fountain's mending); the
  shop's early items (consumables, Quelling Blade, Magic Stick and Wand, the
  three bands, Power Treads, Phase Boots, Blink Dagger and their parts), the
  Town Portal Scroll, Wind Lace and Blood Grenade. The sources are named next to
  the tables in `bota_core`.
- Pudge at 7.41f: Meat Hook, Rot, Meat Shield and Dismember, and Flesh Heap as
  his innate (two strength for every enemy hero dying within 450); no talents,
  no Aghanim's upgrades. The hook is simulated without its follow-through
  (Pudge is not rooted, disarmed or muted while it flies), its vision, its
  instant kill of creeps and its rune mana refund; the dismember without its
  true sight and vision.
- The shop sells 48 of Dota's items and 5 recipes: consumables, wards, the basic
  components, boots, the early builds and a few later items. An item with a
  limited stock (the wards, Clarity, Healing Salve, Tango, Enchanted Mango,
  Blood Grenade) is stocked and restocked per side as in Dota.

Not modelled, although a 1v1 mid in Dota uses it: runes of every kind, and with
them the Bottle; the items the shop does not sell (Faerie Fire, Smoke of
Deceit, Dust of Appearance, neutral items, ...); Aghanim's Scepter and Shard;
buyback; streams. The
neutral abilities that only a player commanding a neutral would use (Heal,
Vex, Mana Burn, Shockwave, Ice Armor, Seed Shot, Fireball, Icefire Bomb, Take
Off) are shown and never go off, as the neutral AI never casts them.
Orders: a cast or a use ordered unqueued during another's cast point, or
while stunned, breaks the first off or is refused, where Dota (since 7.06)
queues it behind the cast in hand; given queued, it waits as in Dota.

The simulation is authoritative on the server. A client receives what its side
can see and sends orders; it cannot run the game itself.

## Crates

```
bota_proto            wire vocabulary and codec           deps: serde, postcard
bota_core             rules a player may know             deps: bota_proto
bota_server           simulation, network, lobby, CLI     deps: bota_proto, bota_core
bota_client_core      logic every client shares           deps: bota_proto, bota_core
bota_radovid          optional helper, see below          deps: bota_client_core, bota_proto, radovid
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
- `bota_client_core`: the bot loop (`Link::play`, `play`, `Seat`), connection
  and seating (`Link`, `Net`, `join`), and `ClientSession`, the client's
  picture of the match. The `ui` feature adds the float-using, drawing-side
  parts, including the replay player. The graphical
  viewer, dbota, is a separate project built on this crate.
- `bota_radovid`: a helper for writers of clients and bots, to find games through a
  radovid service (matchmaking, lobbies, game-entry tickets). bota itself does not need
  it: nothing else in this workspace depends on it, it is outside the default members,
  and everything in it is behind its `radovid` feature (off by default; `native` or
  `web` imply it). Cargo still reads radovid's manifests to lock the workspace.

## Run the server

```
cargo run --release -p bota_server -- --map 2
```

One lobby, one match, then exit. It listens on `127.0.0.1:4455` and starts once every seat
has joined and is ready. The defaults are the safe ones: loopback only, no cheats, no keyless
spectators, every connection bounded. Plain text never leaves the machine unless you
say so: a `--bind` other machines reach needs a certificate (`--tls-cert`, `--tls-key`, then
the plain TCP port stays on loopback) or `--insecure-plaintext`.

| Flag | Default | What it controls | Change it when |
|---|---|---|---|
| `--port N` | `4455` | plain TCP port (`0`: any free port, announced on stdout) | several servers share a host |
| `--bind ADDR` | `127.0.0.1` | address the listeners bind to | players come from other machines |
| `--ws-port N` | none | WebSocket listener, one binary message per frame (feature `websocket`, on by default; `--no-default-features` drops it) | browsers, or TLS |
| `--tls-cert PEM`, `--tls-key PEM` | none | certificate chain (leaf first) and key; `--ws-port` then speaks TLS 1.3 (`wss://`) | any `--bind` that is not loopback |
| `--insecure-plaintext` | off | plain TCP and `ws://` on a non-loopback `--bind`; keys and orders in clear | a network you trust, without a certificate |
| `--map N` | `0` | `0` the Dota map to the Ancient; `1` a hero demo map; `2` mid only: a side loses on its second hero death or first lost tower, a draw after twenty game minutes | |
| `--mode realtime\|lockstep` | `realtime` | wall-clock ticks, or wait for every seat's acknowledgement | bots that must not miss a tick |
| `--tick-rate N` | `30` | ticks per second | |
| `--ack-timeout-ticks N` | `150` | lockstep: tick-lengths to wait for a straggler | |
| `--players N` | `2`, or the seats in `BOTA_SEATS` | seats | |
| `--heroes ID,...` | all | heroes a seat may pick (and is given without a pick); a pick is final once ready | a ruleset limits heroes |
| `--pick-secs N` | `60` | seconds the seats have to pick once all are held, default 60 (Dota's default hero selection time). Past it a seat without a hero loses 2 starting gold per second until it picks, and after 30 more seconds is given a random allowed hero drawn from the match seed; clients are told the time left (`LobbyState.pick`), also those who reconnect | |
| `--tactical-secs N` | `30` | seconds of tactical phase after every seat has locked in its pick, default 30 (Dota's strategy time); a match of bots alone has none | |
| `--same-heroes-allowed true\|false` | `true` | whether two seats may pick the same hero, default `true` as in Dota's 1v1 Solo Mid and Mid Only; with `false` a hero some seat holds is refused to the others (and never drawn for them) | |
| `--start-after-secs N` | none | with reserved seats: start this long after listening; a seat without a pick gets a random hero | under a launcher |
| `--abandon-unjoined` | off | lobby out of time (`--start-after-secs`, or 60 s) with no seat held: exit 3 | under a launcher |
| `--empty-after-secs N` | none | every seat empty this long during the match: exit 3 | under a launcher |
| `--spectators off\|delayed=SECS\|open` | `off` | who watches without a key: nobody; everyone SECS of game time late; everyone live (shows the whole map) | tournaments (`delayed`), local tools (`open`) |
| `--max-spectators-per-seat N` | `7` | spectators through one seat's key at once | |
| `--max-connections N` | `64` | connections at once; one without `Hello` within 10 s is closed | |
| `--messages-per-sec N` | `1000` | messages per connection per second, plus four per tick sent and a two-second burst; more cuts it off | |
| `--idle-timeout-secs N` | `5` | silence (the server pings every second) before a connection is dropped and its seat waits for a reconnect; `0` never | slow links |
| `--seed N` | drawn, known to no one | match randomness; the same seed and orders replay the same match | reproducible tests only: a published seed predicts the match |
| `--match-id N` | drawn | public label in the start message and replay; no randomness comes from it | under a launcher |
| `--replay FILE` | none | write a replay, one Zstandard stream (refused before `READY` if it cannot be created) | |
| `--chat off\|team\|all` | `all` | what the seats may say: nothing, to their side, to their side and everyone | `off` for bot ladders: chat would be a channel between opponents |
| `--spectator-chat BOOL` | `true` | whether spectators talk among themselves (no seat ever hears them) | |
| `--replay-team-chat BOOL` | `true` | whether the replay keeps the sides' team lines (a replay of the whole map then shows both sides') | `false` when replays of the whole map go to players |
| `--order-delay-ticks`, `--order-jitter-ticks` | `0` | delay every order (one value, or one per seat); jitter from a hidden stream | rehearse a remote server |
| `--cheats` | off | honour cheat orders: gold, levels, items, refreshes | practice |
| `--immortal-structures`, `--immortal-heroes` | off | damage taken, nothing falls | practice |
| `--objectives LIST` | the map's | which of the map's objectives stand: `all`, `none`, or names separated by commas (`roshan`, `tormentor`, `watchers`, `outposts`, `shrines`); one the map does not play is refused before `READY`. Map 2 stands none unless asked | |
| `--runes LIST` | the map's | which kinds of rune spawn: `all`, `none`, or names (`water`, `bounty`, `haste`, `amplify_damage`, `arcane`, `regeneration`, `illusion`, `invisibility`, `shield`); one the map does not spawn is refused before `READY`. Map 2 spawns none unless asked | |
| `--dummy-seat` | off | the last seat is a hero that never acts | practice alone |
| `--wire-fingerprint` | | print the wire fingerprint (16 hex digits: registries and rules) and exit | a launcher's version probe |

Keys never come from arguments, which every process listing shows; only the environment:

| Variable | What it is |
|---|---|
| `BOTA_SEATS` | `key_hex:team:name[:kind],...` (or `key_hex:name`, even slots Radiant; kind `bot` or `human` admits only that kind of player to the seat, others are refused), in slot order, keys 64 hex digits, team `0` Radiant or `1` Dire: only these keys take seats; a key may take its seat again once its old connection is gone (until then `SeatHeld`) |
| `BOTA_REFEREE_KEY` | a referee's key: told the lobby, the start, departures and the end, never a tick |

Over plain TCP and `ws://` keys travel in clear; over `wss://` only inside TLS. The
WebSocket request path and `Origin` are not looked at: the key is what admits. A client frame
is at most `bota_proto::MAX_CLIENT_PAYLOAD_LEN` bytes, refused on its announced length.

### Launcher contract

For a service that starts one server per match:

- stdout: the `bota_server listening on ...` lines, then `READY` once players may connect;
  at the end of a played match one line
  `RESULT winner=0|1|draw duration_ms=N report=HEX [left=SLOT]` (`HEX` is a
  `bota_proto::MatchReport` in postcard) and exit code 0. `left=SLOT` names the seat whose
  player left for good (`LeaveGame`); the other team won. A disconnected seat is waited for.
- Exit code 3, no `RESULT`: nobody played (`--abandon-unjoined`, `--empty-after-secs`, or
  every seat empty with none to come back). Any other exit is a failure; a refused command
  line exits before `READY`.
- Neither stdout nor stderr ever carries a key.

## Write a bot

A bot is a client like any other and sees the same `WorldView` a human does.
What the server promises beyond the types (what it never tells, why it
refuses an order, the order delay and the `seen` tick, tick modes and skipped
ticks, how chances are dealt) is the rustdoc of `bota_proto::contract`:
`cargo doc -p bota_proto --open`, module `contract`.

This one attack-moves at the enemy fountain; it is
`crates/bota_client_core/examples/attack_move.rs`, which also takes the server's address as
its first argument:

```rust
use bota_client_core::{IssuedOrder, Link, PlayError};
use bota_core::fountain_pos;
use bota_proto::{HeroId, Order, Target};

fn main() -> Result<(), PlayError> {
    let outcome = Link::play("127.0.0.1:4455", "attack-move", HeroId::SHADOW_FIEND, |session| {
        let enemy = fountain_pos(session.map(), session.enemy());
        Some(IssuedOrder::hero(Order::Attack { target: Target::Pos(enemy) }))
    })?;
    println!("{outcome:?}");
    Ok(())
}
```

```
cargo run --release -p bota_server -- --map 2 --mode lockstep --dummy-seat
cargo run --release -p bota_client_core --example attack_move
```

Start in lockstep: the server waits for every seat's acknowledgement, so a slow
bot never misses a tick. In realtime the server skips the ticks a client is
too slow for. `--dummy-seat` fills the other seat with a hero that never acts.

`Link::play` connects, picks the hero, and calls the closure once per tick
with the `ClientSession`, the client's picture of the match: the current and
previous view, tracks of units seen, projectiles, recent events, the orders
standing after the server's answers. The closure returns the orders it wants
(an `Option` for one, a `Vec` for several); an order that repeats the one
standing is not sent again, and the tick is acknowledged. While the hero is
dead its orders that need the body are not sent, since the server would
refuse them, and are counted instead; buying and selling still go out, as a
dead Dota hero shops into the stash. `IssuedOrder::courier(id, order)` drives
the courier, dead hero or not. The outcome tells the seat, the winner (`Team::Neutral` for a
draw), the last tick, how many orders the server refused and how many were
held back for a dead hero.

The socket transports answer the server's pings and dial again when the
connection breaks (an error, five seconds of silence, a reset), by a `Redial`
policy (`Link::with_redial`, `Net::connect_with`; `Redial::OFF` makes a break
an error). The server gives the seat back to the key the first `Hello` came
with (the transport draws one when the caller gives none) once the old
connection has ended or been found dead (until then it refuses the newcomer
with `SeatHeld`, and the transport tries again), the session picks
the match up from the next snapshot, and the standing move or attack orders
go out again. A player who leaves is absent, not beaten: the match goes on.

The parts compose for a bot that needs more: `join(transport, name, hero, key)`
takes a seat over any `Transport` (`Link` blocks with timeouts; `Net` reads on
a thread), with the key a server with reserved seats asks for; `play(transport,
seated, observer, decide)` drives it, with an `Observer` hooked into every
snapshot and event batch; and a `Seat` turns the lobby and `MatchStart` into a
session for a loop of your own. `bota_server/tests/client.rs` plays the
example's loop over a real socket to the end of a match.

### In process: `LocalMatch`

`bota_server::local::LocalMatch` runs a lockstep match in your process with no
sockets. `step` takes each seat's orders, advances one tick, and returns
the messages each seat would have been sent, through the same fog and event
filters as the network loop. The program below is an example in
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

## Adding a hero, an ability or an item

Every hero lives in its own module, `crates/bota_core/src/heroes/<hero>/`: its numbers, its body,
its abilities, its talents and the pure functions clients read. The step-by-step list (registry
lines, the module, one row in `HEROES` and `ABILITIES`, the server's hook row and system, the
tests, the front-end labels) is the rustdoc of `bota_core::heroes`:
`cargo doc -p bota_core --open`, module `heroes`. An item is one `ITEMS` row in
`crates/bota_core/src/item_table.rs` (a `UseSpec` variant picks what using it does; new behaviour
adds a variant and its arm in the server's `World::use_effect`), and a unit kind is a `UnitKind`
variant plus its `UNIT_KINDS` row. Ids are append-only (below).

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

There is no version negotiation and no compatibility between revisions. A
client sends `bota_core::WIRE_FINGERPRINT` in its `Hello`: a hash of both
registries (`bota_proto::REGISTRY_DIGEST`) and of the version of the rules a
client checks its orders against (`bota_core::RULES_VERSION`, bumped with every
change to what `bota_core` decides; a test over its code, comments and layout
aside, fails until a change is either versioned or declared to decide nothing
new). A server of another fingerprint answers `ServerMsg::WireMismatch`
with its own and closes the connection, so two builds that disagree, a balance
change alone included, fail at the door rather than in mid-match.

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
  the rule and client crates. Entities are visited by slot; hash maps in the
  server are used for lookup only, keyed by integers.
- No side wins a race inside a tick by being spawned first (slots follow spawn
  order, and a side's units spawn together): the systems that act in turn take
  the slots up or down by a coin drawn every tick, the blows landing in one tick
  are felt in an order drawn afresh, and a target choice tied on everything else
  goes by a rank hashed from the match seed and the two slots.
- A tick's orders are applied seat by seat in slot order, each seat's in the order it gave
  them; a unit takes at most one unqueued order a tick, and its queued orders are taken up
  one after another as Dota's Shift-queue does (the contract's "Orders").
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

A replay (`--replay FILE`) is a stream of framed `ReplayRecord`s, written as one
Zstandard stream (a 20-minute match takes some 4.5 MB instead of 400 MB): a header
naming the wire fingerprint it was recorded with, then the fogless spectator messages,
the orders accepted on each tick and the seats' chat lines. It plays back without a
server; the client reads frames from the file instead of a socket. It contains no seed
and is not re-simulated, but the player refuses one of another fingerprint at its header,
naming both, as Dota's replays go stale with a patch. The player reads a packed replay
with a pure-Rust decoder (in the browser too) and bounds what a file may make it hold;
it also plays the frames unpacked. It is in `bota_client_core` behind the `ui` feature.

Chat is beside the match: lines go to everyone, to a side (and the spectators with that
side's seat keys) or among spectators, never into the simulation or a tick's hash, and a
bot needs none of it. A client may ask for its connection to be compressed
(`ClientMsg::Compress`): everything but chat then comes as parts of one DEFLATE stream.
`bota_proto::contract` has the rules of both.

## Tests and benchmarks

```
cargo test --all
cargo test --all --release
cargo clippy --all-targets -- -D warnings
cargo test -p bota_server --features websocket
cargo clippy -p bota_server --all-targets --features websocket -- -D warnings
cargo fmt --all --check
cargo check --target wasm32-unknown-unknown -p bota_client_core -p bota_proto --features bota_client_core/ui
cargo check --target wasm32-unknown-unknown -p bota_client_core --features bota_client_core/web
cargo bench -p bota_server --bench dummy
```

The `phase-profile` feature of `bota_server` times the regions of a tick for profiling.

The `wasm32` check keeps the client crates building for a browser: the socket transports are
desktop-only there; with the `web` feature `WebSocketLink` is the browser's WebSocket as a
`LiveLink` for a live match. On desktop, `Link` and `Net` dial `host:port` (TCP, the default) or,
with the `websocket` feature, `ws://` and `wss://` URLs (one binary message per frame).

`bota_server/src/game/tests/determinism.rs` pins whole-match fingerprints (world
hash and the encoded view and event streams every 1500 ticks) on every map; a
deliberate rule change re-records them with the ignored `print_pins` test.
The same file replays those matches with drivers that see everything and
pins their world hashes alone: what the sides are shown never changes what
the simulation does (`print_blind_pins` re-records them after a rule change).
`bota_server/tests/client.rs` plays a lockstep match through a real socket, and
`bota_server/tests/bad_links.rs` plays through an in-process proxy that delays,
jitters, caps, stalls, black-holes and resets the line. With
`cargo test -p bota_server --features websocket` those, the reconnect tests and
the hostile-peer tests (`tests/malformed.rs`) also run over `ws://` and `wss://`,
the latter with the self-signed test certificate in `bota_server/tests/tls`.

## License

MIT, see [LICENSE](LICENSE).
