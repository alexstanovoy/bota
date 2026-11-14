# bota

Dota 2, rebuilt as a small deterministic simulator in Rust, so that bots and people can play the same game over
the same open protocol.

Dota is one of the hardest games there is to write a bot for, and the real one gives a bot no clean way in: no
lockstep, no reproducible matches, no fair view of the map. bota gives you that:

- **Deterministic.** The same seed and the same orders give the same match, bit for bit, on any machine. A replay is
  a few megabytes and plays back exactly.
  A script bot's orders are an input to the match, so script bots replay only in the reproducible mode of
  `bota_botscript` (`crates/bota_lua/README.md`, "Determinism"); the match itself stays deterministic given the orders.
- **Fair.** The server is authoritative and tells each side only what it can see. A bot gets exactly the view a human
  gets: no hidden state, no shortcuts.
- **Built for bots.** Lockstep mode waits for slow bots, a whole match can run inside your own program with no
  sockets, and a bot is a normal client in a few lines of Rust.
- **Dota's numbers.** Heroes, creeps, towers, items and timings follow Dota 2 7.41f, with the sources named next to
  the tables.

## What you can play today

- **Two heroes:** Shadow Fiend (abilities, talents, innate) and Pudge (his abilities and innate, without talents or
  Aghanim's upgrades).
- **The full Dota map**, up to five a side, and **Solo Mid**, Dota's 1v1 mid-only ruleset: a side loses on its second
  death or its first lost tower.
- **The map's life:** lane creeps and waves, every neutral camp with its abilities, towers and buildings, day and
  night, couriers, the Glyph, town portal scrolls, wards, runes and the Bottle, Watchers, Outposts, Shrines of Wisdom,
  Lotus Pools and the Twin Gates.
- **Buyback**, at Dota's cost, cooldown and respawn penalty.
- **The shop:** consumables, basic components, boots, the early builds and a few later items, with Dota's stock
  limits.
- **Replays, spectators and chat**, with keyed seats and safe network defaults (loopback only unless you ask, TLS for
  anything else).

What is missing: every other hero; Roshan and the Tormentors; Aghanim's Scepter and Shard, neutral items and most of
the late-game shop; Illusion Runes (illusions are not drawn yet, so the rune is out of the rotation). Some
neutral abilities that only a player-controlled neutral would use are shown but never cast.

To see and play a match you need a client: **dbota**, a separate project, draws bota with the art of your own Dota 2
install. **radovid**, another one, adds accounts, matchmaking and lobbies.

## Get started

You need Rust (edition 2024).

1. Start a server for one match on the mid-only map, with an idle opponent, in lockstep:

   ```sh
   cargo run --release -p bota_server -- --map 2 --mode lockstep --dummy-seat
   ```

2. Run the example bot, which walks its hero to the enemy fountain and attacks everything on the way:

   ```sh
   cargo run --release -p bota_client_core --example attack_move
   ```

   Its whole code:

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

   `Link::play` connects, picks the hero and calls your closure once per tick with the session: what your side sees,
   what it saw before, and the orders standing. You return the orders you want.

3. Record a match and watch it: add `--replay match.brp` to the server, then open the file in dbota.

4. Train or evaluate a bot without a server: `bota_gym` runs many matches in your process, on a pool of workers,
   deterministically from seeds, with your policy deciding for every seat:

   ```sh
   cargo run --release -p bota_gym --example random_duel
   ```

   Its crate docs (`cargo doc -p bota_gym --open`) walk through vectorised environments, evaluation tournaments,
   opponent pools and what a policy may and may not see.

`bota_server --help` lists every option: the map, the number of players, tick rate, spectators, chat, cheats for
practice, TLS. What the server promises a bot (what it never tells, why it refuses an order, how ticks and chances
work) is the rustdoc of `bota_proto::contract`: `cargo doc -p bota_proto --open`.

## For developers

[DEVELOPMENT.md](DEVELOPMENT.md) has the crates, the server's options and launcher contract, the client library in
depth, how to add a hero or an item, the protocol and determinism rules, and the tests.

## Replay provenance

Public profile provenance can accompany replay metadata. A matchmaking launcher supplies
`BOTA_MATCHMAKER` (its public control URI, without credentials, query or fragment;
otherwise `RADOVID_PUBLIC_IDENTITY` from the launcher) and
`BOTA_PROFILE_ACCOUNTS` (public account ids in slot order, comma separated; `-` for a seat
without a profile). Accounts without a matchmaker leave the identity unknown and do not prevent launching.
Private/local IP literals and single-label hostnames are never recorded as a matchmaker.
A launch without either writes no Profiles record. Embedded servers can use `game_loop::run_with_profiles`; the legacy `run` API
keeps its existing recording policy. Provenance goes in the metadata sidecar and replay
trailer when explicitly enabled, including capped prefixes, without changing replay records or the wire identity.
Readers treat it as untrusted text and account claims, never as permission to connect.

## License

MIT, see [LICENSE](LICENSE).
