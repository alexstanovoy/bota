# bota — design

A simplified Dota 2 in Rust for AI bots and humans. Fully deterministic simulation,
server authority, minimal dependencies.

## Key decisions

| Decision | Choice | Rationale |
|---|---|---|
| Client rendering | macroquad | one direct dependency, 2D + text out of the box, Linux/Win/macOS/WASM |
| Tick mode | Realtime **and** Lockstep | humans play in realtime, bots are debugged/trained in reproducible lockstep |
| v0.1 scope | 1v1, three lanes, a handful of heroes | a vertical slice of the whole stack on minimal content |
| Transport | TCP + length-prefixed frames | no tokio: a thread per client + a simulation thread + mpsc |
| Serialization | `serde` + `postcard` | derive instead of hand-written put/get; the format is compact (varint) and stable within 1.x |
| Player networking | full snapshots with fog | a 1v1 snapshot is budgeted at 4 KiB by a `bota-proto` size test, 120 KB/s at 30 Hz at most. Deltas will be needed by 5v5, not earlier |
| Live spectating | the same snapshots, without fog | client-side simulation is impossible, and that is intentional |
| Replay | fogless `ServerMsg` stream + all orders | plays back without a server: the client reads frames from a file instead of a socket; coupled to the wire format only, so it survives balance changes; the client still cannot simulate |
| PRNG | `rand_chacha::ChaCha8Rng` in `bota-server` | value-stable by the crate's contract; `rand::StdRng` is **forbidden** — it is allowed to change its algorithm in a minor release |

## Structure

```
bota/
├── Cargo.toml               # workspace, resolver = "3"
├── crates/
│   ├── bota-proto/          # shared vocabulary + codec. deps: serde, postcard
│   ├── bota-server/         # simulation + networking + lobby. deps: proto, rand_chacha, rustc-hash, clap
│   ├── bota-client/         # macroquad: rendering, input, spectating. deps: proto, macroquad, resvg, clap
│   └── bota-bot/            # the rule bot and the seam a bot plays through. deps: proto, clap
```

```
        bota-proto
        ↑    ↑    ↑
   server  client  bot
```

The bot and the client are symmetric consumers of `proto`: a human and a bot see
literally the same `WorldView` type. No asymmetry that could be exploited.

### Membership criterion for bota-proto

The single rule deciding where code lives:

> If it does not cross the wire and is not needed to read the wire — it is not in `proto`.

| In `bota-proto` | In `bota-server` |
|---|---|
| `Fixed`, `Angle`, `Vec2`, `EntityId`, `SlotId`, `PlayerId`, `Team`, `HeroId`, `AbilityId`, `ItemId`, `AbilitySlot`, `ItemSlot`, `MapId`, `UnitKind` | `World`, `step`, units, combat, movement, vision, economy |
| `Order`, `EventKind`, `WorldView`, codec and framing | `Command`, `Event.visible_to`, `MatchRng`, `Stream`, `Chance`, `Ratio` |
| `MatchInfo`, `ClientMsg`, `ServerMsg`, `ReplayRecord` | `MatchConfig`, balance constants, hero stats, ability implementations |

A `bota-core` crate holding the simulation was rejected: "core" has no checkable
membership criterion, and `seed`, `tick_rate` and `PlayerId` leak into it. With the
simulation in the server, the client is **structurally** incapable of simulating:
`World` lives in a crate it does not depend on.

### Module layout

```
proto/src/
├── math.rs       Fixed (Q16.16 in i32), Angle (brads), Vec2
├── ids.rs        EntityId, SlotId, PlayerId, Team, HeroId, AbilityId, ItemId,
│                 Aim, EffectId, AbilitySlot, ItemSlot, MapId, UnitKind
├── attrs.rs      Attribute, Attributes
├── order.rs      Order, Target, Cheat, ModifierSpec, MAX_MODIFIER_TICKS
├── event.rs      EventKind, DamageKind
├── view.rs       WorldView, UnitView, PlayerView, ProjectileView, AbilityView,
│                 ItemView, EffectView, LootView, Kit, StatusFlags
├── msg.rs        ClientMsg, ServerMsg, MatchInfo, ShopEntry, Role, TickMode, Pick,
│                 LobbySlot, RejectReason, MatchStats, SlotStats, SlotOrder,
│                 ReplayRecord
└── codec.rs      encode_frame, decode_payload, FrameReader, CodecError
```

```
server/src/
├── engine/       how state is kept and walked; knows nothing of Dota
│   ├── entity.rs     Entity, Index, Generation, EntityAllocator
│   ├── table.rs      Table<T>: one component per entity slot, generation-checked
│   └── fnv.rs        Fnv: FNV-1a for the fingerprint
├── game/         the game; KNOWS NOTHING ABOUT SOCKETS
│   ├── world.rs          World: the component tables and the tick order
│   ├── match_world.rs    for_match, advance, validate_order, victor, match_stats
│   ├── project.rs        World → WorldView, with fog and without
│   ├── hash.rs           world.hash() over the whole state
│   ├── rng.rs            MatchRng over ChaCha8Rng: streams by purpose, Ratio/Chance
│   ├── seat.rs           Seat, Kept: what belongs to a player rather than a body
│   ├── progress.rs       progress inside an action, in beats
│   ├── movement.rs       isqrt, stepping, turning, facing, distances
│   ├── cells.rs          one bit per terrain cell, for sight
│   ├── clearance.rs      the ground as a body meets it: room per node, exact capsule test
│   ├── path.rs           A* over the walking lattice, corners drawn tight
│   ├── bodies.rs         where every body stands, by bucket
│   ├── spots.rs          Spots<T>: points sorted by x, asked by square
│   ├── local.rs          the next stretch of a walk: A* over spot, facing and tick
│   ├── vision.rs         fog of war: sight blocks and sight lines
│   ├── forest.rs         what of the forest is down and what was put up
│   ├── ground.rs         elevation tiers, water, walkability
│   ├── components/       one file per component
│   ├── systems/          one file per system: combat, walking, casting, gear, econ …
│   └── config/           what the game is made of: ability, blockers, camp, hero,
│                         item, map, match_config, place, protect, roster, rules,
│                         spawn_modifier, terrain, trees, unit, unit_neutral, wave
├── net/          conn.rs: accept loop, reader/writer threads; outbox.rs: Outbox
├── lobby.rs      Roster (PlayerId ↔ SlotId), seats, picks, readiness
├── game_loop.rs  lobby phase, then the tick loop in both modes
├── replay.rs     writes the replay: fogless frames plus per-tick orders
├── profile       phase timings of a tick, sampled with the `phase-profile` feature
└── main.rs       clap arguments
benches/dummy/    criterion: the dummy tick loop, the trainer skirmish, micro cases
```

No ECS library and no arenas of whole units: an entity is a generational handle from
`EntityAllocator`, and each component is its own `Table<T>` indexed by the handle's
slot. Walking every entity is `EntityAllocator::iter`, in slot order, plus a lookup per
table. A freed slot is handed out again under a new generation, and a table slot
remembers the generation it was written for, so a stale handle never reads whoever took
the slot over.

## Contracts

### Simulation (server/src/game)

```rust
pub struct MatchConfig {
    pub match_id: u64, pub master_key: [u8; 32], pub picks: Vec<Pick>,
    pub map: MapId, pub tick_rate: u16, pub mode: TickMode, pub ack_timeout_ticks: u32,
    pub cheats: bool, pub spawn_modifiers: Vec<SpawnModifier>,
    pub immortal_structures: bool,   // server-only: structures never drop below 1 hp (training, practice)
    pub immortal_heroes: bool,       // server-only: heroes never drop below 1 hp (last-hit practice)
}

impl MatchConfig {
    pub fn rng(&self) -> MatchRng;        // see below
    pub fn info(&self) -> MatchInfo;      // projection onto the wire, the type has no seed field
}
```

The match seed is derived with `rand_chacha` itself, no separate hash function needed:
`master_key` is a 32-byte seed, `match_id` is a stream number. A match is reproducible
from the pair `(master_key, match_id)`, which is convenient for debugging. Implemented
in `game/rng.rs`:

```rust
// MatchRng::new(master_key, match_id)
let mut root = ChaCha8Rng::from_seed(*master_key);
root.set_stream(match_id);
let mut seed = [0u8; 32];
root.fill_bytes(&mut seed);

// one stream per purpose — crits, evasion, pierce, camp rosters, flagbearers,
// bounties
rng.global(Purpose::Wave)
rng.for_unit(Purpose::Crit, unit, source)
```

Streams are separated by purpose (`Purpose`: `Crit`, `Evasion`, `NeutralSpawn`, `Wave`,
`Pierce`, `Bounty`, `DamageRoll`) so that a new draw in one place does not shift generation anywhere else. A
per-unit stream is keyed by `(purpose, slot index, source)` packed into the 64-bit
ChaCha8 stream id — purpose in the top bits, slot index in the middle, a source
byte to separate several sources of chance of one purpose on the same unit (the
evasion a unit rolls as a target and the uphill miss it rolls as an attacker). The key
uses the slot index rather than the full `EntityId`, so the stream id space stays
bounded. The world keeps each slot's opened crit, evasion and pierce `Chance` for as
long as the match runs, so a unit reusing a freed slot continues that slot's hidden
sequence, which no observer can distinguish from a fresh one.

```rust
impl World {
    pub fn for_match(cfg: &MatchConfig, rng: MatchRng) -> World;  // rng is initial state, not config
    pub fn advance(&mut self, cmds: &[Command]) -> Vec<Event>;
    pub fn view(&self, team: Team) -> WorldView;        // with fog
    pub fn view_full(&self) -> WorldView;               // spectator
    pub fn validate_order(&self, slot: SlotId, unit: Option<EntityId>, order: &Order)
        -> Result<(), RejectReason>;
    pub fn can_see(&self, team: Team, entity: Entity) -> bool;
    pub fn victor(&self) -> Option<Team>;               // Team::Neutral: a Map2 draw
    pub fn match_stats(&self) -> MatchStats;
    pub fn hash(&self) -> u64;                          // determinism check
}
```

`seed` is not configuration but initial hidden state, so it is passed as a separate
argument, already a constructed `MatchRng`; the policy of deriving it belongs entirely
to the server.

The simulation knows about `SlotId` but not `PlayerId`: network identity must not leak
into the rules of the game. The mapping table lives in `Roster` in the lobby.

`Event.visible_to` is computed by the simulation — who sees what is a gameplay
question. The network layer only routes; what goes on the wire is `EventKind` without
the mask.

### Exchange

```
server:  World --view(team)--> WorldView --encode_frame--> socket
client:  socket --FrameReader--> WorldView --> rendering
```

The client's `WorldView` is born from the wire, not from a constructor. The client has
no `World` and cannot have one, so there is no second `new` contract either. The
asymmetry of the sides is expressed by two types — `World` versus `WorldView` — not by
two implementations of one trait.

There is intentionally no trait over `World`: no polymorphic call sites exist, and
determinism demands exactly one implementation.

## Determinism rules

1. No `f32/f64` in `bota-proto` and `bota-server`: `clippy::float_arithmetic` is denied
   in the workspace lints.
   Float operations themselves are deterministic per IEEE-754, but `sin/cos/sqrt` from
   libm are not — they differ across glibc / musl / macOS / wasm. The client renders in
   float freely. The bot is also free to think in float: what gets recorded are its
   orders, not its reasoning.
2. Scalars are `Fixed` = Q16.16 in `i32`, multiplication through an intermediate `i64`.
   Range ±32768 units, precision 1/65536. The 18432-unit map keeps squared distances
   inside an `i64`; segment projections that would square a dot product go through
   `i128`. The operators debug-assert on overflow and saturate in release: a
   saturated value stops at the end of the range, a wrapped one would land on the
   far side of the map.
3. Angles are "brads": `u16`, 65536 = a full turn. There is no trigonometry: facings and
   headings use an integer octant approximation (`facing_towards`, `heading_of` in
   `game/movement.rs`).
4. Distances are compared as squares; where a length itself is needed, `isqrt64`.
5. Entity iteration is always in ascending slot order (`EntityAllocator::iter`).
   Nothing in the simulation iterates a hash map; `rustc_hash::FxHashMap` is allowed
   for lookup only, keyed by integers.
6. The game loop keeps at most one pending order per seat: an order validated on
   arrival replaces whatever that seat had pending, and the tick's commands go to
   `advance` in slot order.
7. Time exists only as ticks (`u32`), 30 ticks/sec. No `std::time` in `game/`.
8. Damage, gold, experience are integers.
9. External primitives are taken only if value-stable. `rand::StdRng` and
   `std::collections::hash_map::DefaultHasher` explicitly give no such guarantee
   between releases: the former is replaced by `rand_chacha::ChaCha8Rng`, and for
   `world.hash()` we write FNV-1a (ten lines, xor and multiply in a loop).
10. `world.hash()` covers the whole state, hidden included: component tables, stream
    positions, every `Chance` mask. A divergence in randomness consumption must move
    the hash on the tick it happens, not when its first visible outcome differs.

`game/tests/determinism.rs` pins whole-match fingerprints on every map: seeded order
streams drive a match, and the world hash and the encoded view and event streams of
both sides every 1500 ticks, plus the final stats, must match the recorded values. A
deliberate rule change re-records them with the ignored `print_pins` test.

### Chances (crits, evasion, pierce)

The requirements conflict: an exact 30% rate **and** no way to predict the next crit.
A simple accumulator (`acc += 0.30`, firing at `acc >= 1`) gives the first but not the
second: it is periodic, an observer sees crits in damage events and reconstructs the
whole phase after one or two observations.

The solution is exact counting per block with a hidden order inside the block. A chance
is declared as a fraction:

```rust
pub struct Ratio { num: u8, den: u8 }     // rules.rs: SYLLA_CRIT_CHANCE, one per level

pub struct Chance {
    stream: Stream,   // this source's own hidden ChaCha8 stream
    mask: u64,        // which attempts of the current block hit
    idx: u8,          // position within the block
    current: Ratio,   // the ratio the current block was built with
}

impl Chance {
    pub fn roll(&mut self, ratio: Ratio) -> bool {
        if self.idx >= self.current.den() {
            self.current = ratio;                     // new ratio takes effect here
            self.idx = 0;
            self.mask = Chance::pick(&mut self.stream, ratio); // partial Fisher-Yates
        }
        let hit = self.mask & (1 << self.idx) != 0;
        self.idx += 1;
        hit
    }
}
```

- The rate is exactly `num/den` per block. Balance constants are declared as fractions,
  `den <= 64` (the width of the mask).
- The order comes from a ChaCha8 stream: observing past crits says nothing about the
  next block. A replay reproduces bit-for-bit — the stream is deterministic, just
  unreachable from outside.
- Every source starts mid-block at an offset drawn from the same stream, so block
  boundaries are not known to an observer.

Residual leak: within a block the opponent can count (saw 3 crits in 6 hits — knows the
next 4 are clean). Unavoidable for any scheme with an exact rate. The alternative is
giving up the exact rate for a plain hidden PRNG, which contradicts the determinism
requirement.

When the chance changes (a crit buff), the current block finishes under the old
fraction; the new one takes effect from the next block.

The PRNG is also used where an exact rate is meaningless: which roster a camp fills
with, which melee creep of a wave carries the flag.

### Hidden state

The hard rule: the client receives nothing from which a future outcome can be derived.
Players send intents, the server alone computes, and every tick it broadcasts the
outcome.

Secrecy is a property of the channel, not of the data, so the simulation does not think
about it. The protection is structural: the types of `bota-proto` are incapable of
expressing hidden state. `MatchInfo` versus `MatchConfig`, `WorldView` versus `World`.
A leak becomes a compile error, not a review oversight.

The seed is **never** published — not even in replays: a replay is a recorded stream
plus the orders, nothing in it is re-simulated, so nothing in it needs the seed.

Server-only, never reaches `WorldView`:

- `MatchRng` and the positions of all streams;
- each unit's `Chance { mask, idx }`;
- outcomes of scheduled events that have not happened yet (which roster a camp fills
  with next).

It also follows that client-side prediction covers only movement and animation. Damage
numbers and the fact of a crit arrive as events from the server.

Two further channels are closed by rule, because a reward-driven bot will find and
exploit any leak a human reviewer shrugs off:

- A reject reason does not depend on hidden state. A dead target and a fogged one get
  the same `UnknownTarget`, so probing the fog with stale handles reveals nothing.
- A unit never acts on what its team cannot see. A standing
  `Order::Attack { target: Target::Unit(..) }` whose
  target left the team's vision degrades to attack-moving toward the last seen
  position; the unit does not track the hidden target, so its own path reveals
  nothing either.

## Game model v0.1

- Map 18432×18432 — Dota's scale, so speeds, ranges and vision keep their Dota
  absolute values. Three lanes: mid along the diagonal, top up the west edge and
  along the north edge, bottom along the south and east edges. Terrain is a 288×288 bit
  grid of 64-unit cells; walking is planned on a 576×576 lattice of 32-unit nodes
  laid over it.
- A second map, `MapId(1)`, is the game's own hero demo map (`hero_demo_main`),
  imported the same way: one short lane bending through its real path corners,
  a single tier-one tower a side, two fountains, its own 187 trees, its own
  ground — and no Ancients at all, so nothing ends a match there but the
  clock. A real map rather than an invented training lane is ground the bots'
  habits can carry to the big map. Its terrain is baked exactly like the big
  map's: walkability
  from the map's own gridnav, elevation tiers rasterised off its physics mesh
  in 128-unit steps — river bed 0, ground 1, the high spots 2, the cliffs
  above — the river as the water mask, and its one fog blocker wall sealing
  the river pit. It keeps two invented pullable camps in the wooded pockets
  either side of the lane, because the jungle's behaviours are tested against
  this map and the real one runs no jungle. Anything a map may lack is data
  now: `ancients` are per-side options, a side with none anchoring its lane
  at its wave spawner; the forest, the fog walls, the lane tree-clearing band
  and the baked ground are per-map tables. Whether the lane centerline is
  drawn through the towers is a map switch too: the big map's straightened
  lanes are defined by their towers, while the demo map's corners trace the
  real road and its towers stand beside it — drawn through them, every wave
  hooked around its own tower on the way out.
- A third, `MapId(2)`, is the Dota map to a short finish; see Map2 below. What ends
  a match is map data (`death_limit` and `tower_ends_it`) rather than a check on the
  map id in `fight.rs`: a rule keyed on which map it is cannot be given to a second map
  without being written again. The demo map carries neither ending and ends the way
  the big map does.
- The per-map field cache (`BASES` in `clearance.rs`) is indexed by `MapId`, not by
  the position in `MAPS`, so the two have to agree; a test says so, since nothing
  else would notice a map added out of order.
- Teams Radiant / Dire, 1v1 (the architecture is sized for 5v5).
- Buildings: three towers per lane per side — tier one by the river, tier three by
  the base — plus a pair of tier fours by each Ancient, two barracks per lane
  behind its tier three (all at their real map positions), and a fountain that
  heals and burns. What may be struck when is map data: `MapDef.protection` is a
  list of rules, each guarding a structure with an and/or condition over the same
  side's already-fallen structures, and the guard is worn as invulnerability,
  recomputed after stats each tick. The Dota table opens each lane tower by tower
  into its barracks, opens the tier fours on any fallen tier three, and opens the
  Ancient only once both tier fours are down. A condition tree rather than a flat
  prerequisite list because "any tier three" and "both tier fours" are one Or and
  one all-matching name away from each other, and a flat list can say only one of
  them. A structure no rule names is open from the horn.
- A lane's centerline runs through every tower of the lane, so a wave marches from
  tower to tower and can never wander past one outside its own acquisition range.
- The match opens with a 30 s pregame: the game clock counts up from -0:30 and the
  first wave walks out at 0:00, when passive gold starts. `MatchInfo` carries the
  pregame length so the clock renders without knowing the ruleset.
- The landmarks are the current Dota 2 map, extracted from the installed game's
  `dota.vpk` with ValveResourceFormat and shifted by half the map so Dota's
  origin sits at the center: `MAP_SIZE` 18432, both fountains, both Ancients,
  all 22 towers, all 6 lane spawners and all 28 neutral camps at their real
  positions. The two sides are not mirrors; each carries its own table.
- The terrain is the same map's own ground, baked in `game/config/terrain.rs` and
  read through `game/ground.rs`: the gridnav's static walkability (cliffs, pits, the map edge close their cells
  before trees and buildings do) and, from the physics mesh, an elevation tier
  per cell in 128-unit steps — river bed 0, lane ground 1, highground 2, bases
  3 — plus the water mask of the river and pools. A ranged attack landing on
  ground higher than its attacker at impact uses Dota's 25% pseudo-random
  miss distribution for that attacker; buildings, flying attacks and
  abilities never miss this way. The terrain
  rides in `MatchStart` run-length encoded, and the client bakes it into the
  ground texture for the world and the minimap.
- Vision is a radius with sight lines walked over the terrain cells. A cell is
  opaque to a viewer when its ground is higher than the viewer's, when a tree
  stands on it, or when one of the map's own fog blocker walls crosses it —
  eleven named walls of `ent_fow_blocker_node` points, imported like
  everything else, which is what seals the river pit even through its
  entrance. A named group holds several separate walls: only nodes within
  the blocker span of each other bridge a segment, so the far-apart jumps
  inside a group — the two pits share one name — are breaks, not walls. Buildings, water and units block nothing. The viewer's own cell
  and the target's cell never block, so standing beside a tree does not blind
  and a treeline's edge stays visible; a target on ground above the viewer is
  always dark. The opaque cells ride in `MatchStart`, and the client walks
  the same sight lines from its own units to shade unseen ground in the world
  view and on the minimap; spectators see everything. The map's
  `ent_fow_revealer` points wait for outposts.
- Trees are imported one for one from the same map: all 2475
  positions — the main entity lump plus the base layers of both sides — live as
  a table in `game/config/trees.rs`. Two carves adapt them to this map: trees within
  the lane-clear band of a straightened lane centerline are dropped — the real
  forest follows the real curved roads, and these lanes walk tower-to-tower
  chords — and a small pad around each fountain stays clear. The
  full tree list rides in `MatchStart`, so the client draws without knowing the
  layout rules. A standing tree blocks walking and sight. Eating one with a tango
  or cutting one with an item takes it down for `TREE_REGROW_TICKS`, and a tree
  an item plants stands for `PLANTED_TREE_TICKS`; `game/forest.rs` keeps what is
  down and what was put up, and every view carries both (`felled_trees`,
  `planted_trees`).
- The jungle belongs to `Team::Neutral`, hostile to both sides; seats never sit
  there. The twenty-eight camps stand where Dota's own neutral spawners stand. They
  fill with neutral creeps one minute past the horn and every minute after, but only while the camp box is empty — any body inside
  blocks the spawn, which is camp blocking. A neutral answers whoever comes into
  its aggro range or hits it; led past its guard distance for longer than its
  window it goes home deaf, and arriving home restores nothing. Its bounty goes to the killer, its experience to the
  killer's team nearby.
- Creeps: 3 melee + 1 ranged every 900 ticks (30 s) on every lane; a siege creep
  joins wave 11 and every tenth wave after, a flagbearer wave 5 and every second
  wave after, and the counts grow later in the match (`rules.rs`). A wave marches its own lane's waypoints and never joins another
  lane. A lane whose enemy melee or ranged barracks has fallen spawns that kind
  super, its siege once both are down, and every enemy barracks fallen makes the
  whole side's waves mega, at the game's own numbers; the flagbearer stays plain.
  A waypoint counts as reached from anywhere inside its radius, but only while
  the grid line to the waypoint after it is clear: the radius spans a tower,
  and a corner cleared through the tower it routes around leaves a creep
  wrestling that tower.
  A tower landmark becomes a stop beside the tower, on its lane side away from
  the base it guards. The centre of a 144-unit tower hull is out of reach; the
  nearest open cell picks a grid-arbitrary side and sends waves round the back;
  and a stop facing the base lands in the pocket between a tier three and its
  barracks. The lane side is open on every tower of both maps.
  The routes are laid per world on the ground as it stands and laid again the
  next time a wave asks after the ground changes, with every marcher put at
  the waypoint of its new route nearest to it, so a wave does not walk round
  the empty ground a fallen tower stood on. A found path keeps only the corners
  the grid line cannot skip.
- Heroes: Sylla, Pudge and Shadow Fiend (below). Levels 1–30 on the game's own
  experience table and respawn times, taken from the wiki in September 2026.
- Economy: passive gold 1/sec, last hits, a hero kill bounty priced by the
  victim's streak, a hero's head worth in experience a base, thirteen percent
  of what the fallen had earned and the game's own bonus for the streak it
  ends, and a death that costs the fallen a fortieth of its net worth, capped
  by the purse. The `Died` event carries the gold the killing side was
  paid, so a seat reads off the wire what a fight moved. A death that costs
  nothing but the respawn wait is one a bot learns to feed. The dying
  hero's gold is not handed to the killer but vanishes, as in Dota: the
  penalty prices the death, the bounty prices the kill, and a broke victim
  still pays its killer in full.
- Attributes are Dota's three: strength buys health and health regeneration,
  agility buys armor and attack speed, intelligence buys mana and mana
  regeneration, and whichever one a hero is primary in buys its attack damage.
  They are held in fixed point rather than whole points, because Dota's growth
  per level is fractional and rounding it to whole numbers would put the same
  hero on two different curves depending on where the rounding fell. They sit
  on `UnitDef` and in `Stats`, so a creep simply has none of them and pays for
  nothing; the derivation runs once, after items have been added and before
  anything reads what attributes pay for.
  The alternative — leaving attributes out and giving every item flat health,
  mana and damage — was rejected because it throws away the whole cheap end of
  the Dota shop: Circlet, the three 140-gold attribute items, Bracer and its
  two siblings all exist to be bought as attributes, and inventing replacements
  for them is work with nothing behind it.
- Attack speed is a number on the Dota scale, where 100 is a unit's own pace,
  and the interval between two attacks is the kind's own interval scaled by it.
  A percentage taken off the interval directly was rejected: percentages off an
  interval do not add up — two sources of twenty percent are not forty — so item
  bonuses and Frenzy could not share a footing. The bounds are Dota's, 20 to 700.
- Items are built from components. Every built item's price is exactly the sum
  of its parts, with the difference carried by an ordinary catalog entry — a
  recipe — rather than by a number on the item, so there is one rule for what a
  build costs and no second place to keep it. Buying a built item buys only the
  parts the seat does not already hold, asking the same question of a part that
  is itself built, which is what makes buying a component now and the whole
  later worth anything. The build itself runs once a tick over every hero's
  bag rather than at each place an item can arrive: a purchase, a slot moved
  and a courier setting one down would otherwise each need their own hook, and
  a missed one would leave parts sitting side by side. Only what a hero
  carries builds; the stash does not, since it is a shelf at the shop and not a
  pair of hands.
  Order validation and execution use the same purchase plan: missing parts,
  their catalog cost, and room for all of them before assembly. Requiring the
  whole item's price at validation would refuse an affordable upgrade; counting
  only its final slot would accept parts that cannot arrive. Owned, unmarked
  parts in the hero's bag, backpack and stash count toward the plan. Foreign or
  sale-marked parts do not, since assembly cannot consume them. Purchases use
  the hero's location and storage even when the order names its courier; courier
  cargo is neither a component source nor a purchase destination.
- Items follow the Dota slot topology, engine in `game/systems/gear.rs`. A seat owns
  fifteen slots: six inventory, where items work; three backpack, where they ride
  inert — and a stack leaving the backpack for the inventory is muted for six
  seconds before it works again; six stash. A purchase spends gold anywhere, but lands in the
  inventory only inside the home shop area (the fountain circle) — bought
  remotely it waits in the stash, and the stash itself opens only at that shop.
  Selling also happens at the shop: half price back, the full price for an
  untouched item within ten seconds of purchase. `Order::Swap` swaps any two slots.
  Carried bonuses are flat and apply only from unmuted inventory slots; a pool
  keeps its filled fraction whichever way its maximum moves. Growing by the whole
  delta and shrinking by clamping alone is a mint: a Power Treads wheel
  (strength → agility → intelligence → strength) re-gains on every switch back
  what the switch away never took. The scaling floors, so a full
  wheel can only lose a sliver, never gain one; and a pool that held anything
  is kept off zero, so the wheel cannot kill its owner either. Consumables
  drip over their duration (Healing Salve ten seconds, Clarity twenty-five) and
  spill on any hit from a hero or a tower. Items survive the hero's
  death on the seat.
- An item set to an attribute — Power Treads — keeps which one on the stack
  rather than in the catalog, and the wire carries it in `ItemView`, since two
  players holding the same item may have it on different attributes and the
  client has to draw which. Switching is an ordinary `Use` with no target:
  a second order kind for one item would be a wire change bought for nothing.
- Blink is a point-targeted use like any other. Aimed further off than it
  carries it carries as far as it does along the same line rather than
  refusing, which is how Dota reads a click past the edge of the range. A
  landing spot on closed ground steps back along that line a grid cell at a
  time until it finds open ground: refusing outright would have been simpler,
  but it makes the item unusable at exactly the cliffs it exists to cross.
  A blow from a hero or a tower sets it back, and that lives in the
  same place a blow already puts a Salve out, so there is one pass over what a
  blow breaks rather than two.
- Charges gained from enemy casts — Magic Stick and Wand — are counted where
  the cast succeeds, not from the event stream: the events a side is told of
  are already filtered by what it may see, and charges do not answer to
  vision. A stack that may gain charges is kept when its last one is spent;
  every other stack is gone with it.
- An item on the ground is an entity: a transform and the whole stack, charges,
  attribute mode and the rest riding along untouched. An entity rather than a
  world-level list because everything wanted comes with it — a stable
  `EntityId` for an order to name, and the same visibility rows units use, so
  the fog covers ground items without a line of new code. It has no team, no
  health and no hull: it is walked through, cannot be struck, and lies there
  until somebody takes it. Anybody with a bag may — enemies included, which is
  the whole drama of a courier shot down over the river or a Gem dropped in
  Dota. What keeps theft from being a bank raid is ownership, below.
- Two orders cover the ground: `Put` lays what sits in a bag slot out —
  at a point, underfoot when aimed at nothing, or into the first free slot of
  an allied bag when aimed at a unit — and `Take` picks a ground item up.
  Dropping and handing over are one order, not two, because they are one
  motion — out of the bag, differing only in where it lands — and
  `Target` already spells the difference. Both orders walk their unit
  into reach first, the way Dota reads a drop aimed across the map; an aimed
  unit may be moving, so following is needed anyway, and one walk serves both.
  The walking lives in one `Handling` component and one tick pass shaped like
  the courier's errands, cancelled where an errand is: any later order calls
  it off. Item actives refuse beyond their reach instead of walking, and stay
  that way: a use is aimed where the fight is, a put is aimed where the feet
  will be.
- The stash does not `Put`: it is a shelf at the shop, not a pair of
  hands. Move the item into the bag first.
- Selling away from the shop marks the stack for sale instead of refusing; a
  second sell order on the slot unmarks it, so no new order kind is spent on
  cancelling. A marked stack still carries its bonuses — it is owned until it
  is sold — but takes no part in builds, or the boots marked for sale would
  vanish into Power Treads mid-flight. The sale itself is a per-tick pass over
  what stands at the shop, the same shape and the same argument as builds:
  the marked stack may arrive by courier, in its owner's bag, or already sit
  in the stash, and a pass owns all three where hooks would multiply. The
  courier folds in with one change: delivery hands over its load, then takes
  every marked stack from the owner's bag, and the existing put-back leg
  carries them to the stash where the pass sells them. A courier called with
  an empty bag still flies out when something is marked: the call is the ask.
- Every stack knows the seat that bought it, and only that seat may sell or
  mark it. Without this, sell-by-ally is a wire for pumping gold between
  seats, and an enemy who picks a dropped item up cashes it at the shop.
  Wearing, using and handing back are all allowed on anybody's stack — the
  rule guards the till, not the hands. The wire carries the owner as
  `ItemView::owner`.
- A courier's Burst and Shield keep its current errand running while their
  effects and cooldowns follow the ordinary casting path. Keeping only the last
  movement order would lose delivery and stop following a moving owner, even
  though neither buff requests a new destination. Movement, Stop and explicit
  errand casts still cancel or replace the errand. This exception names the two
  auxiliary abilities, not every own-target cast: the errands themselves are
  own-target casts and must still replace one another.
- Fog of war is mandatory: without it a bot learns to play with full information.
- Victory: the Ancient falls.

Aggro replicates Dota. Nobody ranks targets by kind: creeps and towers take the
closest enemy in reach — creeps fighting creeps and towers shooting creeps are
emergent, because creeps arrive first. On top of that sit the aggro calls:

- An attack order against an enemy hero, and every attack swing at one, calls the
  victim's creeps within a radius of the attacker — and the victim's towers the
  attacker stands in reach of — onto the attacker. Creeps hold the grudge for a
  couple of seconds; a tower holds its target for as long as it stays in reach.
- An order aimed at any ally calls enemy creeps and towers off the orderer: the
  classic last-hit-under-pressure trick. Against a healthy ally the order itself is a
  follow, turning into a deny once the ally is low enough.
- Each creep and tower can be called onto a target at most once per call cooldown. A
  call-off works at any time, and until that cooldown expires the called-off unit
  prefers any other target over the orderer, coming back only when nobody else is in
  reach: the trick redirects, it does not blind.
- A creep holds a non-hero target until it dies or the chase breaks: standing closer
  steals no attention, so last-hitting next to a busy wave is safe. A target inside
  the creep's own attack range is held no matter what it is — a ranged creep keeps
  firing at a hero who stays in its range. What the aggro window limits is chasing a
  hero beyond that range: when it closes, the creep re-assesses from the closest
  again, and a kited chase loses to whatever got closer on the way, a tower included.
  Last-hitting creeps calls nobody.
- A hero fights only when told to, but a fight it was told to have carries itself:
  when an attack order's target dies, the order degrades to an attack-move at the
  spot it fell, and the fight carries on with whatever acquisition finds there.
  An aimed cast takes the body over: the order it was given over is not returned
  to, the body swings at nothing while the cast waits, and once the cast has
  gone off the hero takes on whatever acquisition finds. A cast at oneself asks
  nothing of the body and leaves its order be. Arriving off a move
  order or stopping leaves the wave alone. A move order ignores enemies for its
  whole length, Hold attacks whatever is in range without moving, attack-move
  acquires along the way.
- A body has two radii, as in Dota: the collision size nothing walks into, and
  the smaller bound radius that attack range, cast range and areas are measured
  to. One radius for both cannot be right: the bound radius as a hull packs waves
  tighter than Dota's, and the collision size as a reach lengthens every attack
  and cast by the difference. All contact is solid: the
  distance between two units never drops below the sum of their collision sizes,
  and a step deeper into anybody's circle is refused. Easing apart is a safety
  net for spawns and shoves, four units a tick; nothing else moves a body but its
  own step.
- Target retention checks validity, the current hold and reach before ranking a
  replacement. Once a reachable held target has class zero, the best class in
  that unit's priority order, a full search cannot replace it: replacement needs
  a strictly lower class. That search is skipped, without changing acquisition
  for missing, invalid or out-of-range targets, or searches from a worse class.
- Walking is planned in three layers, one over the other, all in integers.
  The static layer is the ground as a body meets it. Terrain closes 64-unit
  cells, met as squares; a tree is the circle of its trunk; a structure the
  circle of its collision size. Over them a lattice of 32-unit nodes keeps at
  every node the room a body has there: the distance from the node's centre to
  the nearest obstacle, capped at 96 units. The terrain's part of the field is
  baked once per map in two one-dimensional passes and cached for the process;
  buildings and trees are laid into it as circles and taken out again by redoing
  the window about them, so a tree felled costs a window, not the map. An exact
  test says whether a body of a radius can walk a segment: the circles come from
  a bucket index, the closed cells are met about every node the segment crosses,
  and a node whose room exceeds the radius and half its own diagonal is passed
  without looking. A body already inside an obstacle is let out: an obstacle it
  overlaps stops only a step that comes nearer it somewhere along the way than
  where the body stands (judging only where a step ends lets it through the
  middle of a wall of bodies). Testing only the walker's centre on the 64-unit
  grid put a hero's body inside a tree and opened or shut a two-cell corridor by
  the luck of where its centres fell. Read at 32 the corridor is right for every
  body up to the siege creep's, and the exact test is what makes a plan honest:
  nothing a route asks is refused by the ground it was planned on. Closed terrain
  stands as solid squares for the body, which
  shuts a two-cell corridor to bodies wider than 32 units, the melee and siege
  creeps; Dota reads terrain against the centre alone. One field and one rule
  were taken over two fields: the wide bodies plan round such corridors and
  heroes pass them.
  The route is A* over the lattice, eight-connected, never cutting a blocked
  corner, a node open when its room covers the body's collision size and an
  8-unit margin, in scratch kept between searches under an epoch stamp, with a
  budget of expansions past which the walk goes to the nearest node reached. A
  walker the margin shuts in, no node with room for it at either end or every
  node the search got to expanded without meeting the goal, has the route laid
  again at its bare collision size: a hero walks at its own size into gaps
  between trees narrower than its size and the margin, and a route laid only at
  the margin finds no way out of them. The
  corners found are pulled straight against the exact test, then each drawn in
  along the bisector of its legs by binary search as far as both legs stay clear,
  then pulled again, so they land on the tangents of what they round to within a
  unit. A goal that cannot be stood on ends the route at the first node with room
  on the walker's own side of it, or at the goal itself when that lies in a
  straight line from the node. A route is kept while its goal drifts under 128
  units, its last leg swung onto a goal that moved a little, its corners dropped
  as they are passed, beside one or beyond it along the next leg with that leg
  clear, and laid again from where the walker stands when the way to its next
  corner is shut. Lane routes are laid for the widest marcher.
  The plan is the next stretch of the walk, tick by tick, laid by A* over states
  of spot, facing and tick, at most 40 ticks ahead. A step of the search is a
  stretch of four ticks along one of sixteen headings or straight at the aim,
  the turn onto it paid first in ticks stood still exactly as the walk would
  stand them, or four ticks stood waiting for a body to pass, or the last few
  ticks straight at the aim. The cost is time; a tie falls to the state nearer
  the aim, then to fewer turns. Headings more than five of the sixteen off the
  way to the aim are not tried: a way back is the route's to find. Bodies within
  the walker's reach over the horizon and their own are foreseen: by their own
  plans where a body planned earlier in the tick, else by their last step carried
  forward twelve ticks and held; a stretch that would bring the body within the
  two collision sizes of a foreseen body at any tick is not taken. The search
  expands at most 120 states and settles for the state nearest the aim, and
  answers nothing when that is under two steps nearer. The aim is a spot up to
  440 units along the route, as far as a straight line from the walker stays
  clear; on the last stretch the route's end, with the order's own arrival: an
  attack's reach, the touching distance of a follow, nothing for a walk. A walk
  with a reach is judged against the destination itself rather than the spot
  the route ends on beside it: a tower's centre cannot be stood on, but its
  reach is measured from there.
  Steering (swings off the line, a side held until the way ahead cleared, slides
  along a graze) was rejected: it pressed walkers into crowds, wiggled them at
  walls, and could not see a body coming. A search over time finds the way round
  a moving body before contact and the wait that lets it pass, and its cost is
  the walk's own turn rule, so what it plans is what happens.
  A hero is foreseen only where it stands, and only once it has stood there
  eight ticks or has been run into six times in three seconds: it goes where a
  player sends it next, which nothing here knows. A hero on the move is met
  when it is met.
  A plan is walked a step a tick. Each step is checked against the bodies as
  they now stand: one that has come to stand in the step refuses it, and one
  that was itself moving costs a block wait of eight ticks stood still, after
  which the way straight on is tried again. That is what keeps creep blocking:
  a creep a hero keeps stepping in front of runs into it, stands, tries again
  and is held to the hero's pace, while the creeps the hero does not cover pass
  by its sides, and a hero that stands still is flowed round. A creep run into
  a hero six times in three seconds plans round where the hero stands, so a
  hero merely walking up the lane ahead of its wave does not hold it for ever. A plan is laid again when its route goal has moved 64
  units, the body's speed changed, the body was put somewhere else, fewer than
  12 ticks of it are left short of the route's end, or a step was refused, and
  not oftener than every four ticks, or sixteen once the body has stood stalled
  a second. A body that stands this tick, held, swinging, in reach of what it
  fights or with nowhere to go, forgets its plan, so nobody foresees a fighting
  creep walking off and plans straight through it. A plan that falls short of
  its aim with bodies about lays the route again at once round the bodies
  standing there as if they were structures, within a small budget and not
  oftener than every 48 ticks: a wall of them is too wide for the plan's own
  budget to find its way round. What the body can walk is judged at its own
  size, not the route's margin, or a body pressed against a tower sees no corner
  along it and lays its route again every tick. A creep stalled thirty ticks
  walks into whatever stands in its way and is eased out after.
  Coming round costs whole ticks, so a turn a little short of the way is the
  faster start: a walk sent straight back sets off a heading short of the
  reverse and comes onto the way as it goes, a curve of a few tens of units,
  which the search finds on its own.
- A creep is on its lane route as on a rail: it aims at the next waypoint it has
  not passed, a waypoint passed once the creep stands within 250 units of it or
  beside or beyond it along the leg to the next with the leg clear, and it
  rejoins the rail there after a chase or a push, never at where it left, which
  would walk waves backwards after every chase.
- A lane creep has no leash of its own: a chase ends `CREEP_CHASE_TICKS` after its
  target was last in attack range, a target lost from sight is walked after to where
  it was last seen, and then the creep takes up the rail again.
- Units turn at a finite rate and only walk or swing once they face their current
  path leg, so corners cost time. Buildings do not turn.

Every swing ends in a backswing the unit stands through, which is the
pause a creep makes over its kill before marching on. A hero's order cancels its
backswing.

Abilities run on a shared engine — the table in `game/config/ability.rs`, casting in
`game/systems/cast.rs`, the body's action in `game/systems/actions.rs`: a row of slots per hero —
four for most, six for Shadow Fiend, the row is per-hero data — each slot with a
level and a cooldown, held on the seat like items, so both survive the
hero's death — and cooldowns keep running while it is dead. A skill point arrives
with every hero level; basic ability level k needs hero level 2k-1, ultimate
levels open at 6, 12 and 18, as in the game. Slots may share a level: `learn_group` folds a family
of ids into one, a point into any of them levels the whole family, and the point
accounting counts the family once. The razes are the one family so far. A cast
order is validated (learned, off cooldown, not disabled, mana, target kind) and
waits as a `PendingCast`, walking the caster into reach when it has to. It then
runs as the body's action: the cast point (`ActionPhase::Before`), the ability's
own `duration` (`During`, which is how Dismember channels) and the backswing
(`After`), all from `AbilityDef`; a cast point of zero goes off in the tick it
starts. A cast point roots the caster and is shown as `StatusFlags::CASTING`; any
order to the body breaks it off before anything is spent, as in Dota. An
`immediate` ability (Feast of Souls, Dota's `DOTA_ABILITY_BEHAVIOR_IMMEDIATE`) has no
cast point, backswing or duration, and ordering it breaks nothing off: it goes off
beside a swing or another cast point. Cooldowns tick in the upkeep before actions run, so a fresh cooldown
surfaces at its full value. Sylla's kit: slot 0 a critical strike
passive fed by the hidden per-unit `Chance` stream (the stream is keyed by the
entity slot, so respawning continues the sequence); slot 1 an attack speed
self-buff for its duration; slot 2 a magical
projectile that bounces to the closest unhit enemy in range, never a structure;
slot 3 an ultimate volley launching an attack projectile at every enemy unit in
its radius. A crit rolls once at windup completion and rides the projectile,
reported only in the `Damaged` event.

Shadow Fiend carries Dota 7.41f's six slots: three razes, Feast of Souls, Presence of
the Dark Lord and Requiem of Souls; the slot row is per-hero data, so squeezing him
into four slots buys nothing. The three razes share one level, which is Dota's rule and
what keeps the trio from costing twelve points: a point into any of them levels all
three, and the spent-point sum counts the trio once. Necromastery is his innate
(`HeroDef::innate`): in no slot, never learned, gathering from his first level, as
Dota's hidden innate is. Presence is a leveled
aura on the enemy: each tick it lays an armor-break modifier with a short linger on
everything hostile in reach, and the stats pass reads that modifier like any other. It does
not go through the `Auras` component, which is static body data handed out to its own
side; a presence is as strong as its learned level, which a `&'static [Aura]` cannot say.

### What a tower and a flagbearer hand out

Both are Dota's, and the numbers are the wiki's rather than anybody's memory. A tower's
**Tower Protection** reaches 900 and adds 3 armor and 1 health a second at tier one, 5
and 3 at every tier above it — so both figures are tables indexed by tier, not the flat
numbers a first reading of them gives. It reaches **allied heroes only**; Dota adds
creep-heroes and illusions, of which this game has neither. A **flagbearer**'s
inspiration reaches 700, mends 3 a second, and reaches the **non-hero units of its own
side** — no heroes since 7.40, and no wards or buildings — adding 4 % magic resistance
for every upgrade interval the flagbearer spawned after, at most 15. Both linger half a second, which is what the `ticks` on an `Aura`
buys, and neither stacks, because `Modifiers::put` keeps one modifier of a kind.

`Aura` has a `Reach` because the fountain hands out to the whole of its own side, the
tower to its heroes alone and the flagbearer to everything but heroes. The magic
resistance that grows with the bearer's upgrades is not a second aura or a per-entity
copy of the static one: `ModifierKind::grown` turns the def's base effect into what a
bearer of so many intervals hands out, at the one place auras are handed out, so the
`Aura` table stays `'static` and nothing else learns about upgrades. `Guarded` and `Inspired`
are separate kinds even though a body could carry both, because naming them apart is what
lets a client tell a tower's doing from a flagbearer's, and what keeps the bot from
reading either as the mending a salve puts on.

What a flagbearer's death pays the enemy heroes around it follows 7.40: the bonus is set
as it spawns — ten and one draw of its own gold range, kept on its `Bounty` — and a
death that is not a deny pays it to every enemy hero within 1500 and to the killer
wherever it stands, each through its own gold income. Drawing it at spawn is Dota's
order, and it is why the spawn, not the kill, spends that draw of the bounty stream. The
killer's share rides in the `Died` event's `gold` with its own bounty; the others' is
read off their seats.

Two more places fall short of the wiki, and neither is reachable on the maps as they
stand. Several auras are not meant to stack, and they do not — but which one holds is
whichever was handed out last rather than the strongest, since `Modifiers::put` keeps one
modifier of a kind and the last writer wins. Two towers would have to stand within 1800 of
each other for it to show, and the nearest pair on the Dota map is 2239 apart. And a
tower's protection is meant to pass an invulnerable hero by only when it is *hidden*;
nothing here checks that, and no hero in the game can hide.

**An effect id is a wire vocabulary that lives in neither crate.** The server hands them
out in `project.rs`, the client reads them in `catalog.rs` and the bot names the few it
weighs in `numbers.rs`, and nothing holds the lists together: a new effect leaves the
client drawing `?` chips while every test passes. By the membership rule the ids cross
the wire and are needed to read it, so they belong in `bota-proto` — which is where the
next one added should put them.

A soul is taken only from what he brings down himself, which is the killer `bury`
already carries, and a hero is worth four where anything else is worth one, up to
twenty. Souls wait on the seat in `Kept` with the abilities and the items, less the
`SOULS_LOST_ON_DEATH_PCT` (30%, rounded down) that his own death lets go. The requiem
reads the souls and keeps them; its cooldown is what limits it. Feast of Souls is the
one thing that carries the count past twenty, and only for a while (Shadow Fiend at
Dota 7.41f, below).

Souls are not a component of their own. Necromastery and Flesh Heap are the same shape -
a count that grows on a death, never runs out, and survives the body - so both are one
`Stacks` component keyed by `StackKind`, and a third of them costs a variant rather than
a table, a field in `StatsCx`, a line in the hash and a line in the projection. The wire
follows: an `EffectView` carries `ticks_left` for one that runs out and `stacks` for one
that is counted, and a gathered count travels as an ordinary effect with its own
`EffectId`. A `souls: u32` on `UnitView` was rejected: it puts one hero's vocabulary
into the shared one, says nothing about the flesh heap, which has the same shape, and
every counter after it would have to buy its own field. The two counts are separate optional fields rather than one
enum of `Ticks` or `Stacks`: an effect that stacks and also runs out is an ordinary thing
to want - Dota's Fury Swipes is one - and a sum type forecloses it in the shared
vocabulary, which is the expensive place to be wrong. The byte an effect pays for the
second `Option` buys that.

A raze takes no aim: it is fired along the caster's facing and lands at its own
distance, as in Dota. `facing_towards` is a piecewise-linear octant map and `heading_of`
inverts it exactly, so a facing round-trips without drift and the aim of a raze is
`Own`. The consequences: the cast never walks the caster in, and pointing it is done
with the body, by turning. Its 0.55 s cast point roots him without turning, so the
facing he begins it with is the one it lands along 17 ticks later; the order he was
walking or swinging on is kept and taken up again once it has gone off, and a new
order to the body inside the cast point breaks it off. Attacks and razes both wait on
facing, so it is load-bearing, not cosmetic.

A raze and a requiem line pass buildings by, as in Dota: they burn, stack, frighten and
slow every hostile unit in their reach but a structure, as the presence and the souls
do. A raze that burned towers through their zero magic resistance would teach a learning
bot to siege from 950 away — a way to push that the game it imitates does not have.

Hero roadmap (added as data + ability implementations; the engine does not change):

| Hero | Type | Abilities |
|---|---|---|
| Sylla | ranged carry | crit passive / attack speed buff / bouncing projectile / ult: multishot |
| Krag | melee tank | stun dash / cleave passive / armor aura / ult: shield + damage return |
| Vex | ranged nuker | nuke / slowing AoE / mana passive / ult: AoE burst |
| Grum | melee initiator | hook / DoT aura / slow / ult: AoE stun |
| Lira | support | heal / shield / wards / ult: team heal aura |

Built since: Pudge (hook / rot / flesh heap / ult: dismember) and Shadow Fiend
(innate necromastery / three razes sharing a level / feast of souls / presence /
ult: requiem).

Flesh Heap is the 7.28 to 7.30 passive, from the wiki's changelog: 12/14/16/18%
magic resistance multiplied with the hero's own and 1.5/2/2.5/3 strength a stack
by level, a stack for every enemy hero dying within 450. Today's game splits it
into an innate strength heap and an active damage block, Meat Shield; a passive
whose levels changed nothing left three of four points dead, and an innate
without the active half is a slot with nothing in it.

## Protocol

Frame: `u32 len (LE) | postcard payload`. The message kind is the postcard enum tag
inside the payload. TCP, `TCP_NODELAY`.

Until the first release there is no versioning and no compatibility;
version and fingerprint fields appear with the first release, when there
is something to be compatible with.

### Client → server

```rust
enum ClientMsg {
    Hello { role: Role, name: String },    // Role: Player|Bot|Spectator
    PickHero { hero: HeroId },
    SetReady(bool),
    Order { seq: u32, unit: Option<EntityId>, order: Order,    // unit: None is the hero
            seen: Option<u32> },                               // tick of the view it was given on
    Ack { tick: u32 },                     // lockstep: "I am ready for the tick"
    ViewAs { seat: Option<SlotId> },       // spectator: watch through one seat's eyes
}

enum Order {
    Move { target: Target }, Attack { target: Target },
    Cast { slot: AbilitySlot, target: Target },
    Use { slot: ItemSlot, target: Target },
    Put { slot: ItemSlot, target: Target }, Take { target: Target },
    Buy { item: ItemId }, Sell { slot: ItemSlot },
    Swap { from: ItemSlot, to: ItemSlot },
    Learn { slot: AbilitySlot },
    Cheat { cheat: Cheat },                // refused unless the match allows cheats
}

enum Target { None, Pos(Vec2), Unit(EntityId) }
```

Casting is one variant, `Cast`, with the target kind expressed by
`Target`; whether the variant fits the ability is validated by the server
(`RejectReason::WrongTargetKind`).

There is no chat in the protocol. The server exists for local bot testing; a message
type that plays no part in the match is not worth its slot in the wire format.

### Server → client

```rust
enum ServerMsg {
    Welcome { player_id: PlayerId, slot: Option<SlotId>, tick_rate: u16, mode: TickMode },
    LobbyState { slots: Vec<LobbySlot> },
    MatchStart { info: MatchInfo },        // the type has no seed field
    Snapshot { view: WorldView },          // whole; the tick is inside the view itself
    Events { tick: u32, events: Vec<EventKind> },
    OrderRejected { seq: u32, reason: RejectReason },
    Orders { tick: u32, orders: Vec<SlotOrder> },  // to a spectator viewing as a seat
    MatchOver { winner: Team, stats: MatchStats },
    ParticipantLeft { player_id: PlayerId, slot: Option<SlotId> },
}
```

A Player/Bot receives a whole `WorldView` on every tick, filtered through its team's
fog. A live Spectator receives the same stream unfiltered. There are no deltas: a
client can start rendering from any snapshot, so joining mid-match requires nothing
special. A `bota-proto` size test budgets a 1v1 snapshot of 25 units at 4 KiB;
`diff`/`apply` will arrive together with 5v5.

Within a tick the order on the wire is fixed: `Snapshot(t)`, then `Events(t)`. An
event may name an entity the snapshot no longer carries — it died on that very tick;
the previous snapshot is what to render such an event against.

A slow consumer cannot stall the simulation: the tick thread never blocks on a
socket. Snapshots are coalescible by construction — each one is whole, so a
connection that fell behind is sent only the latest. Events cannot be skipped, so a
connection whose event queue overflows is closed instead of buffered without bound.

The vision mask does **not** go over the wire. The server decides which entities a side
is told of — sight lines in `game/vision.rs`, who sees what each tick in
`game/systems/visibility.rs` — and the client draws its own fog from what it already
holds: the terrain and `opaque_cells` from `MatchStart`, and the positions and
`vision_radius` of its own units, walking the same sight lines.

There is no shared implementation in `proto`, and that is not an omission. The sides
need different things: the server — an exact answer to "does this team see this
point", the client — a soft gradient for rendering with fade-out and a memory of the
explored, the bot often nothing at all. Since terrain occludes, the computation is a
game rule, and rules do not belong in `proto`: the server's copy is the rule, the
client's a picture of it. Each side keeps its own mask type and grid constants — they
do not cross the wire and are not needed to read it.

The client's picture needs every source of vision to be an entity in the view. Wards
already satisfy this; an ability granting vision without a unit must be modeled as an
invisible source entity.

Divergence of the computations is safe: the server alone decides which units enter the
view, so a bug in the client's mask paints the ground a wrong shade and reveals
nothing.

A replay is a self-contained recording that plays back without a server. A `.brp`
file is a sequence of length-prefixed frames, framed exactly like the socket, each
holding a `ReplayRecord`:

```rust
enum ReplayRecord {
    Msg(ServerMsg),                                       // the fogless spectator stream
    Orders { tick: u32, orders: Vec<SlotOrder> },         // what every seat asked for
}
```

The server records `MatchStart`, then per tick the fogless `Snapshot`, the `Events`
and the orders it accepted. `ReplayRecord` lives in `bota-proto`: a replay file is a
wire, and the client must read it. Replay mode in the client is reading frames from
the file instead of the socket; the order records can be skipped, or rendered — which
is what makes a replay more useful than a live spectator seat when debugging a bot.
Nothing in the file is re-simulated: a replay is coupled to the wire format only,
survives balance changes, and grants the client nothing beyond what a live spectator
already gets. Snapshots are whole, so rendering can resume from any point of the
file. Archiving is an external compressor's job, not the protocol's.

### Bounded replay playback in the client

Opening a replay does not depend on its length. Reading the whole file into the
socket `FrameReader` and decoding every record before the first render costs quadratic
copying (each decoded prefix moves the remaining tail) and holds every snapshot while
the window stays black.

`replay_play.rs` opens the file without reading records. A client-local buffered reader
reads the little-endian length prefix and calls `decode_payload` one record at a time,
with one record of lookahead; the codec, the replay format and the server are
untouched. There is no file-sized buffer, decoded-frame queue, background producer or
replay index, and so no backpressure or thread shutdown to get right.

Each GUI-frame poll is bounded three ways: `MAX_RECORDS_PER_POLL` (64) records,
`MAX_BYTES_PER_POLL` (256 KiB) of prefix and payload bytes and `MAX_READS_PER_POLL`
(128) reader calls, interruptions included; read-ahead is at most
`REPLAY_BUFFER_CAPACITY` (8 KiB). `MAX_PAYLOAD_LEN` (4 MiB) is checked before
allocating. A payload may span polls, and a poll's decoded content is bounded by one
maximum payload plus its byte budget. These bound work and storage, not wall-clock time
of a synchronous read.

The first snapshot is a render boundary: startup stops there, and the playback clock
anchors to its tick, so neither loading nor drawing the first frame counts as game
time. Later time runs at the recorded tick rate and the chosen speed. When a poll's
budget runs out the target clock stays put while later frames finish the work, so no
catch-up debt builds. Pause stops the clock; a step or a forward jump only moves the
target clock, clamped to the last tick on the wire, and the next poll does the reading.
There is no backward seek.

Records keep file order. `ReplayRecord::Orders` becomes `ServerMsg::Orders` in place,
so orders reach the overlay with their snapshot and events. Clean EOF is told apart from
an empty recording, a torn prefix or payload, an invalid length, a decode failure and a
read failure; a failure keeps the record number and byte offset, stops reading, and
leaves what was decoded on screen under the error. EOF alone invents no match result.

The ignored `both_real_replays_stream_correctly_with_bounded_prefix` test streams the
files named by `BOTA_REPLAY_NEURAL` and `BOTA_REPLAY_TEACHER` and checks every message
against a sequential reader.

### What a slot is worth, and who works it out

Everything a slot of the panel shows about the thing in it rides in that unit's
own view, already worked out by the server: the range of the next cast, its
mana, whether a toggle is on, whether a skill point could go into it. None of
it is a number the client looks up. The reason is that none of these are
properties of a catalog entry: a range moves with the level, and once an item
or a talent moves it, two units holding the same ability hold two different
ranges.

The one thing that belongs to no unit is what the shop asks for a thing nobody
holds yet, so `MatchInfo` carries the price and the parts of every item once
per match, and the client prices a purchase against what the seat holds by the
same rule the server charges by: only what the seat bought itself and has not
marked for sale counts, which is why `ItemView` names its owner. `MatchInfo` also
carries both fountains and the shop range, so the client knows where the home shop
is on any map. The client's own catalog is what is left over
after that: names, blurbs and art, and not one number.

`can_level` is a bit rather than a rule: the server answers it from `level_floor` and
the points spent, and answering it once is cheaper than keeping two copies honest.

`Aim` names a fourth and fifth kind of target beyond nothing, a point and a
unit: a tree, and a spot within reach of an allied building. Both cross the
wire as a point — which one was meant is settled where the use is carried out —
but the client has to know which it is to draw the right thing under the
cursor, and saying so is one field where the alternative was a second boolean
beside the aim and a hardcoded item id in the renderer.

### Pressing a slot

A press is sent whatever state the slot is in. A passive, an ability with no
points in it, one on cooldown, an item with no charges — all of them reach the
server, and the server answers with the reason. The client gates nothing,
because every gate it could hold is a copy of a rule that already lives in
`validate_order`, and a copy that drifts turns into a press that vanishes with
no answer at all.

That puts one requirement on the server: everything `use_item` and the cast
system can refuse silently has to be named in `validate_order` first.
Otherwise the order is accepted, quietly does nothing, and the client — which
no longer holds an opinion — has nothing to show.

Both a key and a click on the box go through one function, and what a press
means is decided by a pure one that takes only the facts that settle it: which
slot, whether control is down, what is in the slot and how it is aimed, what
is already taken up, and who is commanded. Two entry points with rules of their
own drift apart: a click and its key would answer differently.

How a slot is drawn is a set of independent answers rather than one state,
because the states genuinely combine: what has no points in it is unlearned
and unusable at once, and a passive may still sit on a cooldown. Unlearned is
drawn darker than merely unusable, so the two never read as one another, and a
toggle that is on is framed in a colour the aiming frame does not use, since
that one is already the colour of a selection.

### Picking, and what carries the camera

One rule for everything that stands. A hero of one's own, an enemy hero, a
courier, a creep, a building — a click picks it, a shortcut picks it, and what
is picked is what the panel shows. There is no separate notion of picking a
seat: a seat is reached through the hero standing in it, and the panel finds
the seat behind whatever hero is picked. What is picked and what a player may
order are two questions, not one: anything at all may be looked at, and only
what this seat drives answers to the keys.

Picking never moves the camera. Reaching for the same thing twice does, and
that is the only thing a pair means: F1 twice pins the camera to one's own
hero, F2 twice to the courier, two clicks on a unit to that unit. A pair is
spent when it is made, so three presses are one pair and one single rather
than two pairs.

The camera is free otherwise. Driving it by hand — the arrows, the edge of the
screen, a click on the minimap — lets go of whatever it was pinned to, because
asking to look elsewhere is asking to stop being carried. The one exception is
the start of a match: the first hero to stand is pinned once, so a match does
not open on an empty middle of the map.

A pin holds a seat rather than a body when what it holds is a hero. A hero
that dies comes back as a new entity, and a camera pinned to the body would be
left behind by the first death and never find its way back without being told
to again.

W, A, S and D pan only for a seat with no hero to give orders to. Once there
is one, those letters are orders — A is attack-move, S is stop — and the
arrows are what is left to drive the camera by hand.

### Gold drawn per kill

A lane creep's or a neutral's gold is a `GoldRange`, and each kill that pays it draws
one whole number evenly from it on `Purpose::Bounty`, a match-global stream of its own:
the order kills happen in is already deterministic, and a stream of its own leaves every
other purpose's sequence where it was. A range of one amount — a tower, a barracks, a
hero's head — draws nothing, so only what Dota randomises spends draws. Upgrades move
both ends of the range (and add their experience) once, as the creep spawns, which is
also when Dota fixes them. A tower adds a team bounty that does not depend on who struck
last, so it is a field of its own on the `Bounty` rather than a second range. The midpoints the ranges replace paid ranged creeps 47
against a mean of 47.5, and paid a flagbearer and every upgraded creep as if it had
never been upgraded: `Growth::gold` and `Growth::xp` were computed and dropped.

A swing's damage is drawn the same way on `Purpose::DamageRoll`, at the moment it
strikes: the least damage the stats carry plus an even draw from `0..=damage_spread`.
The view keeps telling the middle of the range, so `UnitView::attack_damage` means what
it meant when every swing dealt that middle. Sylla and Pudge keep a fixed damage: neither
is Dota's hero of today.

### Strike sight and creeps drawn from the fog

An attack that strikes, and a spell that goes off at a unit within 1800, shows the side
struck at a 150-radius circle around the striker for two seconds (Dota's caster vision).
The circle is an `Exposure` on the striker — one `ExposureCell` per side, so striking
again moves it rather than piling circles up — and the sight pass reads each live cell
as one more viewer of that side. A viewer is the one thing sight already knows how to
answer, so the view, the creeps' targeting and `can_see_point` all see the striker
without being told about strikes. A strike made while the last circle still shows sees
past height and trees, as Dota upgrades a repeated cell to flying sight. The circle
starts showing at the next sight pass, the tick after the strike, and shows for
`STRIKE_REVEAL_TICKS` passes.

Creeps answer a hero going for a hero in two ways (CREEPS.md §1.5). An attack order
draws them whether or not they see the orderer, and the hold it puts on them is kept
through the fog: `select_target` keeps a held target that is only out of sight
(`hostile_unseen`), and the creep walks at where it stands until the hold runs out or
the strike shows it. A swing begun at a hero draws the creeps within 500 that see the
swinger, holding nobody. Liquipedia does not say what a creep that cannot see the
swinger does; drawing it would hand it a target that the very next tick's ranking
drops as unseen, so gating on sight says the same thing without the churn. Together
with strike sight, an archer on high ground draws nobody with its first swing, is shown
by its first strike, and draws the creeps with every swing after.

### Attack classes

Dota's attack classes (CREEPS.md §1.1) are a factor on the swing, taken in `strike`
when the blow or the missile is made: the attacker and the target are both known there,
a missile carries the result, and a crit and the magical pierce riding on the swing are
scaled with it, as Dota scales every conditional attack bonus. The class is read off the
`UnitKind`, not stored on the `UnitDef`: every kind maps to exactly one class (super and
mega creeps share their kinds' classes), so a table field would be a second place to
keep the same fact. Spells take no factor.

### Looking at what is not there

Two different things are not there, and they are answered in two different
places.

A hero that has fallen leaves its abilities and its items on the seat: both
outlive the body, and both are gone from the wire the moment the body is,
since they only ever rode in a `UnitView`. `PlayerView` carries them while no
body stands, under the same rule the stash goes by — told to its own side and
to nobody else. That side is the one that can act on it, and telling the other
side what a dead enemy bought while dead would be telling it more than it saw.

An enemy in the fog is the other case, and the server cannot help with it at
all: what a side may not see is what a side is not told, and that is the one
rule the whole projection is built on. So the client keeps the last state it
saw of every unit a seat owns — a handful of entries, since a seat stands in
one body at a time and a body left behind is dropped when the next one
appears. What is shown of a unit out of sight is that memory, marked with how
long ago it was true.

That memory is also the handle a fallen seat is picked by. A seat with no hero
standing has no unit on the wire, so there is nothing to name it with; the
body it left is the name. This is why picking stayed a unit and did not grow a
second form for seats: the body outlives the hero in the client's memory just
as the kit outlives it on the seat.

The panel and the popups over it read one answer, worked out once: which body
is being shown, what a fallen one left behind, and how stale it is. Two
readings of that question would drift, and the popup over a slot would end up
describing a different item from the one drawn in it.

### Whom units fight

Every unit that fights of its own accord holds a target, and a lane creep holds
timers on it: the hold an attack order puts on it, the chase it keeps up after
a target that left its reach; a hero holds the wait before its attack order
may draw creeps again, and a tower the wait before an order may draw it.
A player reads all of it off the screen — the creep turns, walks and swings at
somebody, and the click that drew it was the player's own or happened in
plain view — so a bot is told it too, as `WorldView::aggro`: one `AggroView`
for every carried unit that has anything to say, sorted by `EntityId`.

| Field | Wire type | Meaning |
|---|---|---|
| `unit` | `EntityId` | the unit described; always one of `units` |
| `target` | `Option<EntityId>` | whom it is set on: what it attacks or walks up to attack |
| `swing` | `Option<SwingView { target: EntityId, strike_in: u32 }>` | the swing in its windup, and in how many ticks it strikes at the current attack speed (at least 1) |
| `hold_left` | `u32` | ticks a lane creep keeps its target whatever else turns up, after an attack order drew it |
| `chase_left` | `u32` | ticks a lane creep keeps after a target out of its attack range while nothing else is within its acquisition; full while the target is in reach, zero when set on nobody |
| `draw_cooldown` | `u32` | a hero, to its own side only: ticks before its attack order may draw lane creeps again; a tower: ticks before an enemy hero's order may draw it again; zero for anything else |
| `returning` | `bool` | a neutral walks back to its camp and takes nothing on |

A unit without an entry is set on nobody, is not swinging and holds no timer.

The fog rule is the projection's own: an id is named only when the unit it
names is in the same view's `units`. A target, or a swing's target, that the
viewer is not told of is sent as absent — the same absent as having no target
at all, so a creep chasing into the fog says that it chases (its timers stay)
and never whom. A debug assertion in the projection checks every id against
`units`.

Not carried: whether a lane creep may be drawn yet before
`FREE_AGGRO_TICK` (that is the public rule plus positions the view already
holds, and answering it per creep costs a scan of the world per creep), and
a neutral's leash and re-aggro counters (a player sees only whether it walks
home).

The list is the last field of `WorldView`, and `Snapshot { view }` is the last
thing in its frame, so the list is the tail of the payload. A bot outside this
repository reads this wire; a decoder that predates the list stops before it,
and postcard's `from_bytes` leaves trailing bytes unread, so it keeps working
unchanged. `bota-proto`'s `compat` test decodes a fresh snapshot through the
old shape. A decoder that insists on consuming every byte of a frame does
not tolerate it.

### Tick modes

- `Realtime` — 30 Hz on the wall clock, a late command applies on the next tick.
- `Lockstep` — the server waits for `Ack(tick)` from every agent. A bot thinks as long
  as it needs, the match reproduces bit-for-bit. `--ack-timeout-ticks` guards against a
  hung bot (empty order).

### Slow receivers

A connection's outbox never blocks the simulation. A tick (its snapshot and its events)
is one coalescible entry: queueing a new tick replaces a tick still waiting, and the
replaced tick's events ride along in the new snapshot's `WorldView::skipped_ticks`, with
their own ticks, one entry for every tick skipped. A slow receiver therefore jumps from
state to state but sees every tick and every event it may know, in order, and a
snapshot is always followed by its own `Events`. The snapshot frames are encoded once
per side; only a snapshot that carries skipped ticks is decoded and encoded again, on
the connection's writer thread. More than `MAX_SKIPPED_TICKS` (900) skipped in a row,
like an overflowing queue, closes the connection.

The outbox only skips once the kernel stops taking bytes, so each connection's send
buffer is capped at `SEND_BUFFER_BYTES` (16 KiB, about four 3.3-3.8 KB Map2 snapshots,
socket2): with Linux's autotuned buffers a stalled reader otherwise trails by tens of
seconds of queued states before the first skip, which is the opposite of what a realtime
player needs. The receiver's own buffer still holds what its kernel accepted before the
stall (128 KiB by default, about a second of Map2 snapshots). A reader keeping up, on
loopback or a LAN, never fills either and is never skipped.

### Order delay

An order given on the view of tick `T` takes effect on tick `T + 1` at the earliest.
Over a network it lands later: the round trip, the wait for the next tick boundary and
the client's own thinking add `d` ticks. Dota runs 30 ticks a second and its players
see 20-120 ms of ping, so `d` is 0-4 ticks there, more on a bad line. Lockstep alone
has `d = 0`, so a bot trained and measured in lockstep would only ever meet the delay
when it plays for real.

- `--order-delay-ticks` holds every order of a seat back by a fixed number of ticks;
  `--order-jitter-ticks` adds `0..=j` more per order, drawn from a stream of its own
  per seat (`MatchRng::for_order_delay`) that no simulation stream shares. Each takes
  one value for every seat or one per seat. Base and jitter together stay within
  `MAX_ORDER_DELAY_TICKS` (30, a second). Both work in either mode; in realtime they
  add to the network's own.
- Orders never overtake one another, as on one TCP stream: an order drawn earlier
  than the one before it waits for it, and a later one due on the same tick
  replaces it, as two orders inside one tick always have.
- An order is validated when it arrives and again on the tick it takes effect; one
  that no longer holds then is answered with `OrderRejected` on that tick. With no
  delay both checks see the same world, so the match is the one it always was.
- The delay line lives outside `World`: the world still takes the tick's commands,
  the replay records them on the tick they took effect, and neither fingerprints nor
  replays change.
- A client says which view an order was given on (`Order::seen`); the server tells
  each side, in `WorldView::order_delays`, how late each of its seats' latest such
  order took effect beyond `T + 1`. One number serves both modes: in lockstep it is
  the configured delay, in realtime whatever the line and the client cost. An order
  without `seen` measures nothing.

## Server algorithm

```
main:
  parse args (--port, --mode realtime|lockstep, --tick-rate, --players,
              --replay out.brp, --map, --seed, --ack-timeout-ticks, --cheats,
              --immortal-structures, --immortal-heroes, --dummy-seat,
              --order-delay-ticks, --order-jitter-ticks)
  listen TCP; the accept thread queues connections
  state = Lobby

Lobby:
  Hello → Welcome + LobbyState
  PickHero / SetReady; when every slot is ready:
    world = World::for_match(&cfg, cfg.rng());
    broadcast MatchStart { info: cfg.info() }; state = Playing

Playing (simulation thread):
  loop {
    // 1. gather input
    realtime: until tick_start + 1/rate
    lockstep: until every seat has acked the tick (or the ack timeout)
    each Order, on arrival: PlayerId → SlotId through Roster, then
      world.validate_order(slot, unit, &order)
        Ok  → onto the seat's delay line, due on tick + 1 + its delay; replaces
              an order due on the same tick
        Err → OrderRejected { seq, reason }
    // 2. the orders due on the next tick are validated again, in slot order,
    //    become Commands and go into the replay; one that fails → OrderRejected
    // 3. events = world.advance(&cmds)
    // 4. broadcast: a seat gets its team's view, with its seats' measured order
    //    delays, and its events as one tick (see Slow receivers); a spectator
    //    the fogless view, or a
    //    seat's view and that seat's Orders after ViewAs; events go out by
    //    Event.visible_to. The fogless frames and all events go into the replay
    // 5. if let Some(winner) = world.victor() { broadcast MatchOver; break }
  }
  close every connection and wait for its writer
```

### Order inside a tick

`advance` takes the tick's commands first; `step` in `game/world.rs` then runs the
systems in the order written there. Changing it moves the pinned fingerprints; replays
are frames and do not notice.

```
tick += 1
upkeep:      waves, camps, gear, assemble bags, passive gold, respawns, couriers,
             item handling, sales, expiries
effects:     modifiers, hooks, requiem lines, trees, presence, auras
stats:       derive stats, guard structures
targeting
movement:    jungle, march lanes, walk, push apart
visibility
actions:     attack orders, regen, actions, missiles, bounces
damage:      hits, break on blows, rouse camps, credit damage, events and misses,
             bury (deaths and the victor)
then:        applied modifiers count down
```

## The tick's hot path

The tick is what a trainer pays for, so it is kept free of passes that walk everything
for every asker. Each shortcut below gives exactly the answer the full scan gave.

- Targeting (`Candidates` in `systems/target.rs`), the sight pass
  (`systems/visibility.rs`) and auras (`systems/aura.rs`) sort what stands by x once
  per pass into a `Spots<T>` (`game/spots.rs`), and each query walks only the square
  its reach spans, then runs the unchanged exact checks. This is exact because the
  target answer is the unique minimum of a key that ends in the entity, who sees a row
  does not depend on the order viewers reach it, and each aura gives each entity its
  effect once, in source order. Nothing moves between laying the spots and asking
  them inside one pass.
- `World::of_wire` resolves a wire handle through `EntityAllocator::resolve` in
  constant time instead of scanning the live entities.
- The forest keeps its felled trees in a sorted list of their own, so views and ticks
  do not walk every tree of the map to find the few that are down.
- A view is projected in one pass over the entities, deciding once per entity whether
  the side is told of it.
- `trainer/map2_skirmish` in `benches/dummy` measures what a trainer pays per tick:
  `advance` and both sides' views.

## bota-bot

A bot that plays by rules rather than by weights, and the seam anything else would hold
a seat through. `Bot` is that seam — seated, match started, one tick in and at most one
`Ask` out, events, refusals, the end — and `play` is the loop that carries it over a
socket, written once so a second bot does not write it again. `Playbook` is the bot that
ships: `Field` reads a snapshot into a settled shape, `decide` walks a fixed list of
wants top to bottom and takes the first that answers. The whole crate is `bota-proto`
and `clap`. Float is allowed here, as it is in the client: what is recorded of a bot is
the orders it gave, not the arithmetic behind them.

**The order of the list is the whole of the judgement.** There is one order a tick, so
which want is asked first is the only priority there is: standing still while stunned,
feared, channelling or in a cast point, a skill point, a courier errand, the shop, tidying the bag, a
drink or a wand, the scroll, leaving while hurt, a spell, turning to aim one, a last hit
or a deny, waiting out a swing already under way, shaking the creeps off, pulling the
wave back, striking their hero or pushing a building behind the wave, and holding the
lane. Nothing is drawn at random, and what is remembered between ticks is only what the
wire does not carry — the attack cycle, recent health, who struck the hero, how long the
stash has been waiting, the scroll's shared wait — so the same match played twice goes
the same way.

**What an ability is comes off the wire; what it does does not.** `AbilityView` carries
the level, the wait, the cost, the reach and how a cast is aimed, so nothing mirrors the
ability table by hand. What an ability *means* does not cross the wire, so there is a
file per hero: `fiend.rs` decides when a raze is worth its mana and `sylla.rs` when a
bolt is. `MatchInfo` carries the shop, so `Stall` reads costs and components off it and
only the item and ability **numbers** the bot uses are written out in `numbers.rs`,
because a salve and a ward are one shape on the wire.

**A raze is aimed by looking, not by pointing.** It lands at its own reach along the line
the caster already faces, so casting one is two decisions. `fiend_spell` works out where
each raze would land from the facing in the snapshot — the server's own octant geometry,
integer for integer, in `aim.rs` — and casts only if something worth burning is standing
there, counting the souls held and forecasting creep health over the 17-tick cast point.
`fiend_aim` is the other half: a mark inside the union of the three burns but off
the line gets a walk towards it, which is how a hero turns. The burns (reaches 200, 450
and 700, radius 250) overlap, so anything within 950 units can be razed by facing it.
The feast goes off ahead of a raze whenever an enemy hero stands within its 600, or a
wave of four creeps does and mana is to spare.

**Buying is sequential, and the list is parts.** The server charges a built item the
price of the parts the seat does not already hold, and assembles a build the moment its
parts are in the bag — so the lists name parts, and gold is spent as it arrives rather
than saved up for the whole. `next_buy` stops at the first thing not owned rather than
skipping to whatever is affordable: skipping spends on the tail of a list the gold its
head was saving for. What is already inside a build counts as owned, worked out by
walking the components the wire gave. Nothing goes on a list that no want reaches for:
an item no rung ever names is gold and a working slot spent for nothing.

**The consumables sit at the head of every list, and that is the restocking.** A stack
that is drunk stops being held, so the next walk of the list finds it wanting again and
buys another. The one exception to stopping at the first thing not owned is a
consumable with no working slot to go in — what holds that up is room and not gold, so
it is stepped over. `SPARE_SLOTS` is the working slots kept clear of drink beyond the one
it takes; at nought a consumable is bought whenever a working slot is free (at one the
bot stopped restocking once five items were held, and the one scroll spent was never
replaced). `tidy` moves anything stranded in the backpack forward, since a build lands
in the lowest slot any of its parts came out of and a part bought with the bag full
comes out of the backpack.

What follows was found by playing rather than by reading, and each is now a test or a
named constant.

**An order is an animation cancel.** The holding spot drifts a little every tick, and a
fresh `Move` every tick threw away the swing already winding up — fifteen ticks of
wind-up against a want resent every eight. So a walk is ordered only when the spot is
more than `HOLD_SLACK` off, and a swing already ordered holds the seat silent until it
lands or `SWING_PATIENCE` runs out. Before these rules the bot took two creeps in nine
thousand ticks; after them, fifteen.

**Stand at the edge of your own reach.** Reckoned off its own creeps, which walk into the
enemy wave, the holding spot walked the hero in with them to take blows for nothing. It
is reckoned off the *nearest of theirs*, at the hero's own reach less `SWING_EDGE`.

**Swing at the worn one, not the near one.** The nearest creep in reach changes whenever
the wave shuffles, and a swing that keeps changing its mind never lands. A body only ever
loses health, so the lowest is the same one next tick.

**A wave in another lane is not this hero's business.** Every wave on the map is visible,
so creeps are filtered to `NEARBY` of the hero — otherwise the bot walks off to stand
behind another lane's creeps, and "the lane is clear" means nothing.

**A tango is paid for by a tree, and a lane has none.** The forest is cleared for
`TREE_LANE_CLEAR` (450) either side of a lane's centre, so a hero where it farms never
has a tree within the 165 a tango reaches; the bot carried its tangoes up the lane and
walked to the fountain instead. Walking to the tree is now part of eating the tango, and
only to trees no further up the lane than the hero already stands.

**A mend spent on a walk to the fountain is a mend the fountain was about to make
free.** A tango gives 115 health over sixteen seconds; the fountain gives 25 a tick for
nothing. While the hero is pulling out, a mend is spent only if it would carry it back
over `MENDED_RETURN`, which a salve's 400 does and a tango's 115 usually does not; and a
hero a mend is already carrying turns round at `MENDED_RETURN` rather than
`RETURN_HEALTH`. `mind_health` settles the pulling-out flag before the drink is weighed
so the two agree.

**The lane is held by talking to the creeps, not by standing somewhere.** An attack order
does something to everybody who is not the one giving it, whether the attack ever
happens or not: aimed at an enemy hero it calls every enemy creep within
`HERO_AGGRO_RANGE` (500) of the orderer onto him for `ORDER_AGGRO_HOLD_TICKS`, and aimed at one of
your own it makes them pick again with him put last. `pull` uses the first when the wave
has been pushed more than `PULL_DRIFT` past where it should meet: standing behind its
own line, the hero is a spot their creeps must walk past that line to reach, and the
fight comes back down the lane with them. Given to one side only, it is worth four and a
half last hits to that side, at the cost of denies for both. `shake` uses the second
when `SHAKE_CREEPS` creeps are chewing on the hero, aimed at the nearest unit of its own.

Aimed at the hero itself, the shake was a free aggro drop Dota does not have — a hero is
on its own side, so `call_of` read a self-click as letting go — and `rouse_bystanders`
now turns away an order whose mark is the one who gave it. Pointing at a creep costs the
follow that pointing at a unit always costs, so the tick after a shake the hero is told
to stand, and that tick must not itself count as a shake, or the hero stands still for
good. Standing at the edge of its own reach, the bot is rarely under a wave, so `shake`
seldom fires; it is kept for what it costs, which is a tick.

**A held body is not the seat's to spend a tick on.** `StatusFlags` carries the stun, the
fear and the channel, so the ladder stops at the top for any of them: an order given
during a channel is how the channel is thrown away, which the bot was doing to its own
scroll.

**What limits the last hitting is not the swing.** Shadow Fiend lands about seven swings
in ten; what it does not do is attempt. Two ways of buying damage into that measured
worse over five seeds, read on the pair's last hits and denies:

| | pair, last hits and denies |
|---|---|
| as it stands | 76.4 |
| a Quelling Blade for both | 70.8 |
| a Quelling Blade for one side only | Shadow Fiend 41.8, from 48.0 |
| standing a hundred and sixty inside reach rather than forty | 67.0 |

Eighteen more damage against creeps widens the window a swing is worth taking in, so the
bot commits earlier and holds its target through `SWING_PATIENCE` while the creep is
taken from under it; standing closer buys blows for the reach. Neither is a knob to turn
until the ticks spent standing where the wave is dying are fewer.

**A scroll is constrained by where it is aimed, not where it is cast from.** The want
once required standing in the shop, where the bot no longer went once it had drink and a
courier. It now carries both ways, home while pulling out and back to just behind the
frontmost tower its lane holds once mended, and it sits above the walk home, since what
that walk is for is the thing a scroll does in three seconds. Its shared wait is not on
the wire, so `Playbook` counts it from the tick the last scroll was aimed.

**Mana was short because nothing was ever bought for it.** A raze costs 75
against a pool that mends about two a second. Every list carries a clarity, and a
wand — twenty charges worth fifteen of each pool — is pressed whenever the pool it fills
is missing what its charges are worth. Shadow Fiend buys a Sage's Mask rather than a
Wraith Band: over five seeds the two were a wash on the scoreboard, and the mask costs
175 to the band's 505 and leaves a working slot for the drink.

A one-in-ten trace misses whether an item is used at all: a cast is one tick and the
replacement is bought on the next. `Playbook` keeps the name of the rung that answered
and the trace (`--trace`) prints it; count those over every tick instead.

What it does not do: it will not dive a tower, leave its lane to gank, stack or pull a
camp, place a ward, or fight for a rune. It takes buildings only by walking up with its
own wave. Two bots of it play a full match to an Ancient, and the same seed run twice
gives the same numbers down to the order count.

The two heroes trade places on the scoreboard with almost every change to the ladder,
and the pair's total farm barely moves: they are the same policy on both sides of one
lane, so whichever gets a little ahead denies the other and the lead compounds. Read the
two seats added together, over several seeds; a swing in one of them alone is the lane
tipping, not the change working.

### The seam

```rust
pub trait Bot {
    fn seated(&mut self, slot: Option<SlotId>);
    fn match_started(&mut self, info: &MatchInfo);
    fn on_tick(&mut self, view: &WorldView) -> Option<Ask>;
    fn on_events(&mut self, _tick: u32, _events: &[EventKind]) {}
    fn on_reject(&mut self, _seq: u32, _reason: RejectReason) {}
    fn finished(&mut self, _winner: Team, _stats: &MatchStats) {}
}

pub fn play(bot: &mut dyn Bot, chair: &Chair) -> io::Result<Outcome>;
pub fn play_on(bot: &mut dyn Bot, link: Link, seated: Seated, chair: &Chair)
    -> io::Result<Outcome>;
```

The hero is picked when the connection is made (`Link::join`) rather than returned from
a callback: picking happens in the lobby, before there is a `MatchInfo` to decide from.

`on_events` earns its place. The attack cycle is not on the wire — a `UnitView` carries
`attack_time` but not where in the interval a unit stands — so a bot cannot tell
whether it may swing now or in forty ticks. What is on the wire is every blow that
lands: one of the bot's own says the cycle began a wind-up ago and comes round again an
interval after that. Without this the bot orders last hits it cannot take for another
second and loses them all.

### One order a tick, and saying nothing

The server keeps one order per seat per tick and the last one wins, so a want is a
single `Ask` and the policy ranks its wants rather than queueing them. Re-sending the
want already standing is not free: an order cancels the recovery after a swing and calls
the creeps onto whoever gave it. So `Steady` holds a want equal to the one in hand back
for `RESEND_TICKS`, and two walks or attack-moves to spots less than `RESEND_DRIFT` apart
count as one want — otherwise following a moving wave throws away the route the server
laid every tick.

### The courier

A courier is what makes the shopping list past the first trip worth having: anything
bought away from the shop falls into a stash the hero cannot reach from the lane. With
one, the bot buys wherever it stands — and away from the shop only while a courier of its
own is standing, because gold in hand is worth more than an item in a stash nothing is
coming for.

The errands are abilities the courier carries, so an order for one names the courier in
`ClientMsg::Order`'s `unit` and casts the slot the errand sits in, read off the courier
rather than assumed. That is why the bot answers each tick with an `Ask` — an order and a
unit — rather than an order alone.

An errand outlives the tick it was given in. Saying it again changes nothing and costs
the one order the seat has that tick: sent every tick, errands took enough of the hero's
ticks that it took a quarter more damage. So an errand already under way is not repeated
for `ERRAND_TICKS`, a safety net for an order that never arrived rather than a schedule.
A trip waits until `COURIER_BATCH` items have piled up or the first has waited
`COURIER_PATIENCE` ticks, and is held back entirely while an enemy hero is within
`FIGHT_RANGE` — a courier walks to where its owner stands, and that is what is being
shot at.

### Server-side walking and courier behaviour

An errand answers to what the courier is carrying, not only to what waits in the
stash. Sent for a stash that is empty while already holding something, it takes what it
holds on to its owner rather than flying home with it: otherwise goods it could not hand
over would be sent home by the very key meant to bring them, and ride back and forth for
ever. What an owner has no room for is carried back to the stash rather than kept aboard.

A courier brought down keeps its load. The stacks wait on the seat while the courier is
gone and come back aboard the next one, the same way a fallen hero's bag waits on the
seat. Spilling the load on the ground was turned down: the wait already prices the
death, and a bot that loses items outright learns to fear the courier rather than to use
it.

A way found round something is walked to the spot it was found for, and that spot is
what is kept beside it. Keeping the last spot asked for instead made a courier fly the
whole of a stale way to where its owner used to stand: a quarry that moves a little
every tick never moves far enough in one tick to look like a new goal. A way round
anything is found only for what walks; what flies is pointed straight at where it is
going and keeps no route.

Walking at something that cannot be struck — an ally, most often, since an order aimed
at one is how creeps are shaken off — closes until the bodies touch and stops there,
following it for as long as the order stands. Aiming at the middle of a body is aiming
at a spot inside it, which cannot be reached: the walk presses in, the pass that eases
overlapping bodies apart pushes back out, and the two read on the screen as circling.

### Hanging up

A connection is closed one half at a time: the writer shuts down the sending side and
leaves the receiving side open. Closing both while the peer still has bytes of ours in
flight resets the connection, and a reset discards what was already sent — so the peer
loses the message saying who won. The server also waits for its writer threads rather
than exiting from under them, since the last message of a match is queued at the moment
the server has nothing left to do.

## Stages

| # | Deliverable | Contents |
|---|---|---|
| 0 | ✅ workspace builds | Cargo.toml, `bota-proto`, workspace lint policy |
| 1 | ✅ `bota-proto` types | `Fixed`, `Angle`, `Vec2`, identifiers, `Order`, `EventKind`, `WorldView`, messages |
| 2 | ✅ codec | serde derive, postcard, framing, `FrameReader` |
| 3 | ✅ `bota-proto` tests | round-trip of every message, torn stream into `FrameReader`, snapshot size budget, enum tag order |
| 4 | ✅ `Fixed` arithmetic | Q16.16 ops through an intermediate `i64`, `Vec2`, `distance_squared` in raw Q32.32, tests in debug and release |
| 5 | ✅ `World` ticks | entities and component tables, units, movement, orders, creeps, towers, Ancient; `rng.rs` with streams and Ratio/Chance |
| 6 | ✅ combat | attacks, projectiles, damage, deaths, gold/xp, victory |
| 7 | ✅ server networking | lobby, both tick modes, snapshot broadcast, replay recording |
| 8 | ✅ `bota-bot` and `bota-client` | SDK + bot, bot-vs-bot match: the client — macroquad: map, units, HP bars, orders, lobby, spectating, replay playback; the bot — the `Bot` seam and `play`, a deterministic playbook for Shadow Fiend and Sylla, lockstep acks, two of it playing a match through to an Ancient |
| 9 | 🔄 heroes and determinism test | Done: abilities, levels, items, shop; Sylla, Pudge and Shadow Fiend; whole-match fingerprints pinned on every map. Not yet: runs on musl/wasm32; a mirror test that a diagonally mirrored match ends in the mirrored outcome |

## Map2: mid-only play on the full Dota map

`MapId(2)` (`SKIRMISH` in `game/config/map.rs`) is `DOTA` with its own id, mid-only
waves and its own ending; no landmark, tree, camp, terrain or blocker table is
duplicated. `lanes` stays three: tree clearance and route construction still see every
lane, and only `wave_lanes` selects mid. Setting `lanes` to one would leave trees on
the side roads and change passability and vision. Side-lane structures and the jungle
stay active.

A side loses on its `MAP2_DEATH_LIMIT`-th hero death, counted across its seats
(couriers do not count), or on its first tower lost on any lane. All deaths of a tick
are buried before the match is judged, and a tick on which both sides lose is a draw,
a tower on one side and a hero on the other included: picking a winner inside the
burial loop would make that draw depend on hit order. The Ancient is not a separate
win condition: its protection cannot open before a tower has ended the match.

`MAP2_TICK_CAP` is fifteen gameplay minutes (27,000 ticks) past the 900-tick pregame,
independent of wall-clock `tick_rate` and tick mode. Tick 27,900 runs in full and then
draws even if a side loses on it; a loss on tick 27,899 still wins. Once a winner is
set, `advance` and `step` change nothing, orders with immediate shop effects included,
and a world already at the cap seals the draw without running another tick. Map0 and
Map1 have neither the cap nor the freeze.

A draw is `Team::Neutral` in `World::victor()` and `ServerMsg::MatchOver.winner`, so
the wire needs no outcome field of its own: the `StructureDestroyed` event names the
fallen tower, and the final death counts and `MatchStats.duration` tell a life-limit
ending from the cap. A consumer reads `Neutral` as a draw, not as a loss for both
seats; the client's banner reads `NOBODY WINS`.

## Mango and stacking Shadowraze

The Mango is a chosen bota mechanic, not a claim of parity with the latest Dota patch:
its price and restoration are Dota's. The raze is Dota 7.41f's in full (Shadow Fiend
at Dota 7.41f, below).

### Mango

`ITEM_MANGO` is 42. The shop sends it as an ordinary `ShopEntry`, and a carried one
counts in `ItemView.charges`. Its numbers, in `game/config/item.rs`:

| Constant | Value |
|---|---|
| `MANGO_COST` | 65 gold for one charge |
| `MANGO_STACK_MAX` | 3 charges per slot |
| `MANGO_MANA` | 100 mana per use |
| `MANGO_HP_REGEN` | `Fixed::from_ratio(2, 5 * TICKS_PER_SECOND)` per charge per tick |

The passive is 873 raw Q16.16 health per tick per charge, 0.399627685546875 HP/s at 30
ticks/s. Each charge is quantized before multiplication, so stacks of one, two and
three add 873, 1746 and 2619 and splitting or merging stacks cannot change their sum.
The ordinary fixed-point regeneration comes that close to Dota's 0.4 HP/s without a
float, a timed effect or an extra accumulator.

A Mango has no mana cost, cooldown or range and is aimed `Aim::Own`: `Target::None`
and the user's own handle are both legal, any other target is `WrongTargetKind`. A use
consumes one charge and restores `min(100, max_mana - mana)` at once. Any positive
deficit, down to one raw unit, qualifies; full or overfull mana, a missing pool or a
nonpositive capacity is `NotReady`, so a charge is never spent for nothing, and the
all-charges restoration of Stick and Wand is not reused. `ItemDef.mana_deficit` marks
the item; its use calls `World::replenish_mana`, and `World::can_replenish_mana` is
the check both share. A use emits a mana-only `Healed` event with the whole points
restored, and none when less than one whole point was restored.

Only unmuted inventory slots 0 to 5 grant the passive or allow a use; the backpack
(6 to 8) and the stash (9 to 14) are inert, and a stack moved from the backpack into
the inventory is muted for `BACKPACK_MUTE_TICKS`. A consumed charge leaves the passive
at the next stats derivation, and the last one empties the slot. Couriers carry
charges intact and get no item bonuses; a hero's bag and a dead courier's load keep
their charges on the seat.

`ItemDef::stack_limit` opts an item into merging; zero keeps every other item's
bundled charges as they are. A purchase fills a compatible stack before it takes an
empty slot: at the home shop (`SHOP_RANGE`) the bag before the stash, elsewhere only
the stash. `World::purchase_fits` checks gold-independent capacity, stack room
included, before anything changes. An explicit slot move merges up to three and
leaves the excess in the source; a full or incompatible destination swaps. Courier and
ground transfers move whole stacks, with no global merge pass. An invalid courier
backpack destination is refused before the source is taken, so no charge is lost.

Two stacks merge only with the same id, owner, attribute mode and sale mark. The
result keeps the oldest purchase tick, the OR of the touched flags and the larger
cooldown and mute, and an explicit move touches both stacks. Per-charge purchase
history is not kept: the merged metadata can only cost a fresh charge its refund, never
renew an old charge's refund window, lift its mute or change its owner. Only the buyer
sells: an untouched stack at most `SELL_REFUND_TICKS` old returns 65 per remaining
charge, anything else half the remaining value rounded down (32, 65 and 97 gold for
one, two and three charges). Consumed charges are never refunded.

### Shadowraze

| Constant in `game::rules` | Value |
|---|---|
| `RAZE_DAMAGE` | 85, 150, 215, 280 magical damage at levels 1 to 4 |
| `RAZE_DAMAGE_PER_SOUL` | 2 for every soul the caster holds as it goes off |
| `RAZE_STACK_DAMAGE` | 35, 50, 65, 80 per earlier live stack of the same caster |
| `RAZE_DEBUFF_TICKS` | 180 ticks, 6 seconds |
| `RAZE_MAX_STACKS` | 255 per victim and caster generation |
| `RAZE_MAX_SOURCES` | 16 caster records per victim |
| `RAZE_DISTANCE` | 200, 450, 700 world units |
| `RAZE_RADIUS` | 250 world units |
| `RAZE_MANA` | 75 mana at every level |
| `RAZE_COOLDOWN` | 270 ticks, for each reach on its own |
| `RAZE_CAST_POINT_MS` | 550 ms, 17 ticks; no backswing |

A hit carries its base damage and the caster's souls as it went off, reads the live
count of its own caster's record on the victim and adds that count times the bonus at
the current level; the total then passes magical resistance and integer truncation.
Level-one hits with no souls at zero resistance deal 85, 120, 155, 190; at 25% they
deal 63, 90, 116, 142. At level four, fifty souls and the full count the raw damage is
20,780, inside the fixed-point range. The count is a saturating `u8`, tied to the cap
by a compile-time assertion; a cap of three would confuse the three reach slots with
the debuff's own limit. Dota's has no cap at all.

Every hit that deals positive damage to a victim that survives it adds one stack and
refreshes that caster's whole record to 180 ticks, at the cap too. A hit in tick H is
live through H+179; `tick_modifiers` removes it before casts resolve at H+180, so a
hit at H+179 gets the bonus and a hit at H+180 starts again at one. Different casters,
allies or not, neither read nor refresh each other's records. With 16 records held, a
new caster evicts the one with the least time left, the first in record order on a
tie; refreshing an existing record evicts nothing, and expired records go first.

The record is a `ModifierKind::Shadowraze` in the victim's timed `Modifiers`, not a
`Stacks` entry on its seat, so death and respawn cannot carry it into a new body, and
no hook in `fight.rs`, extra table or cleanup pass on death is needed. A record may
outlive its caster, but a respawned or reused slot has a new generation and cannot
read it.

`HitEffect::Shadowraze` tags the queued damage with the zero-based cast level. The
count is read and added to during hit resolution, in queue order, so two queued razes
of one caster see each other's hits. Adding it when the cast is queued would leave a
debuff for damage an earlier lethal hit or invulnerability then prevented. Misses,
allies, failed casts (a dead caster's included), invulnerability and damage reduced or
rounded to zero add no stack and refresh none. Resolution reads an active `Shielded`
modifier itself, so a shield cast in that phase protects before the next stats
derivation. A fatal hit puts nothing on the dying body. Other magical hits neither get
nor build the bonus; facing, casting, shared learning and targeting are a plain
raze's.

A raze queued while its caster lived keeps its damage and stacking when an earlier
blow in the same batch kills the caster: cancelling it would make an accepted cast
depend on queue position. Its record belongs to the dead generation, never to the
respawn.

### On the wire and in the hash

`EFFECT_SHADOWRAZE` is `15` (13 and 14 are the tower's and the flagbearer's auras).
Each live record shows on a visible victim as an `EffectView` with `id =
EffectId(15)`, `ticks_left = Some(1..=180)` and `stacks = Some(1..=255)`; several
casters give several anonymous rows. The caster is never projected, fogged or not, and
which caster a row belongs to is not promised. The client names the effect `Razed`.

The world hash covers each record's source index and generation, count and timer,
and a queued hit's effect and level. The item hash covers merge ownership, mode, sale
marks, the dead courier's kept bag and whole ground stacks.

## Shadow Fiend at Dota 7.41f

Shadow Fiend is Dota 7.41f's, numbers and mechanics, wherever the engine has the
mechanic at all. The sources, all read in October 2026:

- the official hero data, `https://www.dota2.com/datafeed/herodata?language=english&hero_id=11`
  (base stats, every ability's special values, talents, no facets);
- the official patch notes from the same datafeed (`patchnoteslist`, then
  `patchnotes?version=7.39 … 7.41f`), which date every change below;
- the ability KV file `scripts/npc/heroes/npc_dota_hero_nevermore.txt` of
  `dotabuff/d2vpkr` at commit `969480b2e3` (client 6933, 2026-09-15, the day 7.41f
  shipped), byte-identical to `spirit-bear-productions/dota_vpk_updates`: cast points,
  behaviours, the death requiem's share and the Necromastery level curve;
- Liquipedia's Shadow Fiend page (`ctrlver = 7.41f`) for what neither of those says:
  attack backswing 0.54 s, the requiem's 0.9 s backswing, its first line due east, the
  death requiem applying the soul loss first and only burning, the feast's 0.5 s
  first gathering and that each enemy feeds it once.

Where they disagree the live data wins: 7.40b's notes and Liquipedia make the
requiem's magic resistance reduction a flat 10%, the 7.41 KV and the herodata both
say 5/10/15%, and 5/10/15% it is.

| | bota before | Dota 7.41f | bota now |
|---|---|---|---|
| Strength, agility, intelligence | 18 / 20 / 18 | 19 / 25 / 16 | 19 / 25 / 16 |
| Gain a level | 2.0 / 3.3 / 2.1 | 2.7 / 3.6 / 2.2 | 2.7 / 3.6 / 2.2 |
| Extra health, mana, damage a level | 38 / 6 / 1 | none | none |
| Base attack damage | 25 | 16–22 | 19 |
| Attack damage at level 1 | 45 | 41–47 | 44 |
| Health, mana at level 1 | 516 / 291 | 538 / 267 | 538 / 267 |
| Armor at level 1 | 3.33 | 4.17 | 4.17 |
| Base attack time | 1.7 s | 1.6 s | 1.6 s |
| Attack point, backswing | 0.5 s, 0.4 s | 0.5 s, 0.54 s | 0.5 s, 0.54 s |
| Turn rate | 0.5 | 0.9 | 0.9 (10431 brads a tick) |
| Mana regeneration | 0.25/s | 0.3/s | 0.3/s |
| Health regeneration, magic resistance, move speed, range, missile | 0.25/s, 25%, 305, 525, 1200 | the same | the same |
| Necromastery | learned, 4 levels | innate | innate, no slot |
| Souls cap | 12/16/20/24 | 20 | 20 |
| Souls a hero, a creep | 3, 1 | 4, 1 | 4, 1 |
| Damage a soul | 2 | 1 + 0.8 every 6 levels | the same, sum rounded down |
| Souls lost on death | 30% | 30% | 30%, rounded down |
| Raze damage | 90/160/230/300 | 85/150/215/280 + 2 a soul | 85/150/215/280 + 2 a soul |
| Raze stack bonus, duration | 50/60/70/80, 8 s | 35/50/65/80, 6 s | 35/50/65/80, 6 s |
| Raze mana, cooldown | 75/80/85/90, 10 s | 75, 9 s | 75, 9 s |
| Raze cast point | none | 0.55 s, no backswing | 0.55 s (17 ticks), none |
| Feast of Souls | absent | see below | see below |
| Presence armor | 2/3/4/5 | 2.5/4/5.5/7 | 2.5/4/5.5/7 |
| Presence reach | 1200, every enemy | 1200, enemies that see him | 1200, enemies whose side sees him |
| Armor the presence takes below zero | counted as zero | amplifies physical damage | amplifies, Dota's curve, for every unit |
| Requiem cast point, backswing | none | 1.67 s, 0.9 s | 1.67 s (51 ticks), 0.9 s |
| Requiem fear per line, max | 0.6 s, 2.13 s | 0.6 s, 2.15 s | 18 ticks, 65 ticks |
| Requiem magic resistance | none | -5/-10/-15%, multiplied | the same |
| Requiem lines | first along his facing | first due east | first due east |
| Requiem on death | none | half the kept souls, burn only | the same |

**Necromastery** is no ability to learn: `HeroDef::innate` names it and
`gathers_souls` asks the hero rather than the book. A soul's worth is held in tenths,
`10 + 8 * (level / 6)`, so twenty souls are worth 20, 36, 52, 68, 84 and 100 at levels
1, 6, 12, 18, 24 and 30; the sum is rounded down once.

**Feast of Souls** (`AbilityId(19)`, slot 3) is immediate: no cast point, nothing
broken off: it waits in a slot of its own on his orders (`Orders::immediate`), which
no other order clears, and goes off when actions run that tick. For 8 s it adds
35/50/65/80 attack speed and 4/6/8/10% movement speed, and every 0.5 s from the cast
— fifteen times; one on the tick it ends would be taken back at once — it takes souls
from up to two enemy heroes or creeps within 600 that his side sees, that damage
reaches and that it has not fed on yet: heroes first, the nearest first among each, 3 a hero and 1 a
creep, from 4/6/8/10 enemies in all. Those souls go on top of the count and past the
cap. When it ends, the souls of those still standing go; those of the fallen stay for
8 s more, and then whatever is over twenty goes. A kill while the count is past
twenty adds nothing. The feast's own state is a `Feast` component on him, its souls
are ordinary souls in `Stacks`; a death ends it at once, as running out would, before
the death takes its 30%. Recasting it on a feast still running (only a cooldown rate
modifier allows that) ends the old one first. Which enemies come first among equals
is not documented by Valve; nearest-first is bota's choice. A hero that dies and
respawns inside one feast is a new body and could be fed on twice; eight seconds
leave no room for it in practice.

**Requiem of Souls** keeps the souls it lets go, one line each up to twenty, the first
due east whatever his facing. A line that burns a unit and leaves it standing
frightens it, slows it and tears its magic resistance — the share it lets through
grows by 5/10/15% — for 0.6 s more a line, up to 2.15 s; those land with the burn in
hit resolution, so a line's own burn never feels its own tear, and lines landing in
one tick all burn at the resistance the tick began with. On his death, if the requiem
is learned and whatever its cooldown or his mana, a requiem goes off where he fell:
after the death's 30%, half of what is kept, rounded down, fly as lines that only
burn.

**What is not matched, and why.**

- A swing deals one number, not a roll between a minimum and a maximum: no unit in
  bota has a damage spread, and adding one is a new hidden stream for every attacker.
  Shadow Fiend's 16–22 is its mean, 19.
- Agility's attack speed and the primary attribute's damage are whole points, as for
  every hero: 75.4 agility at level 15 is 175 attack speed and 75 damage, not 175.4
  and 75.4.
- Intelligence's 0.1% magic resistance a point, the Attribute Bonus that levels on its
  own, talents and Aghanim's Scepter and Shard are not in the game for any hero, so
  Shadow Fiend has none of them. The talent levels that grant no ability point in
  Dota change nothing here: without talents fifteen points max his kit by level 18
  either way.
- Day and night do not exist: his 800 night vision never applies.
- Break does not exist, so nothing stops the souls being gathered.
- The requiem's phase movement in its cast point is not modelled.
- `shadowraze_cooldown 3` in the KV is not a rule of the normal game: the three razes
  keep independent 9 s cooldowns (Liquipedia: "Each Shadowraze ability independently
  checks …"), and the value has stood unchanged since well before the current
  cooldown. It is most likely Ability Draft's charge timing, where Shadowraze (Medium)
  is drafted alone with three charges; that mode does not exist here.
- The raze stack count stops at 255; Dota's has no cap.
- What a death requiem's lines kill pays nobody: a blow's source is a body, his is
  gone, and the bounty is found through the body's seat. Dota credits him. Lines of
  a requiem cast alive that outlive him, and his missiles in flight, were already so.

**On the wire.** These are the changes a third-party client sees:

- Shadow Fiend's slot 3 is Feast of Souls (`AbilityId(19)`, `Aim::Own`, castable, levels
  1–4) instead of Necromastery (`AbilityId(17)`), which no longer sits in any slot.
- Razes and the requiem have cast points: a cast is accepted, `AbilityCast` arrives
  when it goes off, and any order to the body before that breaks it off with nothing
  spent. `StatusFlags::CASTING` (bit 12) is set on a unit in a cast point.
- `EffectId::MAGIC_BROKEN` (18, `ticks_left`) is a requiem's torn resistance;
  `EffectId::FEASTING` (19, `ticks_left` and `stacks`: souls taken so far) and
  `EffectId::FEAST_LINGER` (20, `ticks_left` until what is past the cap is lost and
  `stacks`: souls taken from enemies that have died since) are the feast. They are the first effect ids `bota-proto` names.
- `UnitView.magic_resist` may be negative, down to -1, under a requiem.
- The souls effect (11) may count up to fifty while a feast runs or lingers.
- Raze stacks (effect 15) last at most 180 ticks.

## Wind Lace, Blood Grenade and the shop's stock at Dota 7.41f

The numbers are Dota 7.41f's, read in October 2026 from the official item data
(`https://www.dota2.com/datafeed/itemdata?language=english&item_id=1123` for the
grenade, `244` for the lace, `29` for the boots), the item KV file
`scripts/npc/items.txt` of `dotabuff/d2vpkr` (client 6957, 2026-10-09: stock, stack and
behaviour fields the datafeed does not carry), the official patch notes 7.33–7.41f from
the same datafeed (the grenade was added in 7.33, cheapened and restocked in 7.34; the
lace went to 225 gold and +15 in 7.38), and Liquipedia's Blood Grenade and Movement
Speed pages for the mechanics neither states.

| | Dota 7.41f | bota |
|---|---|---|
| Wind Lace | 225 gold, +15 movement speed | the same |
| Blood Grenade cost, passive | 50 gold, +50 health | the same, per slot |
| Use | point, 900 range, 0.2 s cast point, 75 health (set, never lethal), 10 s cooldown | the same |
| Projectile | 1100 a second to the spot | the same, `ProjectileView` with `AbilityId(20)` |
| Burst | 300 radius, enemy heroes and basic units: debuff, then 50 magical | enemy heroes and creeps |
| Debuff | 15% slow, 15 magical every second from 1 s after, five times, 5 s; a second grenade refreshes the time and keeps the beat | the same, `EffectId(21)` |
| Stacking | two to a slot, one charge a purchase, not shareable | two to a slot |
| Stock | two at the start, three at most, one more every 180 s, per side | the same |

- Boots and Wind Lace stack, but neither stacks with itself: Dota keeps flat movement
  speed in groups, the boots group (Boots of Speed, Power Treads, Phase Boots, ...) and
  the Wind Lace group each count only their fastest member, and the groups add. Before
  this, bota summed every carried bonus, so two Boots of Speed gave +90. `Carried` keeps
  the two groups apart (`move_speed`, `lace_speed`), each the best of its kind.
- The grenade's debuff is its own modifier kind rather than a `Slowed` beside a
  `Burning`: `Burning` beats on the world's 0.1 s clock and a second `Slowed` replaces
  whatever slow is on, while the grenade beats on whole seconds from its own landing and
  must not replace a stronger slow. It holds `5 s + 1 tick`: modifier time runs down in
  upkeep before the effects phase deals the beat, so the fifth beat at exactly 5 s needs
  the extra tick to land. Status resistance shortens it like any slow.
- The burst picks its targets as a raze does: hostile, seen, centre within the radius.
- The health cost sets health after the use went off, down to one point at the least,
  and is not damage: it breaks no drink and feeds no kill.
- Stock is per side, on `World::shop_stock`, ticked in upkeep. A shelf that is full does
  not run its clock; buying from a full shelf starts it. The wire carries the viewer's
  shelves as `WorldView::stock` (tail-appended), and a purchase from an empty shelf is
  refused with `RejectReason::OutOfStock`. A sold grenade does not go back on the shelf.

## Applied unit modifiers and the general stats behind them

A modifier is a bounded stat change that a cheat or trusted match setup puts on a unit;
only its countdown or the fall of the body takes it away. It lives in `World::applied`,
a table of `AppliedModifier` apart from `Modifiers`, so no ability, item, dispel or
ordinary expiry reaches it. It shows neither in `MatchInfo` nor in `UnitView.effects`.
`despawn` removes a unit's copy, so a respawned body starts clean. The countdown runs at
the end of the tick, so `ticks` counts the applying tick as the first. The hash covers
applied modifiers only while there are any, so an unmodified world hashes as one
without the feature.

**The payload is a bounded spec, not one cheat per stat.** `Cheat::ApplyModifier`
carries a `ModifierSpec` of signed basis-point fields (10,000 nominal for scales,
hundredths of a percentage point for resistances) and a tick count bounded by
`MAX_MODIFIER_TICKS`; `Cheat::ClearModifiers` takes it away. The server gate refuses an
out-of-bounds spec or tick count with `RejectReason::BadCheat`, and `World::cheat`
refuses anything unbounded that arrived another way. The order size budget is 80 bytes
for a cheat order and 32 for any other; the widest cheat order measures 70 bytes, and
the canonical bytes of one spec order are pinned in `bota-proto`.

**Modifiers are folded first and additively.** `derive_stats` folds a unit's specs
right after the raised base block and before items, attributes, auras, slows and every
other stage, so the modifier is part of the base the rest works on. Every family is
summed as a delta and written once: resistances in their own units, scales as deltas of
the nominal 10,000, so sources never compound. `move_speed`, `max_hp` and `max_mana`
are scaled from the raised base; flat item bonuses, strength, intelligence and the
`Slowed` and `Hastened` multipliers land on top unscaled. When an applied maximum moves
for a living body, the pool keeps its filled fraction (`hp' = hp * new / old`, clamped),
so a full pool stays full and a pool that held anything stays non-empty. A respawned
body, and a unit seeded at match start once its rules have landed, stands at its full
effective maximum. `Stats` carries `status_resist_bp`, `physical_amp_bp`,
`magic_amp_bp`, `pure_amp_bp`, `cooldown_rate_bp` and `mana_cost_rate_bp`, all neutral
by default; magic resistance was a stat already. Every unit kind walks the same derive.
The fold compiles out while the table is empty (`derive_stats_impl::<false>`), and so
does the amplification multiply in hitting (`hitting_system::<false>`) while no blow is
amplified, so the default game pays nothing.

**The bounds are part of composition.** A match carries at most `MAX_SPAWN_MODIFIERS`
(64) setup rules, a selector names at most `MAX_SPAWN_TARGETS` (16) kinds and
categories, and a unit carries at most those 64 entries plus one cheat entry. Every
sum saturates. Applied magic resistance ends in `0..=10_000` bp and status
resistance in `0..=9_999` bp. The three magnitude scales saturate after addition at one source's
`2_500..=40_000`, keeping world magnitudes within 0.25x to 4x: they bound the size of
the world. Damage amplification, the cooldown and mana cost rates and bounty gold keep
every bounded source, up to `MAX_COMBINED_MODIFIER_SCALE` = 1,960,000 (196x), with
floors of zero or one; every loop and accumulator still has a fixed ceiling.

**Each stat has one reading.** Magic resistance is kept in basis points, added as a
delta and clamped to `0..=10_000` before Flesh Heap multiplies it, so mitigation,
projection and the bots agree; a requiem's torn resistance multiplies last and is the
one thing that takes it below zero, down to `rules::MIN_MAGIC_RESIST_BP` (-100%), where
magic damage lands amplified as in Dota. Damage amplification scales a blow before armor and resistance, after the
Shadowraze stack bonus, by the dealer's field for the damage kind; a blow with no
source is left alone, and pure damage stays unmitigated but still scales. The outgoing
scale is captured when a blow or its carrier is created (attacks, projectiles, hooks,
requiem lines), as a crit is, so nothing that happens to the source afterwards changes
a hit in flight. Status resistance scales the ticks of `Stunned`, `Feared`, `Slowed`, `MagicBroken` and `BloodGrenade`
wherever they are put on, down to one tick, and never touches buffs; time already held
is not shortened again when a disable is extended. A hold renewed every tick of a
channel (Dismember, the hook's drag) has its ticks shortened too, but its length is the
channel's. Cooldown rate scales a cooldown when it is set (a cast, an item use, a
shared item wait, the break-on-damage mute), to at least one tick, and never the
decrement, so a stored cooldown and every view of it stay exact. Mana cost rate scales
every read of a cost: the order gate, the charge, `AbilityView.mana_cost` and
`ItemView.mana_cost`, so what the view calls affordable is accepted and charged the
same. Gold income scales the bounty once, as `pay_for` credits it to the killer; passive
gold, starting gold, sale refunds and death losses are not bounties. A change applied
mid-tick is seen by everything derived or read after it; a disable put on before the
tick's first derive (a hook stun beside the application) still reads the previous
tick's resistance.

**Trusted setup can put modifiers on spawns.** `MatchConfig` carries a bounded list of
`SpawnModifier`, each a selector (a side, exact kinds or categories), a `ModifierSpec`
and a duration: `MatchLong` until the body falls, or a tick count. `World::for_match`
copies the list into the world and puts it on every unit already standing, buildings
included, and `spawn_body` on every unit stood up later: each wave, camp, building and
respawned body gets the rules afresh, a `Ticks` rule restarting on the new body. The
cheat path is independent: a unit may carry both, and clearing a cheat leaves the
setup's entries alone. `MatchConfig::validate` refuses a spec outside the cheat gate's
bounds, a duration outside `1..=MAX_MODIFIER_TICKS` and a list past
`MAX_SPAWN_MODIFIERS`, naming the rule that failed; `World::for_match` calls it and
panics rather than dropping a rule, as does a spawn reached by an unchecked rule. The
hash covers every rule while the list is non-empty. The server binary does not expose
the list: the TCP lobby always starts with none, and a setup builds it in process.
