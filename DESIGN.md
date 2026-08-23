# bota — design

A simplified Dota 2 in Rust for AI bots and humans. Fully deterministic simulation,
server authority, minimal dependencies.

Code conventions — `CLAUDE.md`.

## Key decisions

| Decision | Choice | Rationale |
|---|---|---|
| Client rendering | macroquad | one direct dependency, 2D + text out of the box, Linux/Win/macOS/WASM |
| Tick mode | Realtime **and** Lockstep | humans play in realtime, bots are debugged/trained in reproducible lockstep |
| v0.1 scope | 1v1, three lanes, 1 hero | a vertical slice of the whole stack on minimal content |
| Transport | TCP + length-prefixed frames | no tokio: a thread per client + a simulation thread + mpsc |
| Serialization | `serde` + `postcard` | derive removes ~250 lines of manual put/get; the format is compact (varint) and stable within 1.x |
| Player networking | full snapshots with fog | measured: 1425 bytes per snapshot in 1v1, 41 KB/s at 30 Hz. Deltas will be needed by 5v5, not earlier |
| Live spectating | the same snapshots, without fog | client-side simulation is impossible, and that is intentional |
| Replay | fogless `ServerMsg` stream + all orders | plays back without a server: the client reads frames from a file instead of a socket; coupled to the wire format only, so it survives balance changes; the client still cannot simulate |
| PRNG | `rand_chacha::ChaCha8Rng` in `bota-server` | value-stable by the crate's contract; `rand::StdRng` is **forbidden** — it is allowed to change its algorithm in a minor release |

## Structure

```
bota/
├── Cargo.toml               # workspace, resolver = "3"
├── crates/
│   ├── bota-proto/          # shared vocabulary + codec. deps: serde, postcard
│   ├── bota-server/         # simulation + networking + lobby. deps: proto
│   ├── bota-client/         # macroquad: rendering, input, spectating. deps: proto
│   └── bota-bot/            # bot SDK + example. deps: proto
├── assets/
├── replays/
└── tests/
```

```
        bota-proto
        ↑    ↑    ↑
   server  client  bot
```

The workspace currently contains `bota-proto`, `bota-server` and `bota-client`;
`bota-bot` joins at stage 8.

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

A previous version of the design had a `bota-core` crate holding the simulation. It was
dropped: "core" had no checkable membership criterion, and `seed`, `tick_rate` and
`PlayerId` steadily leaked into it — exactly what does not belong there. A side benefit
of moving the simulation into the server: the client is **structurally** incapable of
simulating, `World` lives in a crate it does not depend on.

### Module layout

```
proto/src/
├── math.rs       Fixed (Q16.16 in i32), Angle (brads), Vec2
├── ids.rs        EntityId, SlotId, PlayerId, Team, HeroId, AbilityId, ItemId,
│                 AbilitySlot, ItemSlot, MapId, UnitKind
├── order.rs      Order, OrderTarget
├── event.rs      EventKind, DamageKind
├── view.rs       WorldView, UnitView, PlayerView, ProjectileView, AbilityView,
│                 ItemView, StatusFlags
├── msg.rs        ClientMsg, ServerMsg, MatchInfo, lobby, RejectReason, MatchStats,
│                 ReplayRecord
└── codec.rs      encode_frame, decode_payload, FrameReader, CodecError
```

```
server/src/
├── sim/          KNOWS NOTHING ABOUT SOCKETS
│   ├── arena.rs      Arena<T>: generational slot store behind EntityId
│   ├── rng.rs        MatchRng over ChaCha8Rng: streams by purpose, Ratio/Chance
│   ├── config.rs     MatchConfig
│   ├── world.rs      World: entity arenas, tick, spawn, FNV-1a hash
│   ├── units.rs      Unit, UnitOrder, SeatState; hero/creep/building constructors
│   ├── heroes/       hero stats and ability implementations (stage 9)
│   ├── abilities.rs  ability engine: cast point / channel / cooldown / mana (stage 9)
│   ├── combat.rs     windups, projectiles, the damage queue, armor and resist
│   ├── movement.rs   isqrt, stepping, blocking, turning, passability grid
│   ├── path.rs       A* over the grid, grid line of sight
│   ├── vision.rs     fog of war: pure radius queries, nothing cached
│   ├── econ.rs       gold, experience, levels, deaths, respawns
│   ├── rules.rs      balance constants
│   ├── project.rs    World → WorldView
│   └── step.rs       Command, Event, validate, step: the tick order
├── net/          accept loop, per-connection reader/writer threads, Outbox
├── lobby.rs      Roster (PlayerId ↔ SlotId), seats, picks, readiness
├── game_loop.rs  lobby phase, then the tick loop in both modes
├── replay.rs     writes the replay: fogless frames plus per-tick orders
└── main.rs       clap arguments
```

No ECS: `World` is a set of `Arena<T>` stores — generational slot arenas iterated in
slot order. Removing an entity bumps the slot's generation, so a stale `EntityId` never
resolves to whoever took the slot over.

## Contracts

### Simulation (server/src/sim)

```rust
pub struct MatchConfig {
    pub match_id: u64, pub master_key: [u8; 32], pub picks: Vec<Pick>,
    pub map: MapId, pub tick_rate: u16, pub mode: TickMode, pub ack_timeout_ticks: u32,
}

impl MatchConfig {
    pub fn rng(&self) -> MatchRng;        // see below
    pub fn info(&self) -> MatchInfo;      // projection onto the wire, the type has no seed field
}
```

The match seed is derived with `rand_chacha` itself, no separate hash function needed:
`master_key` is a 32-byte seed, `match_id` is a stream number. A match is reproducible
from the pair `(master_key, match_id)`, which is convenient for debugging. Implemented
in `sim/rng.rs`:

```rust
// MatchRng::new(master_key, match_id)
let mut root = ChaCha8Rng::from_seed(*master_key);
root.set_stream(match_id);
let mut seed = [0u8; 32];
root.fill_bytes(&mut seed);

// one stream per purpose — crits, runes, spawn scatter
rng.global(Purpose::Rune)
rng.for_unit(Purpose::Crit, unit, source)
```

Streams are separated by purpose (`Purpose`: `Crit`, `Block`, `Evasion`, `Rune`,
`NeutralSpawn`) so that a new draw in one place does not shift generation anywhere
else. A per-unit stream is keyed by `(purpose, slot index, source)` packed into the
64-bit ChaCha8 stream id — purpose in the top bits, slot index in the middle, a source
byte to separate several sources of chance on the same unit (a crit passive and a
bash). The key uses the slot index rather than the full `EntityId`: the stream id
space stays bounded, and a unit reusing a freed slot continues that slot's hidden
sequence, which no observer can distinguish from a fresh one.

```rust
impl World {
    pub fn new(cfg: &MatchConfig, rng: MatchRng) -> World;  // rng is initial state, not config
    pub fn step(&mut self, cmds: &[Command]) -> Vec<Event>;
    pub fn view(&self, team: Team) -> WorldView;        // with fog
    pub fn view_full(&self) -> WorldView;               // spectator
    pub fn can_see(&self, team: Team, target: EntityId) -> bool;   // order validation
    pub fn winner(&self) -> Option<Team>;
    pub fn stats(&self) -> MatchStats;
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

1. No `f32/f64` in `bota-proto` and `bota-server`: `#![deny(clippy::float_arithmetic)]`.
   Float operations themselves are deterministic per IEEE-754, but `sin/cos/sqrt` from
   libm are not — they differ across glibc / musl / macOS / wasm. The client renders in
   float freely. The bot is also free to think in float: what gets recorded are its
   orders, not its reasoning.
2. Scalars are `Fixed` = Q16.16 in `i32`, multiplication through an intermediate `i64`.
   Range ±32768 units, precision 1/65536. The 16384-unit map keeps squared distances
   inside an `i64`; segment projections that would square a dot product go through
   `i128`.
3. Angles are "brads": `u16`, 65536 = a full turn. sin/cos from a hardcoded table of
   1024 entries.
4. Distances are compared as squares, no sqrt.
5. Entity iteration is always by ascending `EntityId.idx`. `HashMap` is forbidden in
   the simulation.
6. Commands are sorted by `(tick, slot, seq)` before applying.
7. Time exists only as ticks (`u32`), 30 ticks/sec. No `std::time` in `sim`.
8. Damage, gold, experience are integers.
9. External primitives are taken only if value-stable. `rand::StdRng` and
   `std::collections::hash_map::DefaultHasher` explicitly give no such guarantee
   between releases: the former is replaced by `rand_chacha::ChaCha8Rng`, and for
   `world.hash()` we write FNV-1a (ten lines, xor and multiply in a loop).
10. `world.hash()` covers the whole state, hidden included: entity arenas, stream
    positions, every `Chance` mask. A divergence in randomness consumption must move
    the hash on the tick it happens, not when its first visible outcome differs.

### Chances (crits, block, evasion)

The requirements conflict: an exact 30% rate **and** no way to predict the next crit.
A simple accumulator (`acc += 0.30`, firing at `acc >= 1`) gives the first but not the
second: it is periodic, an observer sees crits in damage events and reconstructs the
whole phase after one or two observations.

The solution is exact counting per block with a hidden order inside the block. A chance
is declared as a fraction:

```rust
pub struct Ratio { num: u8, den: u8 }     // rules.rs: CRIT_CHANCE = Ratio::new(3, 10)

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
            self.reshuffle();                         // partial Fisher-Yates off the stream
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
- The initial block offset of every source is drawn from the same stream, so block
  boundaries are not known to an observer.

Residual leak: within a block the opponent can count (saw 3 crits in 6 hits — knows the
next 4 are clean). Unavoidable for any scheme with an exact rate. The alternative is
giving up the exact rate for a plain hidden PRNG, which contradicts the determinism
requirement.

When the chance changes (a crit buff), the current block finishes under the old
fraction; the new one takes effect from the next block.

The PRNG is also used where an exact rate is meaningless (rune drop, spawn scatter).
Streams are separated by purpose via `ChaCha8Rng::set_stream`, so that a new call in
one place does not shift the rest of the generation.

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
- outcomes of scheduled events that have not happened yet (which rune will spawn).

Checked by a test: `view_full()` is run through serialization and compared against a
whitelist of fields, so a new field in `Unit` cannot leak silently.

It also follows that client-side prediction covers only movement and animation. Damage
numbers and the fact of a crit arrive as events from the server.

Two further channels are closed by rule, because a reward-driven bot will find and
exploit any leak a human reviewer shrugs off:

- A reject reason does not depend on hidden state. A dead target and a fogged one get
  the same `UnknownTarget`, so probing the fog with stale handles reveals nothing.
- A unit never acts on what its team cannot see. A standing `AttackUnit` order whose
  target left the team's vision degrades to attack-moving toward the last seen
  position; the unit does not track the hidden target, so its own path reveals
  nothing either.

## Game model v0.1

- Map 16384×16384 — Dota's scale, so speeds, ranges and vision keep their Dota
  absolute values. Symmetric along the diagonal. Three lanes: mid along the diagonal,
  top up the west edge and along the north edge, bottom its mirror; the diagonal
  mirror that swaps the sides also swaps top and bottom. Passability is a 256×256 bit
  grid.
- Teams Radiant / Dire, 1v1 (the architecture is sized for 5v5).
- Buildings: three towers per lane per side — tier one by the river, tier three by
  the base — plus a pair of tier fours by each Ancient. The Ancient is invulnerable
  until its last tier four falls. A fountain that heals and burns. Barracks and the
  rest come later.
- A lane's centerline runs through every tower of the lane, so a wave marches from
  tower to tower and can never wander past one outside its own acquisition range.
- The match opens with a 30 s pregame: the game clock counts up from -0:30 and the
  first wave walks out at 0:00, when passive gold starts. `MatchInfo` carries the
  pregame length so the clock renders without knowing the ruleset.
- The landmarks are the current Dota 2 map, extracted from the installed game's
  `dota.vpk` with ValveResourceFormat and shifted by half the map so Dota's
  origin sits at the center: `MAP_SIZE` 18432, both fountains, both Ancients,
  all 22 towers, all 6 lane spawners and all 28 neutral camps at their real
  positions. The two sides are not mirrors; each carries its own table, and
  `mirror()` survives only as a utility.
- The terrain is the same map's own ground, baked in `sim/terrain.rs`: the
  gridnav's static walkability (cliffs, pits, the map edge close their cells
  before trees and buildings do) and, from the physics mesh, an elevation tier
  per cell in 128-unit steps — river bed 0, lane ground 1, highground 2, bases
  3 — plus the water mask of the river and pools. A ranged attack landing on
  ground higher than it was fired from misses one time in four, from a
  match-global exact-ratio chance stream; abilities never miss. The terrain
  rides in `MatchStart` run-length encoded, and the client bakes it into the
  ground texture for the world and the minimap.
- Vision is a radius with sight lines walked over the terrain cells. A cell is
  opaque to a viewer when its ground is higher than the viewer's, when a tree
  stands on it, or when one of the map's own fog blocker walls crosses it —
  eleven named walls of `ent_fow_blocker_node` points, imported like
  everything else, which is what seals the Roshan pit even through its
  entrance. A named group holds several separate walls: only nodes within
  the blocker span of each other bridge a segment, so the far-apart jumps
  inside a group — the two pits share one name — are breaks, not walls. Buildings, water and units block nothing. The viewer's own cell
  and the target's cell never block, so standing beside a tree does not blind
  and a treeline's edge stays visible; a target on ground above the viewer is
  always dark. The opaque cells ride in `MatchStart`, and the client walks
  the same sight lines from its own units to shade unseen ground in the world
  view and on the minimap; spectators see everything. The map's
  `ent_fow_revealer` points wait for outposts.
- Trees are static blockers imported one for one from the same map: all 2475
  positions — the main entity lump plus the base layers of both sides — live as
  a table in `sim/trees.rs`. Two carves adapt them to this map: trees within
  the lane-clear band of a straightened lane centerline are dropped — the real
  forest follows the real curved roads, and these lanes walk tower-to-tower
  chords — and a small pad around each fountain stays clear. The
  full tree list rides in `MatchStart`, so the client draws without knowing the
  layout rules. Trees are closed into the passability grid at world build; they
  do not block vision yet and are indestructible until an axe exists.
- Roshan stands on the map's own spawner point in the south-east river pit —
  both pit enclosures came in with the terrain, and the north-west one stays
  empty until day and night exist. He behaves like a neutral: answers whoever
  comes close or hits him, leashes back to the pit and heals in full. His
  death pays the killing seat the bounty, every seat of the killing team the
  team gold, and experience around the pit; the grave lasts eight minutes plus
  up to three more on a hidden draw, then he returns. No Aegis until items can
  resurrect.
- The jungle belongs to `Team::Neutral`, hostile to both sides; seats never sit
  there. The twenty-eight camps stand where Dota's own neutral spawners stand. They
  fill with neutral creeps one minute past the horn and every minute after, but only while the camp box is empty — any body inside
  blocks the spawn, which is camp blocking. A neutral answers whoever comes into
  its aggro range or hits it, and dragged beyond its leash it goes home deaf and
  arrives at full health. Its bounty goes to the killer, its experience to the
  killer's team nearby.
- Creeps: 3 melee + 1 ranged every 900 ticks (30 s) on every lane, a siege creep
  every 5th wave. A wave marches its own lane's waypoints and is leashed to its own
  lane.
- Hero: Sylla (ranged carry). 3 abilities + an ultimate, levels 1–10.
- Economy: passive gold 1/sec, last hits, kill bounty with streaks.
- Attributes are Dota's three: strength buys health and health regeneration,
  agility buys armor and attack speed, intelligence buys mana and mana
  regeneration, and whichever one a hero is primary in buys its attack damage.
  They are held in fixed point rather than whole points, because Dota's growth
  per level is fractional and rounding it to whole numbers would put the same
  hero on two different curves depending on where the rounding fell. They sit
  on `UnitDef` and in `Stats`, so a creep simply has none of them and pays for
  nothing; the derivation runs once, after items have been added and before
  anything reads what attributes pay for. The base numbers of both heroes were
  cut by exactly what their attributes now hand back, so level one is the body
  it always was.
  The alternative — leaving attributes out and giving every item flat health,
  mana and damage — was rejected because it throws away the whole cheap end of
  the Dota shop: Circlet, the three 140-gold attribute items, Bracer and its
  two siblings all exist to be bought as attributes, and inventing replacements
  for them is work with nothing behind it.
- Attack speed is a number on the Dota scale, where 100 is a unit's own pace,
  and the interval between two attacks is the kind's own interval scaled by it.
  It replaces the earlier haste effect, which took a percentage off the
  interval directly: percentages off an interval do not add up — two sources of
  twenty percent are not forty — so item bonuses and Frenzy could not have been
  put on the same footing. The bounds are Dota's, 20 to 700.
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
- Items follow the Dota slot topology, engine in `game/systems/gear.rs`. A seat owns
  fifteen slots: six inventory, where items work; three backpack, where they ride
  inert — and a stack leaving the backpack for the inventory is muted for six
  seconds before it works again; six stash. A purchase spends gold anywhere, but lands in the
  inventory only inside the home shop area (the fountain circle) — bought
  remotely it waits in the stash, and the stash itself opens only at that shop.
  Selling also happens at the shop: half price back, the full price for an
  untouched item within ten seconds of purchase. `MoveItem` swaps any two slots.
  Carried bonuses are flat and apply only from unmuted inventory slots; a pool
  keeps its filled fraction whichever way its maximum moves. The earlier rule —
  grow by the whole delta, shrink by clamping alone — was a mint: a Power
  Treads wheel (strength → agility → intelligence → strength) re-gained on
  every switch back what the switch away never took, and a few dozen switches
  refilled both pools from next to nothing. The scaling floors, so a full
  wheel can only lose a sliver, never gain one; and a pool that held anything
  is kept off zero, so the wheel cannot kill its owner either. Consumables
  (Healing Salve, Clarity) drip over
  thirty seconds and spill on any hit from a hero. Items survive the hero's
  death on the seat.
- An item set to an attribute — Power Treads — keeps which one on the stack
  rather than in the catalog, and the wire carries it in `ItemView`, since two
  players holding the same item may have it on different attributes and the
  client has to draw which. Switching is an ordinary `UseItem` with no target:
  a second order kind for one item would be a wire change bought for nothing.
- Blink is a point-targeted use like any other. Aimed further off than it
  carries it carries as far as it does along the same line rather than
  refusing, which is how Dota reads a click past the edge of the range. A
  landing spot on closed ground steps back along that line a grid cell at a
  time until it finds open ground: refusing outright would have been simpler,
  but it makes the item unusable at exactly the cliffs it exists to cross.
  A blow from a hero, a tower or Roshan sets it back, and that lives in the
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
- Two orders cover the ground: `PutItem` lays what sits in a bag slot out —
  at a point, underfoot when aimed at nothing, or into the first free slot of
  an allied bag when aimed at a unit — and `TakeItem` picks a ground item up.
  Dropping and handing over are one order, not two, because they are one
  motion — out of the bag, differing only in where it lands — and
  `OrderTarget` already spells the difference. Both orders walk their unit
  into reach first, the way Dota reads a drop aimed across the map; an aimed
  unit may be moving, so following is needed anyway, and one walk serves both.
  The walking lives in one `Handling` component and one tick pass shaped like
  the courier's errands, cancelled where an errand is: any later order calls
  it off. Item actives refuse beyond their reach instead of walking, and stay
  that way: a use is aimed where the fight is, a put is aimed where the feet
  will be.
- The stash does not `PutItem`: it is a shelf at the shop, not a pair of
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
  rule guards the till, not the hands. Ownership stays server-side; the wire
  does not carry it.
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
  when an attack order's target dies, the attack rolls onto the closest enemy in
  acquisition range. Rest never starts one — standing idle, arriving off a move
  order or stopping leaves the wave alone. A move order ignores enemies for its
  whole length, Hold attacks whatever is in range without moving, attack-move
  acquires along the way.
- Collision follows Dota's documented two-pather model. The long path is planned
  against static blockers only — structures on the grid; the short path steers the
  walker along it around standing bodies in continuous space: within the steer
  range the walker aims at a tangent point of the first standing circle across its
  segment, resolved over a few hops when one tangent uncovers the next body, so a
  stander is skirted along its hull at full speed — standing in front of a wave
  does not hold it, exactly as in Dota, where stationary units are avoidance
  obstacles for the short pather. Whoever occupies the walker's own goal is not
  steered around. All contact is solid: the distance between two units never drops
  below the sum of their radii, a step deeper into anybody's circle is refused,
  and a walker pressed right against a stander traces its circle by sidesteps.
  The static grid is a hard wall too: a step or sidestep into a cell closed by a
  structure or a tree is refused outright, while a step out of one is always
  allowed, so nothing ever wedges inside the forest. A
  unit that is walking is not avoided at all: whoever runs into it presses into
  the body, fully stopped, for the block wait, and only then starts sidestepping
  around. That stop, paid again on every new contact, is what makes creep-blocking
  work — a hero zigzagging across the wave's path re-stops every creep whose step
  his hull intersects, while a hero standing still is simply flowed around. Nobody
  is ever pushed: the unit standing its ground does not move a hair, and a unit
  stands still for its whole attack point and backswing.
- A calm creep dragged off the lane beyond its leash gives up, goes deaf to targets
  and walks straight back to the nearest point of the lane; an open aggro window
  overrides the leash.
- Units turn at a finite rate and only walk or swing once they face their current
  path leg, so corners cost time. Buildings do not turn.

Movement routes around structures with A* over the passability grid: structures close
cells when the world is built and reopen them when they fall. A unit walks straight
whenever the grid says the line is clear, and otherwise follows the corner waypoints
of a route; each leg is walked only after turning onto it, so corners cost time.
Standing units are hugged around at contact, walking units stop whoever runs into
them — and every swing ends in a backswing the unit stands through, which is the
pause a creep makes over its kill before marching on. A hero's order cancels its
backswing.

Abilities run on a shared engine in `sim/abilities.rs`: four slots per hero, each
with a level and a cooldown, held on the seat like items, so both survive the
hero's death — and cooldowns keep running while it is dead. A skill point arrives
with every hero level; basic ability level k needs hero level 2k-1, ultimate
levels open at 6, 8 and 10. A cast
order is validated (learned, off cooldown, mana, target kind, cast range) and
executes in the ability phase of the same tick, instantly — cast points and
channeling come later. Casts of the same tick run after cooldown ticking, so a
fresh cooldown surfaces at its full value. Sylla's kit: slot 0 a critical strike
passive fed by the hidden per-unit `Chance` stream (the stream is keyed by the
arena slot, so respawning continues the sequence); slot 1 an attack speed
self-buff for its duration; slot 2 a magical
projectile that bounces to the closest unhit enemy in range, never a structure;
slot 3 an ultimate volley launching an attack projectile at every enemy unit in
its radius. A crit rolls once at windup completion and rides the projectile,
reported only in the `Damaged` event.

Shadow Fiend carries three razes and the requiem, and gathers souls without spending
a slot on them. Dota gives him six entries — three razes, Necromastery, Presence of the
Dark Lord and Requiem of Souls — and a hero here has four slots. The three razes are
what the hero is played by, so they keep their slots; Necromastery becomes an innate of
the body, a `souls` flag on `HeroDef` that stands the `Souls` component up at spawn and
grows its cap with the hero level rather than with a skill point. Presence is dropped
rather than merged into something else: an armor aura is the one part of the kit that
changes nothing about how he is played.

A soul is taken only from what he brings down himself, which is the killer `bury`
already carries, and a hero is worth three where anything else is worth one. Souls
outlive his own death — they wait on the seat in `Kept` with the abilities and the
items — which Dota does not do. Dota's version spends the accumulation twice, once on
death and once on the requiem, and a hero whose only scaling is a resource that two
different events take away is a hero the bots learn to stop gathering with. The requiem
therefore reads the souls and keeps them; its cooldown is what limits it.

Souls are not a component of their own. Necromastery and Flesh Heap are the same shape -
a count that grows on a death, never runs out, and survives the body - so both are one
`Stacks` component keyed by `StackKind`, and a third of them costs a variant rather than
a table, a field in `StatsCx`, a line in the hash and a line in the projection. The wire
follows: an effect carries an `EffectAmount`, either `Ticks` for one that runs out or
`Stacks` for one that is counted, and a gathered count travels as an ordinary effect with
its own `EffectId`. A `souls: u32` on `UnitView` was written first and thrown away: it
puts one hero's vocabulary into the shared one, it says nothing about the flesh heap,
which has the same shape and was not on the wire at all, and every counter after it would
have to buy its own field. The two counts are separate optional fields rather than one
enum of `Ticks` or `Stacks`: an effect that stacks and also runs out is an ordinary thing
to want - Dota's Fury Swipes is one - and a sum type forecloses it in the shared
vocabulary, which is the expensive place to be wrong. The byte an effect pays for the
second `Option` buys that.

A raze is aimed at a point rather than fired along the facing. The engine has no
angle-to-vector table — `facing_towards` is an octant approximation and facing is
cosmetic, nothing compares it — so the only exact direction available is the one from
the caster to a point, and `point_along` walks that line out to the raze's own distance.
The consequence is that the aim names the line and not the landing spot: a raze aimed
short still lands at its reach, and one aimed long walks the caster in first, on the
same rule every other ability with a cast range uses.

Hero roadmap (added as data + ability implementations; the engine does not change):

| Hero | Type | Abilities |
|---|---|---|
| Sylla | ranged carry | crit passive / attack speed buff / bouncing projectile / ult: multishot |
| Krag | melee tank | stun dash / cleave passive / armor aura / ult: shield + damage return |
| Vex | ranged nuker | nuke / slowing AoE / mana passive / ult: AoE burst |
| Grum | melee initiator | hook / DoT aura / slow / ult: AoE stun |
| Lira | support | heal / shield / wards / ult: team heal aura |

Built since: Pudge (hook / rot / flesh heap / ult: dismember) and Shadow Fiend
(three razes / ult: requiem, with souls innate).

## Protocol

Frame: `u32 len (LE) | postcard payload`. The message kind is the postcard enum tag
inside the payload. TCP, `TCP_NODELAY`.

Until the first release there is no versioning and no compatibility: the wire carries
no version field and no ruleset fingerprint, and nothing — protocol, replay files,
hash baselines — promises to survive across pre-release commits. Mismatched builds
are not detected; they are simply not run against each other. Version and fingerprint
fields appear with the first release, when there is something to be compatible with.

### Client → server

```rust
enum ClientMsg {
    Hello { role: Role, name: String },    // Role: Player|Bot|Spectator
    PickHero { hero: HeroId },
    SetReady(bool),
    Order { seq: u32, order: Order },
    Ack { tick: u32 },                     // lockstep: "I am ready for the tick"
}

enum Order {
    Stop, HoldPosition,
    Move { pos: Vec2 }, AttackMove { pos: Vec2 }, AttackUnit { target: EntityId },
    CastAbility { slot: AbilitySlot, target: OrderTarget },
    UseItem { slot: ItemSlot, target: OrderTarget },
    LevelUpAbility { slot: AbilitySlot },
    BuyItem { item: ItemId }, SellItem { slot: ItemSlot },
}

enum OrderTarget { None, Point { pos: Vec2 }, Unit { target: EntityId } }
```

Casting is one variant, `CastAbility`, with the target kind expressed by
`OrderTarget`; whether the variant fits the ability is validated by the server
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
    MatchOver { winner: Team, stats: MatchStats },
    ParticipantLeft { player_id: PlayerId, slot: Option<SlotId> },
}
```

A Player/Bot receives a whole `WorldView` on every tick, filtered through its team's
fog. A live Spectator receives the same stream unfiltered. There are no deltas: a
client can start rendering from any snapshot, so joining mid-match requires nothing
special. Measured: 1425 bytes per snapshot in 1v1 (25 entities), 41 KB/s at 30 Hz.
`diff`/`apply` will arrive together with 5v5, where it would reach about 150 KB/s.

Within a tick the order on the wire is fixed: `Snapshot(t)`, then `Events(t)`. An
event may name an entity the snapshot no longer carries — it died on that very tick;
the previous snapshot is what to render such an event against.

A slow consumer cannot stall the simulation: the tick thread never blocks on a
socket. Snapshots are coalescible by construction — each one is whole, so a
connection that fell behind is sent only the latest. Events cannot be skipped, so a
connection whose event queue overflows is closed instead of buffered without bound.

The vision mask does **not** go over the wire. It is derived from the positions and
`vision_radius` of the units already present in the view: your own units are always
visible, so the computation needs nothing beyond what the client already received.
Saves two kilobytes per snapshot.

There is no shared implementation in `proto`, and that is not an omission. The sides
need different things: the server — an exact answer to "does this team see this
point", the client — a soft gradient for rendering with fade-out and a memory of the
explored, the bot often nothing at all. One shared function would either spoil the
picture or coarsen the filter.

The server-side representation of vision is intentionally not pinned down in the
design. A grid is not the only option and probably not the best one: as soon as trees
and vision cones appear, region geometry becomes both more precise and cheaper. This
is decided when `sim/vision.rs` is implemented. For the same reason each side keeps
its own mask type and grid constants — they do not cross the wire and are not needed
to read it.

This rests on two conditions, and breaking either puts the mask back on the wire:

1. Vision is a pure radius. As soon as terrain starts occluding it, the computation
   becomes a game rule, and rules do not belong in `proto`.
2. Every source of vision is represented by an entity in the view. Wards already
   satisfy this; an ability granting vision without a unit must be modeled as an
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
    Orders { tick: u32, orders: Vec<(SlotId, Order)> },   // what every seat asked for
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
file. In 1v1 the stream runs at about 2.5 MB per minute; archiving is an external
compressor's job, not the protocol's.

### What a slot is worth, and who works it out

Everything a slot of the panel shows about the thing in it rides in that unit's
own view, already worked out by the server: the range of the next cast, its
mana, whether a toggle is on, whether a skill point could go into it. None of
it is a number the client looks up. The reason is that none of these are
properties of a catalog entry: a range moves with the level, and once an item
or a talent moves it, two units holding the same ability hold two different
ranges. `AbilityView.mana_cost` already worked this way, and everything else
followed it.

The one thing that belongs to no unit is what the shop asks for a thing nobody
holds yet, so `MatchInfo` carries the price and the parts of every item once
per match, and the client prices a purchase against what the seat holds by the
same rule the server charges by. The client's own catalog is what is left over
after that: names, blurbs and art, and not one number.

`can_level` is a bit rather than a rule. The client used to hold its own copy
of "a basic ability's level k waits for hero level 2k-1, an ultimate for 6, 8
and 10" and its own subtraction of points spent from the hero level; the
server already answers all of that in `level_floor`, and answering it once is
cheaper than keeping two copies honest.

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
is already taken up, and who is commanded. Two entry points that each grew
their own rules is what the client had before — a click on an ability box did
nothing at all, and a click on an item box worked only for consumables while
its key worked for everything.

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

### Tick modes

- `Realtime` — 30 Hz on the wall clock, a late command applies on the next tick.
- `Lockstep` — the server waits for `Ack(tick)` from every agent. A bot thinks as long
  as it needs, the match reproduces bit-for-bit. `--ack-timeout` guards against a hung
  bot (empty order).

## Server algorithm

```
main:
  parse args (--mode realtime|lockstep, --tick-rate, --players, --replay out.brp)
  listen TCP; the accept thread queues connections
  state = Lobby

Lobby:
  Hello → Welcome + LobbyState
  PickHero / SetReady; when every slot is ready:
    world = World::new(&cfg, cfg.rng());
    broadcast MatchStart { info: cfg.info() }; state = Playing

Playing (simulation thread):
  loop {
    // 1. gather input
    realtime: drain incoming until deadline = tick_start + 1/rate
    lockstep: wait for Ack(t) from every agent (or ack-timeout)

    // 2. validation and PlayerId → SlotId translation through Roster
    cmds = incoming
        .filter(the slot belongs to this player_id)
        .filter(world.can_see(team, target))     // anti-cheat: no clicking into fog
        .sort_by(slot, seq)
        .dedup_by(slot)                          // 1 order per slot per tick, the last one wins
    // 3. append the accepted orders to the replay
    // 4. events = world.step(&cmds)
    // 5. broadcast: teams get their view; spectators get the fogless view;
    //    events go out by Event.visible_to. The fogless frames also go into the replay
    // 6. if world.winner().is_some() { broadcast MatchOver; flush; break }
    realtime: sleep until the next tick with drift compensation
  }
```

### Order inside `world.step()`

Fixed. Changing it invalidates every recorded replay and every hash baseline.

```
1.  tick += 1
2.  apply orders → unit.order
3.  scheduled events: creep wave, neutral and hero respawns, runes
4.  status tick: buff durations, DoT, cooldowns, hp/mana regen
5.  aggro: towers and creeps pick targets by deterministic priorities
6.  order execution: movement, collision separation
7.  attacks: attack point → projectile launch / instant hit; projectile movement
8.  abilities: cast point → effect, channeling
9.  damage queue resolution: armor, magic resist, crit, block → apply
10. deaths: gold/xp by radius, respawn timers, denies
11. vision recompute, per-team fog masks
12. victory condition check
13. return the accumulated Events
```

## bota-bot

```rust
pub trait Bot {
    fn seated(&mut self, slot: Option<SlotId>);
    fn match_started(&mut self, info: &MatchInfo);
    fn on_tick(&mut self, view: &WorldView) -> Option<Order>;
    fn on_events(&mut self, _tick: u32, _events: &[EventKind]) {}
    fn on_reject(&mut self, _seq: u32, _reason: RejectReason) {}
    fn finished(&mut self, _winner: Team, _stats: &MatchStats) {}
}

pub fn play<B: Bot>(bot: &mut B, seat: &Seat) -> io::Result<Outcome>;
```

The hero is picked when the connection is made rather than returned from a callback:
picking happens in the lobby, before there is a `MatchInfo` to decide from.

`on_events` earns its place. The attack cycle is not on the wire — a `UnitView` carries
`attack_interval` but not where in the interval a unit stands — so a bot cannot tell
whether it may swing now or in forty ticks. What is on the wire is every blow that
lands: one of the bot's own says the cycle began a wind-up ago and comes round again an
interval after that. Without this the bot orders last hits it cannot take for another
second and loses them all.

### One order a tick, and saying nothing

The server keeps one order per seat per tick and the last one wins, so a want is a
single `Order` and the policy ranks its wants rather than queueing them. Re-sending the
want already standing is not free: an order cancels the recovery after a swing and calls
the creeps onto whoever gave it. So a want equal to the one in hand is not sent again
for `resend_ticks`, and two walks to spots less than `resend_drift` apart count as one
want — otherwise following a moving wave throws away the route the server laid every
tick.

### The courier

A courier is what makes the shopping list past the first trip worth having: anything
bought away from the shop falls into a stash the hero cannot reach from the lane, so
without one the only way to spend gold is to walk home for it. With one, the bot buys
wherever it stands — and only while a courier of its own is standing, because gold in
hand is worth more than an item in a stash nothing is coming for.

The errands are abilities the courier carries, so an order for one names the courier in
`ClientMsg::Order`'s `unit` and casts the slot the errand sits in. Which slot that is, is
read off the courier rather than assumed: the order the server fills its book in is the
server's business. A `Want` therefore carries whom it is for, and what the bot answers
with each tick is an `Ask` — an order and a unit — rather than an order alone.

An errand outlives the tick it was given in. Saying it again changes nothing about what
the courier does and costs the one order the seat has that tick, which is an order the
hero did not get to give: the first cut of this sent five hundred and thirty-six errands
in one match, and the hero took a quarter more damage for want of the ticks. So an errand
already under way is not repeated until `courier_repeat` ticks have passed, which is a
safety net for an order that never arrived rather than a schedule.

A trip is not made for one item: it waits until `courier_batch` of them have piled up or
the first has waited `courier_patience` ticks. And it is held back entirely while the bot
is being shot at — a courier walks to where its owner stands, and where its owner stands
is what is shooting.

An errand answers to what the courier is carrying, not only to what waits in the
stash. Sent for a stash that is empty while already holding something, it takes what it
holds on to its owner rather than flying home with it: a courier that comes to be
carrying goods it could not hand over would otherwise be sent home by the very key
meant to bring them, and the goods would ride back and forth for ever. And what an
owner has no room for is carried back to the stash rather than kept aboard, so a full
bag leaves the goods somewhere its owner can reach them rather than orbiting the lane.

Delivery also collects: having handed its load over, the courier takes every stack the
owner has marked for sale, and the put-back leg it already flies carries them to the
stash, where the sale pass cashes them. A courier with nothing to deliver still answers
the call while something is marked — the call is the ask, and refusing it would leave
marked goods stranded on a hero who cannot reach the shop.

A courier brought down keeps its load. The stacks wait on the seat while the courier is
gone and come back aboard the next one, the same way a fallen hero's bag waits on the
seat. Spilling the load on the ground was considered and turned down: the wait already
prices the death, the goods staying out of reach until the courier stands again is
punishment enough, and a bot that loses items outright learns to fear the courier
rather than to use it.

A way found round something is walked to the spot it was found for, and the spot it
was found for is what is kept beside it. Keeping the last spot asked for instead is what
made a courier fly the whole of a stale way to where its owner used to stand: a quarry
that moves a little every tick never moves far enough in one tick to look like a new
goal, so the way was never found again until its last corner had been reached. And a way
round anything is found only for what walks. What flies is over all of it, so it is
pointed straight at where it is going and keeps no route at all.

Walking at something that cannot be struck — an ally, most often, since an order aimed
at one is how creeps are shaken off — closes until the bodies touch and stops there,
following it for as long as the order stands. Aiming at the middle of a body instead is
aiming at a spot inside it, which cannot be reached: the walk presses in, the pass that
eases overlapping bodies apart pushes back out, and the two together read on the screen
as circling. Stopping a hair outside the hulls keeps that pass out of it entirely.

### The numbers held apart from the decisions

Every threshold the policy weighs — how low is low, how far is far, how many ticks a
swing takes to land — is a field of `Params` with a range, not a constant at the place
that reads it. Three things follow: a run can be handed a set from a file, a search can
walk over the set, and the numbers standing for what the wire does not carry (the wind-up,
the arrow's speed, what an ability reaches) are tuned the same way as the numbers standing
for taste. `Params` is `f32` throughout: the bot is allowed float, and uniform fields are
what let a search treat the set as a vector without a case per field.

A trained set lives in `params.txt` **beside the repository, not inside it**, and is not
committed. Weights are the same, in `weights.safetensors`. Both are read when the bot runs
rather than carried inside the binary, so a training run takes effect without a rebuild
and a machine without one plays by the numbers the code was written with.

They are not committed because of what they are. A set of numbers is what one machine's
training run happened to arrive at over a few hours against one opponent — it is an
artefact of a run, not a statement about the game, and the run that produced it is
reproducible from its seed. Committing it would put a binary blob in review that nobody
can read and everybody would have to merge.

Two sets, and the difference matters:

| | What it is | Who plays by it |
|---|---|---|
| `Params::default()` | the numbers the code was written with | the tests, `--plain`, and any machine with no kept file |
| `Params::learned()` | `params.txt` as it is on disk now | `Brain::new()`, `play`, and `train` as its starting point |

The tests pin the policy against `default()`, never against a trained set: what a test
measures is the decision, and a trained set is data that moves under it. What a test does
hold is the round trip — a knob renamed leaves an old file naming something nothing is
called, and a bot that quietly fell back to the plain numbers would play worse for no
visible reason.

### Self-play

Training is `bota-bot train`, and it needs nothing the server does not already do. A
server plays one match and exits, so a bout starts one on a port the system picks, joins
both seats, and kills it with the bout. Joining waits for the seat: seats go out in the
order the server sees connections arrive, two connections made back to back arrive in
whichever order the threads behind them run, and a set that played the same side twice is
measured against a side rather than against an opponent. That was a real bug and it made
every measurement bimodal. Lockstep is what makes it
worth doing: the server advances as soon as every seat has acknowledged the tick, so a
match runs as fast as the two brains think — twelve thousand ticks, a little under seven
minutes of game, in about three seconds. A bot that does **not** acknowledge holds the
match at the straggler timeout, one tick at a time.

The search is a (1+λ). A round breeds several challengers out of the set in hand, each
differing in a few numbers, and measures every one of them the same way: two matches
against a champion, one from each side. Whichever came out furthest ahead of the champion
takes the set in hand, if it came out further ahead than that set did. Both sides are
played because the map is not symmetric to a search: left to one side it would learn the
side rather than the game. The matches of a round are independent, so they run at once — a
round costs two matches of wall clock, not fourteen.

Measuring against a champion rather than head to head against the set in hand is what
makes a round mean something. Head to head, the thing being climbed moves under the search
every time it takes a step: a challenger that beat the set in hand says nothing about the
round before it, the number in the journal drifts, and a run of them walks rather than
climbs. That was tried first and it is what it did — twenty rounds of taking challengers
left the set no better against a fixed opponent than it started. Against something frozen,
better is one number that means the same in the first round and the hundredth.

The champion is frozen, not fixed: it starts as the numbers the code was written with and
the best set replaces it every `champion_every` rounds, so the bar rises. A search
measured against one weak opponent forever learns to beat that opponent.

How far a nudge reaches is not fixed either. It widens while more than a fifth of the
challengers beat the set in hand and narrows while fewer do: a search that keeps failing
is reaching too far, and one that nearly always succeeds is not reaching far enough. A
fifth is the old rule of thumb. What matters about it is that both halves happen —
widening on any success at all only ever widens, which is the same as not adapting.

What is scored is mostly what the seat did — creeps taken, creeps denied, levels, gold,
damage put on the other hero and damage taken — because two even bots farm for twenty
minutes and neither Ancient falls; the win itself outweighs any of it when it does come.
The damage is counted from `EventKind::Damaged` rather than read from `MatchStats`: the
final numbers arrive only when a match runs to its end, a match played for a fixed span
never does, and the server reports `hero_damage` as zero besides. Without those two columns
the whole fighting half of the policy is unconstrained — every match ends nought kills to
nought, so nothing else in the score can tell a bot that harasses from a bot that does not.

Nothing about this reaches into `bota-server`: the trainer is a process that starts other
processes, and the bot still sees only what its own side may see. A search that could see
through the fog would learn to.

`Watched` writes a line a tick — where it stood, what it had, what was near, and the order
it gave. It is what a match is read back from without watching it, and it is the shape a
policy learned from recorded play would be trained on.

### The second bot: a network that chooses

There are two bots in the crate and they share everything but the choosing. The
rule-driven one weighs its wants in a fixed order and takes the first that answers. The
other draws up the same wants, scores each with a network, and takes the highest.

Scoring candidates rather than emitting an action is what makes a network fit this game
at all. An order is parameterised — walk *there*, hit *that one* — so a fixed row of N
action classes cannot name the action space, and a head that regresses a position has to
learn from scratch that positions off the lane are worthless. Scoring sidesteps both:
which orders exist stays with the code that knows the rules, the number of them is free
to change from tick to tick, and the same weights judge a swing at one creep and a swing
at another.

A row shown to the network is the tick and one candidate laid end to end — twenty-four
numbers about the world, thirty-two about the candidate, every one of them brought to
about the same size. Two hidden layers of a hundred and twenty-eight, one number out:
some twenty-four thousand weights. The library is behind one file, `net/model.rs`, so
what the bot decides does not depend on which tensor crate is underneath.

### Teaching it, in two halves

**Copying first.** A network started from nothing spends a very long time discovering
that walking into a tower is bad, and every match costs seconds. It does not have to:
there is already a bot that plays a respectable lane, and every order it gives is an
answer to a question the network will be asked. So the first half is not a search at all.
Play matches, write down the candidates and which was taken, and move the weights until
the network takes the same one. Thirty-one thousand decisions and four passes — about a
minute — gets it agreeing with the rules **87%** of the time, and playing level with them.
Chance is one in twenty-four.

How faithful the copy is turns out to decide everything. Thirty-one thousand decisions
and four passes gets 87% agreement, and a network that agrees seven times in eight plays
*worse* than what it copied — errors compound, and a lane is unforgiving. Sixty-two
thousand and ten passes gets **92.6%**, and that one plays better than what it copied. The
gap between those two runs is the difference between a second bot that is a curiosity and
one that is worth keeping.

That the candidate list holds what the rule bot chose is checked and reported rather than
assumed: **98.7%** of its orders are candidates the network could have picked. Whatever is
short of that is behaviour the network cannot be taught, and a number that drops after a
change to either bot says so.

**Practising second.** Copying cannot beat what it copied. The second half plays the
network against **a frozen greedy copy of itself** on the same seed — one side wandering,
one side taking what it already believes — and moves the weights towards the wandering
choices of matches where wandering paid.

The frozen side is the whole trick, and it was learned the hard way. The first cut scored
each seat against the average of the generation, which sounds reasonable and is nearly
noise: both seats play the same weights, so half of them come out above average whatever
they did. Sharpening towards those halves sharpens randomness, and it showed — over five
rounds the loss fell from 0.121 to 0.057 while the matches got *worse*, 44 down to 33. A
policy agreeing with itself ever harder looks exactly like a policy learning. Against a
side that made no unusual choices on the same seed, the difference is what the unusual
choices were worth, which is the thing being asked.

**Measure against something that does not move.** The second thing learned the hard way,
and the same lesson the search over the numbers taught: a round's own matches are worth
whatever their seeds were worth, so reading that number as progress is reading the luck of
the draw. Every fifth round the network plays the rule-driven bot greedily, on the same
handful of seeds every time, from both sides. That margin is the only number in a run that
means the same thing in the first round and the last — and it is what showed that the 87%
copy was twenty-odd points *behind* while its own matches looked fine. From the better
copy, which starts level and a little ahead, the margin climbs: +17 at five rounds, +26 at
ten. The share of wandering that pays falls as it goes, from three quarters towards a
third, which is what a policy absorbing its own good accidents looks like.

Two smaller things keep it honest. Each seed is played twice with the sides swapped, so
nothing learned is a fact about which end of the map a seat began at. And a slice of what
was copied is gone over again every round: a policy taught only from its own recent
matches forgets the parts of the game those matches did not visit, and there is nothing
in a lane to remind it.

The weights are kept in `weights.safetensors` beside the repository, on the same rule as
the numbers: read when the bot runs, not committed, not carried inside the binary. Asked to
play by weights that are not there, the bot says so rather than playing something else.

Credit inside a match is handed out per decision: what followed it over the next while,
discounted so the near future counts for most of it, judged against what other decisions
taken at about the same point in a match were worth. Early ticks pay little and late ticks
pay much whatever is chosen, so comparing against the run of all decisions would only be
measuring the clock.

Whether that is better than giving every decision the match's score is **not settled**. Run
against run from the same weights and seed, the blunt scheme reached +23.4 against the rule
bot and the per-decision scheme +23.6 — a tie. What the per-decision scheme did need before
it could even tie was a fix to something else: the network used to write down a decision on
every tick, including the thousands where it was standing by what it had already said.
Those frames are near-duplicates of their neighbours, their returns differ by noise, and
learning the difference is learning nothing. Recording only the ticks that actually issue an
order — which is what watching the rule bot always did — cut the frames fivefold and brought
the scheme from +17.6 back to parity.

### Two bots can each be better than the other

The net that came out of that run is +35.8 against the rule bot, the best measured, and
−4.6 against the clone it was itself trained from, losing every match. Both verdicts are
twenty matches with the doubt on them under five.

This is not a contradiction to be explained away. Practising against a frozen copy of
oneself is training to beat *that opponent*, and a policy can get better at one style while
getting worse against another. It means a single number cannot say which of two bots is
stronger — only which is stronger against what it was measured on. The honest fix is a pool
of opponents rather than one, which the arena is already shaped for; until then, a verdict
here should always be read with its opponent named.

A forward model (rolling out hypothetical futures for planning) is not supported and
not needed now: the bot has no hidden state, so it could not roll forward with the
real engine anyway. If it becomes needed, the simulation moves out of `bota-server`
into its own crate, which is cheap since `sim/` is already a separate subtree with no
dependencies on the network layer.

### Playing without a socket

Most of a match is not the match. Measured on one match of twenty thousand ticks: the world
itself steps in 231 ms, projecting it through the fog for both sides costs 46 ms, the model
decides in 3363 ms — and the remaining 5812 ms is postcard, TCP, two processes and the
lockstep acks between them. Three per cent of the wall clock is the game.

So `bota-bot-v2` can play its matches in its own process. All of that lives in one module,
`bench.rs`, which is the only place in the bot that names `bota-server` at all; the server
is not changed for it and does not know it exists. A bench hands over the view of that
seat's own side, fog and all, which is byte for byte what the socket would have carried.
Reading the world directly would be faster still and would train a bot that cannot play,
having learned on what a seat is never shown.

The dependency is behind the `builtin` feature, so a bot built without it carries no
simulation. That the bot could reach further into `bota-server` than `bench.rs` does is a
matter of one module's discipline rather than of the compiler's — the cost of not bending
the server around the bot.

There is one seat loop, not two. `play_on` takes anything that answers three questions —
hear, order, done thinking — and a socket and a chair both do. Two loops would part company
by the second change to either, and then the model would be trained on one game and played
on another.

The gain is real and smaller than it looks from the breakdown: **about a quarter**, not the
two and a half times the numbers above suggest. The server's share of that 5812 ms runs on
another core, in parallel with the bot's thinking, so removing it does not remove wall clock
one for one. What it does remove is a process and a socket per match, which is what matters
once there are more lanes than cores.

Two things had to be got exactly right, and both were found by comparing the two paths on
the same seeds rather than by reading the code.

**A tick waits for every seat that is still there.** The seats' acks started life meaning
"has thought about everything", so a seat that had not spoken yet did not hold the tick and
the world walked on without it. The match still ran, and still repeated itself when run
twice in a row, which is what made it look right.

**There is no snapshot of the tick a match begins on.** The server gathers orders, advances,
and only then sends, so the first snapshot a seat ever sees is of tick one. An arena that
handed out tick zero put every seat a tick ahead of itself for the whole match. The first
three lessons agreed to the mark either way — they are short — and everything from three
thousand ticks on drifted apart.

With both fixed, one model scored identically on all seven lessons down to the last tenth,
by both paths. That equality is the whole warrant for training on the fast one.

### Hanging up

A connection is closed one half at a time: the writer shuts down the sending side and
leaves the receiving side open. Closing both while the peer still has bytes of ours in
flight resets the connection, and a reset discards what was already sent — so the peer
loses the message saying who won and sees an aborted socket instead of a result. It cost
an afternoon to find, because the seat reported it as a connection error at a tick number
that looked like a length limit.

Two things around it are part of the same lesson. The server waits for its writer threads
rather than exiting from under them, since the last message of a match is queued at the
moment the server has nothing left to do. And a harness that starts a server reads its
error output rather than discarding it: thrown away, a server that panics reaches the
caller as a socket that closed, naming neither the server nor the panic.

## bota-bot-v2

A second bot, built the other way round. The first one weighs candidates the rules drew
up; this one is handed a fixed vector and a fixed numbered list of deeds, and names one.
Nothing of the first is reused: a bot that depended on another would mean every future bot
carrying every past one about with it.

The whole contract is four pieces.

| | |
|---|---|
| `field.rs` | one tick read into a settled shape: who is who, in what order, seen from where |
| `sight.rs` | **156 numbers** built from that |
| `deed.rs` | **62 deeds**, flat and numbered |
| `doing.rs` | which of them may be done now, and what a chosen number turns into |
| `marks.rs` | what a tick is worth, lesson by lesson |

Between the numbers and the choice sits a `Mind`: handed numbers and flags, answers with
one number. That is the whole seam. A mind that knows what a creep is has reached across
it, and a game that knows what a weight is has reached back.

### Why the reading of a tick is its own piece

Because the vector and the decoder have to agree about which creep is the third one. Read
the tick twice and they drift, and every hour of training above them is learning noise.
For the same reason the order has a tie-break on the handle: two creeps the same distance
off would otherwise swap places from tick to tick, and the number that named one would
name the other.

### Turned about

Forward is always towards the other side's fountain and left is always left of that. In
world coordinates a bot would learn the game twice, once from each corner of a map that is
a mirror of itself.

### Legality is ours, not the model's

Every tick carries a flag per deed. The model never picks one that is false — the numbers
of the impossible are sent to nothing before anything is compared, so they can never come
out on top and never carry a gradient. Letting it pick freely and taking the points off
afterwards was considered and dropped: there is one order a tick, so a wasted pick is a
lost creep, and what is legal is known to us for nothing.

The counting goes both ways, and it earned its keep immediately. The first match run this
way had the model choose nothing illegal — and the **server refuse eleven hundred of its
orders out of eight thousand**. What the bot believed about legality and what the server
enforced were not the same thing: a snapshot carries an ability's level and its wait but
not whether it can be cast at all or what it must be aimed at, and the bot was offering
casts of passives and bolts aimed at the ground. With that written down in `spells.rs` the
refusals went to nought. A bot that had only counted its own mask would have called itself
correct and quietly thrown away one tick in eight.

### What of the ground made it into the list

Of the item orders the wire grew later, the list took only selling: one deed per
inventory slot, marking far out and cashing at the shop, so the whole trip is the sell
deed plus the deliver errand it already had. Laying an item down and taking one up are
not deeds. In selfplay nothing ever lies on the ground — a courier keeps its load
through death and no deed drops anything — so a take-deed would be a logit that is
masked on every tick of every match, and a put-deed a way to burn a tick and half an
item's price. The list is append-only exactly so that either can be added the day
something puts loot on the ground in front of a bot.

### The model

Two heads over one trunk of two layers: a number per deed, and one number for what the
position is worth whatever is chosen. Some **232 thousand weights** against the first
bot's twenty-four.

The value head is not decoration. Judging a decision needs something to judge it against,
and the first bot's home-made baselines — the match's own score, then the average of
decisions at the same point on the clock — were measured against each other and came out a
tie. A learned value is the answer the tie was pointing at.

It is shown seven ticks laid end to end rather than only the newest, because a swing that
has begun, a creep about to die and a creep just dead look alike in one frame. Frames
rather than a memory of its own: a memory carried through twelve thousand ticks and reset
on every death costs more to train than that is worth. If a measured gap ever asks for
recurrence, the seam is the place to put it and nothing above or below would notice.

**The frames are spaced by doubling ages, not taken consecutively.** Half a second back,
then one, two, four, eight and sixteen seconds, and the tick being decided on. Four
consecutive ticks reached an eighth of a second, which is enough to see a swing land and
nothing else: whether a wave is being pushed, whether the other hero has been closing for
the last ten seconds, whether the bot has been standing in the same place since it walked
there — all of it happens on a scale the old window could not reach. Doubling buys a
quarter of a minute for three more frames, at the price of resolution the far end does not
need.

**Ages are the match's own ticks, not how many times the model was asked.** The seat
chooses nothing while there is nothing to choose, which is mostly being dead, so counting
calls would let a death quietly stretch a sixteen-second window into a minute. Each frame
is the newest tick seen at or before its age, so a gap reads as the last thing the seat
actually saw rather than sliding the other frames along. One frame from beyond the window
is kept for exactly that reason — after a long gap the oldest age asks for a tick older
than the window itself.

The price is three more frames on the first layer: `INPUT` goes from `NUMBERS x 4` to
`NUMBERS x 7`, 624 numbers to 1092, and the model from 240441 weights to 360249. It bought
them for nothing. A forward pass measured 23.5 us before and 22.5 us after — half again as
many weights and no more time, because at a batch of one the pass is bound by the ten
candle operations it is made of and not by the arithmetic inside them. The same reason a
GPU would lose here is the reason this was free.

Weights trained against the old input cannot be loaded against the new one, and are not
silently reshaped: `shape mismatch in set, lhs: [1092, 256], rhs: [624, 256]`, and the
load fails.

### Lessons

A match pays almost nothing almost all of the time, so the bot is not asked to learn the
game at once. Seven lessons, each a longer match than the last, each paid for something
narrower than winning.

| | ticks | scored in |
|---|---|---|
| stock up | 300 | `marks/stock_up.rs` |
| find the lane | 900 | `marks/find_the_lane.rs` |
| hold the lane | 1200 | `marks/hold_the_lane.rs` |
| meet the wave | 3000 | `marks/meet_the_wave.rs` |
| work the lane | 12600 | `marks/work_the_lane.rs` |
| take the towers | 36000 | `marks/take_the_towers.rs` |
| grow rich | 54000 | `marks/grow_rich.rs` |

**A lesson is one file and one function.** How long it runs is its row of `LADDER`; what it
pays for is the file that row names, weights and all. `score` is the only place in the
crate that branches on which lesson is being taught, and it does nothing but hand the tick
to that lesson's own function. There used to be seven such branches, spread over standing,
walking, blows, buying and fighting, and reading what one lesson was worth meant reading
all of them.

**A lesson is scored once a tick.** The seat holds a tick's events until the next snapshot
says what the tick came to, then scores it whole. Two entry points — one for the snapshot,
one for the events — would force every lesson to be cut in half along a seam that is the
wire's, not the lesson's.

**A lesson's marks are its own.** Nothing a lesson pays for depends on what an earlier one
taught. Lessons used to keep a quarter of the habit before them, and it was measured not to
work: at the last rung a quarter of the shopping habit is one mark against three hundred,
which no selection can see, and every bred model had forgotten how to shop by the end of
the ladder. Rescaling the quarter would have been a knob to guess; dropping it is one less
thing that can be wrong.

Isolated marks are also readable all at once, which is how they are now read. One match,
run to the longest lesson's clock, is scored by every lesson: each counts the ticks inside
its own window and stops. The whole ladder for the price of its longest rung, and a card
that describes one game rather than seven different ones.

The last rung is net worth itself — unspent gold plus what everything owned cost — paid a
tick at a time as the difference since the tick before, which adds up to what the seat
ended up worth less what it started with. Downwards as well, so gold lost on dying is net
worth lost. One mark a gold, which puts the number an order of magnitude above every other
lesson's; that is harmless because a lesson's marks are never added to another's, and it
means the number reported is net worth and not a scaled shadow of it.

Four decisions inside the marks were settled by measuring, each after a run that learned
nothing or learned the wrong thing.

**No flat floor.** Nearness was a straight slope from full marks at six hundred units to
nothing at three thousand. Past three thousand every position scored the same nothing, so a
bot that had wandered off had nothing in the numbers pointing home. It is now a falloff
that halves at six hundred and never reaches zero.

**Not the line — the spot.** The first lane lesson paid for standing near the line its lane
runs along. A fountain is on that line. Doing nothing whatsoever scored 8.9 out of a ceiling
of 9.0; the lesson now pays for the spot halfway along, where the waves meet.

**Ground closed, not ground left.** Paid for nearness alone, the same lesson stuck at 0.5
out of 9.0 for thirty rounds: a random walk cannot cross most of a lane in thirty seconds,
and until it does, nothing it does changes what it is paid. Marks go to the distance
*closed* since the last tick, which pays from the first step in the right direction. With
that the lesson went 3.6 to 10.0 in ten rounds.

**Blows count only against the other side.** Paid for the swing alone, the gradient trainer
found what the wording allowed and went all the way into it: nought enemy creeps killed a
match and twenty-four of its own, because its own wave is always beside it and never fights
back. Closing the wording moved the same trainer from nought last hits to fifty-three and
from twenty-four denies to none.

The last rung pays for four things at once: damage to their towers, a tower falling,
killing them, and staying whole, with dying counted against it. A tower is worth more the
earlier it falls, by a falloff that halves at five minutes, and worth more for every one
already taken, so the second is twice the first. Two departures from the plain reading of
"damage over time, times towers taken": multiplying the whole score by the towers taken
makes every point of damage before the first tower worth exactly nothing, which is the flat
plateau again, so damage pays on its own and the multiplier applies to the towers alone;
and dividing by the clock makes a tower taken in the first seconds worth unboundedly more
than one taken a minute in, so a falloff is used instead. Health and mana are paid for only
outside its own base — paid wherever it stood, the surest route to full health, full mana
and no deaths at all is never to leave the fountain.

Spending is read off what the seat owns — the bag, the stash and the courier's load, each
item at its price — rather than off the gold falling, which also falls on death and rises
on its own. Only increases count; selling gold back is not spending it.

### Grow strong

The eighth rung is net worth again, with the purse counted at half its face value. A gold
is paid for twice over, half each time: half when it is earned, and the other half when it
is turned into something. Gold that is never spent is never paid its second half.

It exists because **`grow rich` cannot tell hoarding from wearing.** Net worth counts the
purse at face value, so buying a five-hundred item moves five hundred from one side of the
sum to the other and the number does not move. Two hands that end a match on the same net
worth score identically, whether one of them is wearing it and the other sitting on it —
and one of those two is a hero and the other is a wallet. A test asserts exactly that gap:
the same five hundred, `grow rich` paying both hands alike and `grow strong` paying the one
that spent it twice as much.

That `grow rich` will *eventually* reward spending is true and useless. Items win fights,
fights win farm, farm is net worth — but that is four causal steps, and a breeding search
that gets one number per match will not find it. The half is a direct signal for what the
long chain only implies.

**A half rather than nothing.** Counting only the goods would be `stock up` without a
ceiling: earning gold would pay nothing at all until it was spent, and dying with a full
purse would cost nothing, which is not true of a hero. A half keeps both ends — income is
worth something the moment it arrives, and a death still costs.

**It is the longest rung, not `grow rich`'s equal.** Two rungs of the same clock break
three things the ladder promises at once: that each runs longer than the last, that exactly
one lesson is still counting when a match ends, and that `Lesson::longest()` names one
lesson rather than whichever of two ties `max_by_key` happens to return. Five minutes more
is what it costs to avoid weakening all three, and in release that is about six minutes
across a whole ladder.

**`grow rich` stays.** A card scores every lesson off one match, so keeping both means every
report says what the same game was worth on each counting, and the gap between the two
numbers is exactly how much gold the bot is sitting on.

### Breeding

Lessons are taught by breeding rather than by gradient. A crowd of models plays the
lesson, the best are kept, and the crowd is refilled by copying them with noise added.
The crowd carries over from one lesson to the next.

What decided it was not determinism but this: **what is improved and what is reported
become the same number.** Under gradient they were two things and they came apart twice
in measurement — a run whose loose play climbed from 49 to 88 while its greedy play fell
from 63 to 21, and a lesson that sat at its starting mark for sixty rounds. Breeding
scores the match, and the match is also the report.

It also deletes seven numbers nobody could check: the discount, the window a decision is
credited over, the value head's share of the loss, the entropy bonus, the heat, the step
size and the batch. One of them was already known to be wrong — at the wave lesson the
loss ran to 2.6 because a creep pays ten and the value head's error swamped the policy's.
What replaces them is four with plain meanings: how many models, how many matches each,
how many survive, how far a child moves. `step.rs`, `roll.rs`, `adam.rs` and the value
head all go once breeding is shown to be better, and the gradient trainer stays under
`descend` until then.

Three decisions inside it.

**The trial seeds move with the generation.** A crowd judged on the same matches every
generation is a crowd selected for those matches, and with two hundred thousand numbers to
play with it will learn them rather than the game. Seeds are a function of which
generation it is, so a run still repeats to the byte — two runs of one seed were checked
to produce identical logs and identical weights — while no model is asked twice to do well
at the same match. A separate set that never moves is used to report and never to choose,
and a test asserts the two sets never meet.

**Children are handed round the survivors in turn** rather than heaped on the winner,
or a crowd becomes one model and its copies before a lesson has finished asking anything
of it.

**Ties never swap.** Two models worth the same keep the order they had, and a match that
came to nothing does not shuffle the crowd. Without that a run is not repeatable.

The cost is known and was measured before building: breeding gets one number per match
where gradient gets one per decision, so it needs roughly a hundred times the matches. At
thirteen matches a second that is fine for the short rungs and marginal for the last,
where a match is twelve thousand ticks.

### The plan

`bota-bot train <FILE>` follows a plan: a YAML file naming a sequence of stages, each a
lesson and the crowd bred at it. `train.yaml` beside the crates is the ladder as it
stands, so the built-in run is a file rather than a branch.

It replaces a subcommand that took nine flags and applied all nine to all seven rungs.
Every real run wanted otherwise — a bigger crowd on the late rungs, more matches where
the variance is worst, a short clock while a lesson's marks are being read — and getting
it meant seven invocations chained by a shell loop, which is a ladder nobody else can
walk and one nothing records. A plan is the run, and it is a file that can be committed
beside the weights it produced.

**Field names are the ordinary ones**, not the crate's: `population`, `generations`,
`survivors`, `mutation`, `matches`. Inside, those are `folk`, `lives`, `keep`, `spread`,
`trials`, and the translation lives in `plan.rs` alone. The crate's vocabulary is worth
having where the code reasons about a crowd; a file somebody writes by hand at two in the
morning is not that place.

**A stage may say nothing but `score`.** What it leaves out comes from the plan's
`defaults`, and what those leave out comes from the lesson's own rung and a plain crowd.
Seven stages differing in one number each is the common case, and repeating nine fields
seven times is how a plan comes to disagree with itself.

**A stage's `ticks` moves the scoring window, not only the match limit.** A lesson used to
stop paying at its rung's tick count wherever the match ended, so a stage asking for a
longer clock than its rung would have run the extra ticks for nothing and reported the
same mark — a knob that silently does half of what it says. `Marker` now carries a window
per lesson instead of reading `LADDER`, and the taught lesson's is the stage's.

**The whole plan is checked before the first match.** Every stage is settled up front and
anything nonsense — a population of one, no survivors, a lesson nobody has heard of, a
field spelled wrong — is an error naming the stage. A run of several hours that stops on
its sixth stage over a typo has thrown away the five before it.

**Unknown fields are refused.** `serde(deny_unknown_fields)` on both, so `populaton: 40`
is an error rather than a setting silently ignored and a run that reads as though it did
what was asked.

**Where the weights live is the command's business, not the plan's.** A plan says what to
teach; `--weights` says which model is being taught, defaulting to the standing file. Kept
in the file, the same teaching run against a fresh model and against last week's would be
two plans differing in one line that has nothing to do with teaching.

**A run continues.** When the weights file already exists the crowd starts from it — the
kept body itself and children moved off it, exactly as a generation refills — rather than
from noise. Before this `train` always drew a fresh crowd and the file was only ever
written, so a ladder taught in the morning could not be taught further in the evening; the
gradient trainer already continued from what was there, and the two now agree.

`serde_yaml` is the eighth dependency. It is deprecated upstream and pinned at 0.9.34
knowing that: YAML here is a shallow map of scalars read once at startup, the crate is
frozen rather than abandoned, and nothing in the simulation touches it. Replacing it is
`plan.rs` and nothing else.

### What is not built yet

The other half of training. The design constraint that decides whether it is worth building at all: **every
deed the rule-driven bot can take must be one number in this list**, so that the first half
of training is copying a bot that already plays a respectable lane rather than a search
from nothing. That was measured on the first bot — a clone at 87% agreement played worse
than what it copied, at 92.6% it played better, and widening the choice beyond what the
teacher demonstrates cost twenty points. A second bot that threw the teacher away would be
starting a search at our budget of one match a second, which is where months go.

## Stages

| # | Deliverable | Contents |
|---|---|---|
| 0 | ✅ workspace builds | Cargo.toml, `bota-proto`, workspace lint policy |
| 1 | ✅ `bota-proto` types | `Fixed`, `Angle`, `Vec2`, identifiers, `Order`, `EventKind`, `WorldView`, messages |
| 2 | ✅ codec | serde derive, postcard, framing, `FrameReader` |
| 3 | ✅ `bota-proto` tests | round-trip of every message, torn stream into `FrameReader`, snapshot size budget |
| 4 | ✅ `Fixed` arithmetic | Q16.16 ops through an intermediate `i64`, `Vec2`, `distance_squared` in raw Q32.32, tests in debug and release |
| 5 | ✅ `World` ticks | arenas, units, movement, orders, creeps, towers, Ancient; `rng.rs` with streams and Ratio/Chance |
| 6 | ✅ combat | attacks, projectiles, damage, deaths, gold/xp, victory |
| 7 | ✅ server networking | lobby, both tick modes, snapshot broadcast, replay recording |
| 8 | 🔄 `bota-bot` and `bota-client` | SDK + bot, bot-vs-bot match, self-play. Done: the client — macroquad: map, units, HP bars, orders, lobby, spectating, replay playback; the bot — lane policy over tunable numbers, lockstep acks, hill-climbing trainer |
| 9 | hero Sylla complete and determinism test | abilities, levels, items, shop; 20 000-tick hash baseline, run on musl/wasm32; mirror test: a diagonally mirrored match ends in the mirrored outcome |
