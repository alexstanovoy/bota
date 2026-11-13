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
max_duration_secs = 3600
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

`max_duration_secs` is a backstop against stuck and endless matches, not a rule of the game, so keep it above the longest
legitimate match. radovid tells the server the limit (`RADOVID_MAX_DURATION_SECS`, read by `bota_server`): five minutes
before it (half the limit for a short one) every client is sent `ServerMsg::TimeLimitNear`, and at the limit the server ends
the match as `ServerMsg::MatchAborted` (the client shows a match-ended screen naming the time limit, never a lost
connection), closes its replay and prints a result with `aborted=time_limit`: radovid records the match as ended by the
time limit, never rated. radovid asks a server it must stop (an admin, its shutdown, a limit the server missed) to finish
the same way with SIGTERM before it kills it.

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

## Protocol 19 coordination

Rebuild every matchmaking client with the service's version-19 protocol/client crates. Versions below 19 are
refused at `Hello`; queue records gained appended audience/search fields and requests. Read
`AccountGameLimit::Limited(n)` or `Unlimited`, rather than interpreting a numeric sentinel. Each account still
holds one ladder rating per ruleset, shared across simultaneous matches.

The service and rebuilt matchmaking clients need the same protocol boundary when they resume exchanging control
messages. Keep game-server/client wire fingerprints matched separately: the control-protocol change does not alter
bota's game wire or replay format. Existing source configurations keep working when their lobby layout is explicit.
The service's [coordination note](../../../radovid/docs/protocol_19.md) describes the generic upgrade contract.

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
limits returns both resolved mode policies. Rebuild clients and service with the current protocol together.

## Leaderboards and profile ranks

`Client::leaderboard(&ruleset, AccountKind::Human, prefix)` returns a sequence number;
its normal reply is `Reply::LeaderboardPage { page }`. Select the mode's rated ruleset ID
(the `rating_ruleset` parent for an unrated sibling). `page.rows` contains the configured
top N, 100 by default, and preserves absolute category ranks during nickname-prefix search.
`page.own` contains this account's position in its own category, including when a different
category is selected. The menu highlights or pins it only when `own.kind == page.kind`:
the Bots tab does not pin a human row. Within the selected category, highlight it among the
displayed rows or pin it separately if absent. The menu caps its visible list at 100 rows,
so test membership against those displayed rows, even when the operator permits a larger
service response. `None` means no rated position exists in that mode.

Prefixes are literal, case-insensitive ASCII letters, digits and underscores, at most 20 bytes.
The SDK rejects invalid prefixes locally as `ClientError::Refused(BadLeaderboardSearch)`;
the service validates raw requests too. Use the echoed mode, category and prefix to reject
stale replies, coalesce edits, and keep at least 300 ms between queries with one in flight.
`Throttled { retry_after_ms }` refuses a whole lookup and can be retried after that delay.

`Client::limits(None).leaderboard_size` reports the configured limit. To read the profile,
request `ClientMsg::Account { account: client.me().id }` through `Client::request`;
`PublicAccount.ratings` holds the per-mode rating and game counts with the appended
`RatingSummary.rank`. Missing records have no rated position. Every client and the service
must rebuild together for the current protocol.

Protocol19 adds `Error::LookupTooExpensive`: a fixed work-limit failure is not a retryable
throttle. Retry `Throttled { retry_after_ms }` after its delay; an unchanged request that
returns `LookupTooExpensive` needs a different request or service-side intervention.
Lookup admission is per account across its sessions, with one in flight; another account
cannot consume that quota. Active bans hide rows and other accounts' profile views.

The protocol-19 service uses schema-4 materialised visibility and category totals.
Reader admission has one shared FIFO queue. Its final per-request SQLite budget is
five million instructions; `LookupTooExpensive` stays non-retryable. A bounded
one-second sweep restores timed-ban ranks (at most 256 expired accounts per tick),
so a large expiry backlog can temporarily leave an expired account unranked. The
SDK DTOs and calls are unchanged.

Profile summaries survive an expensive ordinal-rank walk. Each mode has its own
rank work budget (five million SQLite instructions), with twenty million shared across
all mode ranks; `RatingSummary.rank == None` can mean either budget was exhausted
even when `games > 0`. Keep rendering the rating and game counts with an unavailable
rank. A later cheap mode can still return a rank. Board/own-row response semantics
are unchanged. Short background checkpoints do not reserve the writer turn. A failed
checkpoint keeps normal writes available below the separate 512-MiB emergency WAL
threshold; mutations beyond that threshold are refused until checkpoint recovery.
## Account avatars (protocol 20)

`Client::set_avatar(&png)` and `Client::remove_avatar()` change only the signed-in
account. `Client::get_avatar(account)` fetches an image. These return request
sequence numbers; consume `Incoming::Reply` as for `leaderboard()`.
`Reply::Avatar { account, avatar: Option<Avatar> }` carries the image or absence.
`Avatar { version: AvatarVersion([u8;32]), png: Vec<u8> }` contains a content hash
and at most `MAX_AVATAR_BYTES` (512) bytes. `PublicAccount.avatar` and
`LadderRow.avatar` carry the version only, including the own row. After checking `Client::me()`, fetch the own
profile with `ClientMsg::Account { account: me.id }`.

Prepare a static, non-interlaced, eight-bit 32×32 PNG. Palette and transparency
are supported. Strip all metadata; only IHDR/PLTE/tRNS/IDAT/IEND are accepted.
Resize and quantize before uploading; the service validates the image and never
resizes it. Oversize input is refused locally with `BadAvatar`; a small invalid
image is refused by the service with the same error.

Cache by service, account and version. Cache a fetch under the version it returns
(a simultaneous replacement may return a newer one). `None` clears the current
reference. There are no image URLs or avatar broadcasts. Refresh profile/board
metadata when opening those views. A row's version does not require an immediate
fetch if that cache entry is already present.

Uploads and removals share one attempt per minute per account, including invalid
images and no-op removals. The limit survives reconnects and restarts; honor
`Throttled.retry_after_ms`. Reads share the existing per-account lookup quota.
Other accounts' deleted or currently banned targets return `UnknownAccount`.
Current protocol 21 adds the batch call below; protocol 20 single-image clients remain supported.
The screen implementation belongs to the consumer.

The client control layer checks the bounded PNG envelope before exposing a fetched
avatar to an image decoder. Treat image-decoder errors from an untrusted service
as a missing image; full checksum/pixel validation is performed by a conforming
service. Keep the image cache bounded and discard obsolete account/version entries.

### Conditional avatar batches (protocol 21)

Use Client::get_avatars(&[AvatarQuery { account, version }]) for visible rows,
at most 100 distinct accounts per call. Supply the hash of bytes actually cached,
or None to fetch an image; do not supply a metadata hash when its bytes are absent.
One batch spends one lookup token. Invalid/empty/duplicate batches return
BadAvatarBatch locally and on the service. Split larger visible lists into batches.

Reply::Avatars { changes } carries AvatarChange { account, avatar: Option<Avatar> }
for changed images only. None clears a cached image, including hidden/deleted or
unreadable images. Missing entries leave the supplied cache state unchanged.
Cache replacements under their returned hash. No image history or URLs are kept.

Uploads are decoded and re-encoded by the service with fixed settings; returned
bytes may differ from the file supplied. Hashes identify those returned bytes.
Canonical output must also fit 512 bytes. Corrupt stored images are absent with a
server WARN. Existing valid uploads are normalized on fetch without spending the
upload quota. All client image-cache bounds from above still apply.
