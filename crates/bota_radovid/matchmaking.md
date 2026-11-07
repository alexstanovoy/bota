# Run bota through matchmaking

`bota_radovid` owns the game-specific integration with the independent radovid service. radovid starts a configured
server, hands players opaque tickets and keeps the ladder; this crate understands how a bota client takes its seat.
The dependency direction is bota → radovid client/protocol. The service never imports bota's rules or wire types.

## Local quick start

Build the matching revisions of the service and the game server, then create a development certificate:

```sh
cargo build --release -p bota_server
cargo build --release --manifest-path ../radovid/Cargo.toml -p radovid
mkdir -p dev
../radovid/target/release/radovid dev-cert dev/
target/release/bota_server --wire-fingerprint
```

Run from the bota repository root. Write `dev/radovid.toml`, replacing the fingerprint below with the printed digits
and the binary/artifact paths with absolute local paths:

```toml
listen = "127.0.0.1:4460"
database = "dev/radovid.db"
tls_cert = "dev/cert.pem"
tls_key = "dev/key.pem"
game_ports = "20000-20015"
lobby_teams = [5, 5]
lobby_ruleset = "solo_mid"
[[ruleset]]
id = "solo_mid"
name = "1x1 Mid Only"
game = "bota"
game_fingerprint = "<16 printed hex digits>"
teams = [1, 1]
rated = true
max_duration_secs = 1800
[ruleset.launcher]
kind = "local"
binary = "/absolute/path/to/bota_server"
public_url = "ws://127.0.0.1:{port}"
version_probe = ["--wire-fingerprint"]
args = ["--map", "2", "--match-id", "{match_id}", "--bind", "127.0.0.1", "--port", "0",
        "--ws-port", "{port}", "--heroes", "1", "--start-after-secs", "60",
        "--abandon-unjoined", "--empty-after-secs", "120"]
env = { BOTA_SEATS = "{seats}" }
```

Start the service with `../radovid/target/release/radovid serve --config dev/radovid.toml`. Game clients connect to
`wss://localhost:4460`, trust `dev/cert.pem`, and use this crate to queue, accept and connect directly to the game.

[radovid.example.toml](radovid.example.toml) carries the full, solo-mid and demo configurations, including server
arguments, replay/metadata files and the demo `info` byte consumed by `bota_radovid::is_demo`. Its container images
and certificates are operator-supplied examples: use a server whose wire fingerprint matches the configured value.
`lobby_teams = [5,5]` is explicit because the neutral service default is `[1,1]`; a mismatched layout fails loading.

## Queue audiences

Every rated ruleset has humans and everyone queues. A bot uses `Client::queue` (everyone).
A human client uses `queue_with_audiences(&ruleset, &[Audience::HumansOnly])`; when the player
allows bots, pass `[Audience::HumansOnly, Audience::Everyone]`. This is one search holding two
queue places, admitted and cancelled together. `Searches::entries()` exposes the raw places;
`Searches::groups()` gives one display row per search. A proposal reserves both places; accepting
consumes them, and a failed proposal restores both original places. Other searches remain independent.

## Protocol 15 coordination

Rebuild every matchmaking client with the service's version-15 protocol/client crates. Versions below 15 are
refused at `Hello`; queue records gained appended audience/search fields and requests. Read
`AccountGameLimit::Limited(n)` or `Unlimited`, rather than interpreting a numeric sentinel. Each account still
holds one ladder rating per ruleset, shared across simultaneous matches.

The service and rebuilt matchmaking clients need the same protocol boundary when they resume exchanging control
messages. Keep game-server/client wire fingerprints matched separately: the control-protocol change does not alter
bota's game wire or replay format. Existing source configurations keep working when their lobby layout is explicit.
The service's [coordination note](../../../radovid/docs/protocol_15.md) describes the generic upgrade contract.

## Integration tests

Build the release game server, then run `cargo test --profile test-release -p bota_radovid --features e2e` with
`BOTA_SERVER_BIN` pointing to it. These tests use a real service, actual launched servers and game clients, including
ratings, reconnects and replay retrieval. The shipped configuration is loaded by the same integration suite.

### Ranked and unranked modes

The example offers a rated and an unrated queue for both full games and solo mid.
An unrated ruleset names its rated sibling in `rating_ruleset`; they share one ladder
for matching, and only the rated result changes it. Lobbies remain unrated with either
ruleset. Demo has no queue.

Select the desired ruleset ID with the same queue API. `RulesetInfo.rated` identifies
ranked play; `rating_window` identifies queue availability, and `rating_ruleset` links
an unrated variant to its mode. The handoff and result retain the selected ID and
`rated = false` for unranked play, with no result rating deltas.

`[limits.ranked_window]` defaults to `initial = 100`, `widen_per_minute = 100`,
`maximum = 1000`; unranked defaults are `600`, `300`, `3000`. A per-ruleset
`[ruleset.rating_window]` overrides individual values. Unranked must be at least as
loose as its rated sibling, and every player must accept the group's full rating spread.
The service serves these policies in the catalog and `Limits`; querying either sibling's
limits returns both resolved mode policies. Rebuild clients with protocol 16 together.
