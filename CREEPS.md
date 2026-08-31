# Creep system — agreed plan

Status: **in progress.** Steps 1 to 5 of section 6 are in the tree; 6 to 10
are not. Sections 1, 3, 5 and 7 describe what is being built and are kept in
step with the code; section 2 marks what is already closed. When section 6 is
finished the facts fold into `DESIGN.md` and this file goes away.

Target: Dota 2 **7.41e** (Summer Scrub, August 2026 — the current gameplay
patch).

## 0. Sources

Every number below is either read out of the shipped game data or taken from
the mechanics wiki. Nothing is guessed except where it says **[approximation]**.

| Source | What came from it |
|---|---|
| `scripts/npc/npc_units.txt`, client 6874 (2026-07-31) | every unit stat: health, damage range, armour, BAT, attack point, acquisition range, attack range, projectile speed, bounty, hull, vision, move speed, turn rate |
| `scripts/npc/npc_abilities.txt`, same build | `creep_irresolute`, `creep_piercing`, `creep_siege`, `flagbearer_creep_aura_effect` |
| Dota 2 Wiki, *Lane Creeps* | spawn schedule, wave composition, upgrade table, aggro rules, target priority, chase and return behaviour |
| Dota 2 Wiki, *Neutral Creeps* | camp categories, spawn rule, aggro radii, guard distance, leash timers, lane-creep interaction |
| Valve Developer Community, *BoundsHullName Size Reference* | hull radii |

---

## 1. The ruleset

### 1.1 Lane creep stats, wave 1

Hull radius is the collision radius: what other units cannot enter.

| | Melee | Flagbearer | Ranged | Siege |
|---|---|---|---|---|
| Health | 550 | 550 | 300 | 935 |
| Health regen | 0.5 | 0.5 | 2 | 0 |
| Armour | 2 | 2 | 0 | 0 |
| Magic resistance | 0 % | 40 % | 0 % | 80 % |
| Attack damage | 19–23 | 19–23 | 21–26 | 35–46 |
| BAT | 1.0 s | 1.0 s | 1.0 s | 3.0 s |
| Attack point | 0.467 s | 0.467 s | 0.5 s | 0.7 s |
| Attack range | 100 | 100 | 500 | 690 |
| Projectile speed | — | — | 900 | 1100 |
| **Acquisition range** | **500** | **500** | **600** | **800** |
| Move speed | 325 | 325 | 325 | 325 |
| Turn rate | 0.5 | 0.5 | 0.5 | 0.5 |
| Hull radius | **16** | **16** | **8** | **16** |
| Vision, day = night | 750 | 750 | 750 | 750 |
| Gold bounty | 34–39 | 34–39 | 43–52 | 59–72 |
| XP bounty | 57 | 57 | 69 | 88 |

Attack modifiers, for the record — see §7 for which of these ship now:

- `creep_irresolute` — melee and flagbearer: **−25 %** damage to heroes.
- `creep_piercing` — ranged: **+50 %** to creeps, **−50 %** to heroes,
  **−50 %** to heavy targets.
- `creep_siege` — siege: **+150 %** to buildings; incoming **−50 %** from
  heroes, **−30 %** from basic sources, **−40 %** from player-controlled units.
- `flagbearer_creep_aura_effect` — 700 radius, +3 health regen to allies;
  +10 gold and +3 XP area bounty within 1200 on death by an enemy player;
  magic resistance grows +4 % per 7.5-minute interval, capped at 15 intervals.

### 1.2 Spawning

- First wave at **0:00**, then every **30 s**. Ranged behind melee; siege
  between the melee creeps.
- Base wave: **3 melee + 1 ranged**.
- **Flagbearer**: from wave **5**, then every **2nd** wave. It *replaces* a
  random melee creep — wave size does not change.
- **Siege**: from wave **11**, then every **10th** wave (first at 5:00).
- Count growth: wave 31 (15:00) → 4 melee; wave 61 (30:00) → 5 melee;
  wave 71 (35:00) → 2 siege; wave 81 (40:00) → 2 ranged; wave 91 (45:00) →
  6 melee.

### 1.3 Stat upgrades

Every **7:30**, applied to *newly spawned* creeps, **30 times maximum**
(fully upgraded at 225:00):

- melee: +12 health, +1 attack damage, +1 gold
- ranged: +12 health, +2 attack damage, +6 gold, +8 XP
- siege: no periodic upgrade
- flagbearer: no upgrade

### 1.4 Lane creep behaviour

- A creep walks its lane's fixed path and **never leaves it on its own**. It
  moves aggressively — the same rule as a player's attack-move.
- It engages **the closest** hostile unit inside its acquisition range.
- A held target is **kept**. Distance alone never takes a creep off it: walking
  a hero up to a creep that is busy with another creep steals nothing, and that
  is what makes laning possible at all. A creep looks again in exactly three
  cases:
  - it lost the target — dead, gone, or out of sight;
  - something of a **better class** came into its **attack range**. A creep
    chewing on a building drops it the moment a unit it can hit arrives, per
    §1.5;
  - the held target left the creep's **attack range**, whatever it is. A creep
    never abandons what it is hitting: a ranged creep shooting a hero keeps
    shooting it however close a creep stands, and the same holds for a creep
    target. Out of reach it weighs its options again, and whatever it can hit
    wins. Only a click makes an out-of-reach hero stick, and only for its three
    seconds.
- Target entered fog → walk to the last seen spot; still nothing → return.
- Target outside acquisition range → chase at most **2.3 s**, then return.
- Return is to **the point where the creep left its lane**, not the nearest
  point of the lane. A creep never joins another lane, however close.
- A creep that never left its lane has nothing to return to: it simply resumes
  the march from where it stands. Only a creep dragged off the lane walks
  back.
- Disarmed → stands completely still and ignores everything.

### 1.5 Target priority

Priority is **class first, then distance**. Classes, best first:

1. heroes and ordinary units
2. siege creeps
3. buildings
4. wards

Within a class the **closest** wins. Among heroes at about equal distance:

1. a hero with an attack order on this creep's side
2. a hero with no attack order, or one attacking the third faction
3. a hero attacking its own allies

Non-hero units at about equal distance are all equal regardless of behaviour.
A creep never prefers a distant attacker over a close bystander.

Siege creeps use the same system with a different class order: **buildings
first**, then enemy siege creeps, then everything else, then wards. A building
entering a siege creep's attack range takes it off its current target at once.

**Order aggro.** An attack order alone aggroes or de-aggroes, whether the
attack happens or not and however far the ordered target is:

- attack order on an enemy hero → that hero's *enemy* creeps within **their
  own acquisition range** of the ordering hero **switch their target to that
  hero outright**. Not a re-ranking: the ordering hero wins even with a closer
  creep standing right next to them, which is what makes the pull work at all.
- attack order on an allied unit → the same creeps put that hero **last**,
  however close it stands: anyone else in acquisition range is taken first,
  and the wave lets go even when the hero is by far the nearest thing to it.
  Last, not struck off — with nobody else in range the hero is taken again in
  the same tick, so the creep is never left standing with no target.
- an attack order on an enemy **creep** is a last hit and moves nobody
- **3 s cooldown** per creep on both
- the switch **holds for 3 s** — **[approximation]**, see §7 — and this hold is
  the only thing that makes a hero target stick at all. When it runs out the
  creep weighs its options again; the ordering hero can still win that on
  §1.5's tie-break while it keeps swinging at the creep's own side, which is
  why shedding a wave takes either distance or a click at an ally.

Towers are outside this spec and keep Dota's own tower rule: a dive draws the
tower outright rather than through the tie band, and an order at an ally sheds
it at once, however recently it was drawn.

**Before 5:00** a lane creep cannot be aggroed by player units at all, unless
it already has an enemy lane creep or a neutral creep inside its acquisition
range, or it stands within 1500 of its own tier-1 tower. This bites rarely:
once the waves have met, every creep has an enemy creep in range. Letting go is
not restricted — the rule is about being called on.

### 1.6 Neutral creeps

- **Spawn** at **1:00**, then every minute, only when the camp box is empty of
  units. That one rule is both camp blocking and camp stacking.
- A camp never spawns the same roster twice in a row.
- **Aggro** is drawn two ways only:
  - a hostile unit comes within **240** of the neutral
  - damage or a single-target spell from within **1800**
- Aggroed neutrals then follow §1.5 — closest target, same class order. One
  extra rule: a hero inside the aggro range issuing an attack order on a hero
  of the *other* faction makes the neutrals switch to the *ordered* hero.
- Untargetable units cannot aggro neutrals. Damage from an invisible unit makes
  that neutral and every neutral within 500 **flee** to a random spot 750 from
  the camp for 5 s.
- **Guard distance 400.** Once further than 400 from its spawn spot a **5 s**
  timer runs; on expiry the neutral loses aggro and walks home. Coming back
  inside 400 resets it. Effective chase distance is 1750–2200.
- After a leash break: proximity cannot re-aggro until the neutral is home;
  damage cannot re-aggro for **3 s**. Damage after those 3 s re-aggroes with a
  **3 s** window instead of 5. Once the whole camp is home, 5 s again.
- A target turning untargetable drops aggro immediately; the neutral is still
  aggressive on the way home.
- Upgrades every **7:30**, **30 times**, applied to **living** creeps as well:
  +30 health, +0.5 armour, +3 attack damage, +5 attack speed, +1 gold, +5 XP.
- Towers never attack neutrals.
- Night: aggro range 0. bota has no day cycle — see §7.

### 1.6.1 Returning to the camp

This is the state machine that decides how far neutrals can be dragged, so it
is written out in full rather than left as one bullet.

A neutral tracks **its own spawn spot**, not the camp centre. Three numbers
govern the whole thing: the **guard distance 400**, the **aggro window** (5 s,
or 3 s when re-aggroed early), and the **re-aggro block 3 s**.

```
inside 400 of the spawn spot   -> the aggro window is held full, timer idle
beyond 400                     -> the window counts down
crossing back inside 400       -> the window resets to full
window hits zero               -> aggro dropped, walk home, and:
                                    proximity cannot re-aggro until home
                                    damage cannot re-aggro for 3 s
                                    the next window will be 3 s, not 5 s
whole camp back on its spots   -> the next window is 5 s again
```

Consequences that fall straight out of it and need no extra rule:

- The reachable chase distance is `400 + window * move_speed`, so
  **1750–2200** for a 270–360 speed neutral that only chases. A neutral that
  stops to attack covers less.
- The camp a neutral is dragged towards is irrelevant; only the distance from
  its own spawn spot counts.
- Arriving home does **not** restore health. bota currently heals to full,
  which is why jungle farming is free today.
- A neutral walking home is still aggressive: it acquires anything inside its
  acquisition range on the way, and that does not touch the timer.

### 1.6.2 Which neutrals lane creeps will fight

Your read is that this is purely a distance effect — that neutrals simply
cannot be dragged far enough from most camps, and no camp condition exists.
I checked, because it is the more economical explanation. It is not what the
game does: there are **two independent rules**, and the camp one is Valve's,
introduced in 7.23b (Outlanders). The patch line reads

> Neutrals' lane creep aggro is now based on which neutral spawn area they're
> in (enabled for the traditional safelane/offlane pull camps).

and the current wiki still describes it:

> The only neutral creeps which lane creeps attack are the ones from the small
> camps, and the large within the main jungles at the off lanes. [...] Neutral
> creeps from all other camps are completely ignored by lane creeps. However,
> neutral creeps can always attack lane creeps, no matter where they are from.
> Even when attacked by the neutrals, the lane creeps still ignore them if they
> do not come from the mentioned four camps.

So the two rules do different jobs:

- **§1.6.1 guard distance** decides how far a neutral can be *dragged*. It
  applies to every camp.
- **the spawn-area flag** decides whether a lane creep will *target* a neutral
  at all. It applies to four camps: one small camp and one large camp per side,
  the traditional safelane and offlane pull camps.

The distinguishing observation, if you want to check it in a game: drag a
medium or ancient camp's neutrals into a lane so they start hitting your lane
creeps. Under your model the lane creeps fight back; under Valve's they keep
walking and let themselves be chewed on. The wiki asserts the second, twice
and explicitly.

**Settled by the map itself.** The map's `npc_dota_neutral_spawner` entities
carry an `AggroType` field, and exactly four of the twenty-eight have it set
to one: `neutralcamp_good_1` and `neutralcamp_evil_2`, both small, and
`neutralcamp_good_2` and `neutralcamp_evil_1`, both large. One small and one
large per side, which is the wiki's sentence word for word. It is a per-camp
flag, not a distance effect.

The rest of this section is kept for the record:

- The camp flag ships as a single `pullable: bool` per camp in `camp.rs`, read
  in exactly one place — the hostility function in `acquire.rs`.
- Setting every camp `pullable: true` reduces the behaviour to your model
  exactly, with the guard distance doing all the work.
- `pull.rs` in the test plan covers both readings, so switching is a one-line
  change plus a test flip.

### 1.6.3 Neutral creep stats

Straight out of `npc_units.txt`, client 6874. Every one of these is a real
unit the four camp categories draw from.

**Hull radius is 24 for every neutral.** None of them sets `BoundsHullName`,
and the template they inherit from, `npc_dota_units_base`, sets
`DOTA_HULL_SIZE_HERO`. Neutrals are therefore hero-sized obstacles, which is
what makes camp blocking and jungle pathing behave the way they do.

Vision is 800 day and night unless noted; the exceptions are kobold,
gnoll_assassin (400), harpy_scout (1200), harpy_storm (1800) and the ranged
ancients (1400 day).

| Unit | Health | Armour | MR | Damage | BAT | Point | Acq | Range | Projectile | Speed | Gold | XP |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| kobold | 240 | 0 | 0 % | 15-16 | 2 | 0.38 | 500 | 100 | - | 290 | 3-5 | 14 |
| kobold_tunneler | 325 | 1 | 0 % | 22-23 | 2 | 0.38 | 500 | 100 | - | 270 | 12-14 | 17 |
| kobold_taskmaster | 400 | 2 | 0 % | 24-26 | 2 | 0.38 | 500 | 110 | - | 330 | 19-21 | 30 |
| forest_troll_berserker | 500 | 1 | 0 % | 28-37 | 2 | 0.3 | 300 | 500 | 1200 | 270 | 18-20 | 28 |
| forest_troll_high_priest | 450 | 0 | 0 % | 28-34 | 2 | 0.3 | 300 | 600 | 900 | 290 | 18-20 | 28 |
| gnoll_assassin | 400 | 1 | 0 % | 25-27 | 2 | 0.4 | 800 | 500 | 1500 | 270 | 16-18 | 30 |
| fel_beast | 400 | 1 | 0 % | 14-15 | 2 | 0.4 | 500 | 100 | - | 350 | 16-18 | 26 |
| ghost | 500 | 2 | 0 % | 38-43 | 2 | 0.3 | 300 | 400 | 900 | 320 | 23-25 | 42 |
| harpy_scout | 400 | 1 | 0 % | 28-34 | 2 | 0.3 | 300 | 300 | 1200 | 280 | 14-16 | 26 |
| harpy_storm | 500 | 2 | 0 % | 30-36 | 2 | 0.3 | 300 | 450 | 1200 | 310 | 25-27 | 42 |
| centaur_outrunner | 350 | 1 | 0 % | 18-20 | 2 | 0.3 | 500 | 100 | - | 320 | 16-18 | 32 |
| centaur_khan | 1100 | 4 | 0 % | 49-55 | 2 | 0.3 | 500 | 100 | - | 320 | 54-60 | 90 |
| giant_wolf | 500 | 1 | 0 % | 15-17 | 2 | 0.33 | 500 | 90 | - | 350 | 18-22 | 40 |
| alpha_wolf | 600 | 3 | 0 % | 27-29 | 2 | 0.33 | 500 | 90 | - | 350 | 32-34 | 60 |
| satyr_trickster | 300 | 0 | 0 % | 10-12 | 2.0 | 0.3 | 280 | 280 | 1500 | 300 | 12-14 | 24 |
| satyr_soulstealer | 600 | 2 | 0 % | 21-23 | 2 | 0.3 | 300 | 100 | - | 270 | 18-22 | 46 |
| satyr_hellcaller | 1100 | 2 | 0 % | 49-55 | 2 | 0.3 | 300 | 100 | - | 290 | 60-66 | 90 |
| ogre_mauler | 800 | 1 | 0 % | 22-24 | 2 | 0.3 | 500 | 100 | - | 270 | 22-26 | 32 |
| ogre_magi | 600 | 0 | 0 % | 18-20 | 2 | 0.3 | 500 | 100 | - | 270 | 28-32 | 48 |
| mud_golem | 750 | 0 | 30 % | 24-26 | 2 | 0.3 | 500 | 100 | - | 310 | 19-21 | 32 |
| mud_golem_split | 250 | 0 | 33 % | 10-14 | 2 | 0.3 | 500 | 100 | - | 310 | 6-10 | 18 |
| polar_furbolg_champion | 700 | 3 | 0 % | 39-44 | 2 | 0.3 | 500 | 100 | - | 320 | 30-38 | 66 |
| polar_furbolg_ursa_warrior | 950 | 4 | 0 % | 49-55 | 2 | 0.3 | 500 | 100 | - | 320 | 62-66 | 90 |
| wildkin | 350 | 2 | 0 % | 18-20 | 2 | 0.3 | 500 | 128 | - | 300 | 16-18 | 26 |
| enraged_wildkin | 950 | 4 | 0 % | 50-56 | 2 | 0.3 | 500 | 128 | - | 320 | 58-64 | 90 |
| dark_troll | 500 | 0 | 0 % | 24-27 | 2 | 0.3 | 250 | 250 | 1200 | 270 | 17-19 | 42 |
| dark_troll_warlord | 1100 | 4 | 0 % | 40-45 | 2 | 0.3 | 250 | 250 | 1200 | 300 | 40-46 | 90 |
| warpine_raider | 850 | 6 | 30 % | 39-41 | 2 | 0.3 | 500 | 100 | - | 310 | 48-50 | 76 |
| black_drake | 950 | 2 | 25 % | 20-22 | 2 | 0.5 | 300 | 300 | 900 | 350 | 37-43 | 95 |
| black_dragon | 2000 | 4 | 30 % | 62-68 | 2 | 0.5 | 300 | 300 | 1500 | 300 | 76-80 | 124 |
| rock_golem | 800 | 4 | 30 % | 22-24 | 2 | 0.3 | 500 | 100 | - | 270 | 37-43 | 95 |
| granite_golem | 1500 | 8 | 30 % | 80-84 | 2 | 0.3 | 500 | 128 | - | 270 | 76-80 | 124 |
| small_thunder_lizard | 800 | 3 | 50 % | 32-34 | 1.8 | 0.5 | 800 | 300 | 1500 | 270 | 42-49 | 95 |
| big_thunder_lizard | 1700 | 3 | 30 % | 60-65 | 2 | 0.3 | 300 | 300 | 1500 | 270 | 76-80 | 124 |
| frostbitten_golem | 900 | 7 | 30 % | 29-31 | 2 | 0.3 | 500 | 100 | - | 300 | 37-43 | 95 |
| ice_shaman | 1500 | 3 | 30 % | 58-62 | 2 | 0.7 | 500 | 500 | 1500 | 290 | 76-80 | 124 |

Ancient camp creeps additionally carry the `IsAncient` flag, which in Dota
blocks conversion and several spells. bota has no such spells, so the flag is
carried as data and read by nothing yet.

### 1.6.4 Camp rosters

Reconstructed from the wiki's per-camp totals and checked against the stats
above: for every camp but one, the roster's health sums exactly to the
published total, which is a strong check that these are right.

| Category | Camp | Roster | Health check |
|---|---|---|---|
| Small | Kobold | 3x kobold, 1x kobold_tunneler, 1x kobold_taskmaster | 1445 = 1445 |
| Small | Hill Troll | 2x forest_troll_berserker, 1x forest_troll_high_priest | 1450 = 1450 |
| Small | Hill Troll and Kobold | 2x forest_troll_berserker, 1x kobold_taskmaster | 1400 = 1400 |
| Small | Vhoul Assassin | 3x gnoll_assassin | 1200 = 1200 |
| Small | Ghost | 2x fel_beast, 1x ghost | 1300 = 1300 |
| Small | Harpy | 2x harpy_scout, 1x harpy_storm | 1300 = 1300 |
| Medium | Centaur | 1x centaur_outrunner, 1x centaur_khan | 1450 = 1450 |
| Medium | Wolf | 2x giant_wolf, 1x alpha_wolf | 1600 = 1600 |
| Medium | Satyr | 2x satyr_trickster, 2x satyr_soulstealer | 1800 = 1800 |
| Medium | Ogre | 2x ogre_mauler, 1x ogre_magi | 2200 = 2200 |
| Medium | Golem | 2x mud_golem, each splitting into 2x mud_golem_split | 2500 = 2500 |
| Large | Large Centaur | 2x centaur_outrunner, 1x centaur_khan | 1800 = 1800 |
| Large | Large Satyr | 1x satyr_trickster, 1x satyr_soulstealer, 1x satyr_hellcaller | 2000 = 2000 |
| Large | Hellbear | 1x polar_furbolg_champion, 1x polar_furbolg_ursa_warrior | 1650 = 1650 |
| Large | Wildwing | 2x wildkin, 1x enraged_wildkin | 1650 = 1650 |
| Large | Troll | 2x dark_troll, 1x dark_troll_warlord | 2100 = 2100 |
| Large | Warpine | 2x warpine_raider | 1700 = 1700 |
| Ancient | Dragon | 2x black_drake, 1x black_dragon | 3900 = 3900 |
| Ancient | Large Golem | 2x rock_golem, 1x granite_golem | 3100 x 1.15 aura = 3565 |
| Ancient | Thunderhide | 2x small_thunder_lizard, 1x big_thunder_lizard | 3300 vs 3400 published |
| Ancient | Frostbitten | 2x frostbitten_golem, 1x ice_shaman | 3300 = 3300 |

The Thunderhide camp is the one that does not reconcile: 800 + 800 + 1700 is
3300, the wiki says 3400. Either the wiki lags a stat change or the roster is
not two small and one big. I will settle it against the shipped data during
implementation rather than ship a guessed roster.

Spawn chance per category on a camp of that category, first spawn then every
following spawn, from the wiki totals: small 17 % / 20 %, medium 20 % / 25 %,
large 20 % / 25 %, ancient 25 % / 33 %. The "never the same roster twice in a
row" rule is what turns the first figure into the second.

### 1.6.5 Flooded camps

7.38 "Wandering Waters" added flooded camps: any camp sitting in a stream is
populated by amphibians instead of its normal roster, and every 5 minutes one
creep in the camp is permanently promoted a tier. The tiers, all present in
current shipped data:

| Tier | Melee | Ranged | Health | Damage | Gold | XP |
|---|---|---|---|---|---|---|
| 1 | tadpole | - | 400 | 19-21 | 17-19 | 30 |
| 2 | froglet | froglet_mage | 700 | 22-24 / 24-27 | 25-29 | 42 |
| 3 | grown_frog | grown_frog_mage | 900 | 41-46 / 40-45 | 37-41 | 55 |
| 4 | ancient_frog | ancient_frog_mage | 1250 | 60-64 / 58-62 | 53-56 | 104 |

This needs a river that knows which camps it covers, and bota has no river
state at all. Out of scope; see section 7.

### 1.7 Hull radii

| Hull | Radius | Used by |
|---|---|---|
| `SMALL` | 8 | ranged creep |
| `REGULAR` | 16 | melee creep, flagbearer, most neutrals |
| `SIEGE` | 16 | siege creep |
| `HERO` | 24 | heroes, every neutral |
| `HUGE` | 80 | nothing on the Dota map |
| `BUILDING` | 81.28 | ancient, fountain |
| `BARRACKS` | 144 | barracks |
| `TOWER` | 144 | towers |

---

## 2. What the current implementation does instead

| # | Was | Now | |
|---|---|---|---|
| 1 | one acquisition range for every creep (500) | 500 / 600 / 800 by type | done |
| 2 | `pick_target` ranks by distance only | class tiers, then the tie band, then hero behaviour | done |
| 3 | siege creeps have no building preference | buildings first | done |
| 4 | aggro window is a 70-tick timer per creep; cooldown 90 ticks | the click switches outright and holds 3 s; 3 s cooldown | done |
| 5 | no pre-5:00 aggro restriction | present | done |
| 6 | leash is "800 from the lane centreline, return to the nearest point" | 2.3 s out-of-range chase, return to the departure point | done |
| 7 | neutrals aggro at 400, leash at 700 by distance, heal to full on return | 240 proximity / 1800 damage; guard distance 400 plus a 5 s timer; no free heal | done |
| 8 | `Team::Neutral` is hostile to lane creeps everywhere | only the pull camps | done |
| 9 | all creeps radius 8; heroes 27 | 16 / 8 / 16 by type, heroes 24, neutrals 24 | done |
| 9a | towers 40, ancient 72, fountain 60 | 144, 81, 81 | not done, see section 7 |
| 10 | siege attack interval a bare constant | derived from the real BAT | done |
| 11 | 3 melee + 1 ranged, siege every 5th wave, no growth, no upgrades | sections 1.2 and 1.3 | done |
| 12 | no flagbearer | flagbearer from wave 5 | done |
| 13 | neutrals: one generic unit, 2 per camp, no camp identity | real per-type stats, real rosters, four camp categories | done |
| 14 | walker-vs-walker contact is a full stop for 12 ticks, then a sidestep | continuous slide; no stop timer | done |
| 15 | `steer_target` ignores moving units entirely | every unit is an obstacle | done |
| 16 | lane path is a straight tower-to-tower polyline | a found path laid between the landmarks | done |

Rows 14 and 15 were why creep blocking barely worked; both are closed. Row 9a
is not, and deliberately: see section 7.

---

## 3. Decisions taken

| Question | Decision |
|---|---|
| Neutral camp fidelity | **Real per-type data.** `npc_units.txt` stats for every neutral type (§1.6.3) and the real camp rosters (§1.6.4), so full accuracy is reachable later by adding abilities only. Abilities themselves are a separate system and stay inert for now. |
| Lane creep aggro on neutrals | **Camp flag, as Valve has it.** `pullable: bool` per camp, read only by the hostility function in `acquire.rs` (§1.6.2). |
| Lane routes | **A\* over the passability grid** from the spawner to the enemy Ancient, computed once at world build. No hand-placed waypoints. |
| Barracks, super and mega creeps | **Out.** They do not change creep logic. The wave builder carries a `CreepRank` so they drop in later without restructuring. |
| Combat rework | **Out. `combat.rs` is not touched.** Everything achievable through per-unit data in `units.rs` is done; what needs `combat.rs` is listed in §7. |

Two things I decide myself unless you say otherwise:

- **Camp typing.** The 28 camp positions in `rules.rs` carry no type. I build
  the small/medium/large/ancient tag and the `pullable` flag as a table in
  `camp.rs`, derived from the map geometry and cross-checked against the
  published camp map, and show you the table before wiring it in. `pullable` is
  read in exactly one place, so flipping every camp to `true` collapses the
  behaviour onto the pure-distance model — see §1.6.2.
- **Day and night.** Not added. Sleeping neutrals therefore do not exist —
  §7.

---

## 4. Proposed state

New module tree, replacing the creep logic currently spread across `step.rs`,
`econ.rs`, `steer.rs`, `movement.rs` and `rules.rs`. Names stay unique across
the crate because `sim/mod.rs` re-exports with globs.

```
sim/creep/
  mod.rs        mod + use only                                    in tree
  acquire.rs    the shared target-priority function               in tree
  camp.rs       CampKind, CampDef, the 28-camp table              in tree
  lane_ai.rs    LaneCreepAi: route, chase, return                 in tree
  wave.rs       schedule, composition, upgrades, spawn ranks      in tree
  neutral.rs    36 neutral kinds and the 21 camp rosters           in tree
  neutral_ai.rs NeutralAi: guard distance, window, re-aggro block  in tree
```

`Unit` lost `provoked_ticks`, `aggro_cooldown`, `shunned` and `lane_step`, and
gained one field:

```rust
/// Per-kind autonomous behaviour. Absent for heroes and buildings.
pub ai: Option<CreepAi>,
```

It kept `lane`, because towers name one too; `order_cooldown`, because towers
answer attack orders too; and `camp` and `returning`, which belong to neutrals
until step 7 moves them into `NeutralAi`.

```rust
pub enum CreepAi {
    Lane(LaneCreepAi),
    // Neutral(NeutralAi) arrives with step 7.
}

pub struct LaneCreepAi {
    /// How many waypoints of its route are behind it.
    pub step: u16,
    /// Where it left the route. Absent while it is on the route.
    pub anchor: Option<Vec2>,
    /// Ticks left of the chase after the target left acquisition range.
    /// Zero once the chase is spent.
    pub chase_left: u32,
    /// Ticks left in which the ordinary ranking may not take the creep off
    /// the target an attack order handed it. Zero when the ranking rules.
    pub provoked: u32,
    /// Where a target was last seen before the fog took it. Set the moment a
    /// target is acquired and refreshed while it stays in sight.
    pub last_seen: Option<Vec2>,
}

pub struct NeutralAi {
    /// Index into the camp table.
    pub camp: u8,
    /// The exact spot this creep spawned on, and returns to.
    pub home: Vec2,
    /// Ticks left before aggro is lost while beyond the guard distance.
    /// Zero while inside it.
    pub leash_left: u32,
    /// Ticks during which damage cannot re-aggro after a leash break.
    pub reaggro_block: u32,
    /// Length of the aggro window granted on the next aggro, in ticks.
    pub next_window: u32,
    /// Where it is fleeing to, and for how long.
    pub flee: Option<(Vec2, u32)>,
    /// Walking home, deaf to proximity but not to damage.
    pub going_home: bool,
}
```

`World` gains:

```rust
/// Waves spawned so far. Drives composition and the upgrade count.
pub wave_count: u32,
/// Roster last spawned at each camp, so a camp never repeats one.
pub camp_last: Vec<u8>,
/// The A* lane routes, built once at world creation.
pub lane_routes: [[Vec<Vec2>; 3]; 2],
```

---

## 5. Proposed algorithms

### 5.1 Target acquisition — `acquire.rs`

One function serves lane creeps, neutrals, towers and attack-moving heroes:

```rust
pub fn acquire(world: &World, id: EntityId, range: Fixed, order: PriorityOrder)
    -> Option<EntityId>
```

`PriorityOrder` is `Normal` (heroes and units, siege, buildings, wards) or
`SiegeFirst` (buildings, siege, everything, wards). Candidates are the
attackable hostiles inside `range`, sorted by

```
(class_tier, tie band from the nearest, hero_behaviour_rank, distance, entity_id)
```

- Runs when §1.4 says the creep may look again, never on its own schedule.
  A held target is not re-ranked while the creep can still hit it.
- The closest candidate of the best class sets the mark; everything within
  `AGGRO_TIE_RANGE` of that mark counts as equally close — **[approximation]**,
  `AGGRO_TIE_RANGE = 100`. A band from the nearest, not a grid of buckets:
  quantising absolute distance would split two candidates 60 apart into
  different buckets depending on where they happen to stand.
- `hero_behaviour_rank`: 0 attacking a **hero** of my side, 2 attacking its own
  allies, 1 otherwise. Non-heroes are always 1, and so is a hero last hitting a
  creep — that is what keeps last hits from drawing aggro even when the creep
  re-acquires for some other reason.
- distance, then `entity_id`, so the choice is platform-stable.

Hostility becomes a function, not `team !=`: neutrals are hostile to both
sides, lane creeps are hostile to neutrals **only** from a pull camp, towers
are never hostile to neutrals.

### 5.2 Lane creep tick

```
if disarmed             -> stand, clear everything
tick order_cooldown and the provoke hold down
look_again = target lost,
             or something of a better class is in attack range,
             or no pull holds the target and it has left attack range
pick = look_again ? acquire(acquisition_range, order) : None
if a pull is holding and the held target still lives -> keep it
else if pick is Some                                 -> engage = pick
else if engaged:
    target gone      -> drop
    target unseen    -> drop, walk to last_seen
    otherwise        -> chase_left -= 1; drop at zero
if not engaged:
    last_seen set                 -> walk there, forget it on arrival
    else off the lane with anchor -> walk to the anchor, clear it on arrival
    else                          -> attack-move the next route waypoint
```

`acquire` runs only when `look_again` says so. Running it every tick instead
looks reasonable and is wrong: it hands the creep to whichever enemy stands
nearest, so a hero merely walking past pulls a wave off the creeps it is
fighting.

The anchor is written the tick a creep first takes a target and is cleared
when the creep is back within `LANE_WAYPOINT_RADIUS` of it **or** of its own
lane. The second test is what keeps a creep that only ever fought in its lane
from walking backwards to the spot the skirmish started.

`order_aggro` works over the ranking in both directions: an order at an enemy
hero sets `engage` to that hero and starts the hold; an order at an ally runs
the ranking with that hero demoted below everyone else, falling back to it when
the ranking comes up empty.

### 5.3 Neutral tick

```
if disarmed -> stand, drop aggro
if fleeing  -> walk, count down, then go home
if going_home:
    still acquire within acquisition range (aggressive on the way)
    arriving home clears going_home; health is NOT restored
if beyond the guard distance (400 from home):
    leash_left -= 1
    at zero -> drop aggro, going_home = true,
               reaggro_block = 3 s, next_window = 3 s
else:
    leash_left = next_window
aggro sources:
    proximity: a hostile unit within 240, only while not going_home
    damage or a single-target spell within 1800, only while reaggro_block == 0
on aggro -> engage = acquire(own acquisition range, Normal)
```

The free heal on return goes away — it is a bota invention and it makes
jungling free.

### 5.4 Movement

The complaints about obstacle avoidance, formation and blocking share one
cause, so the whole locomotion path is replaced.

1. **Hulls.** Real radii from §1.7. This alone changes what fits where.
2. **Static pathing.** Keep the 64-unit grid — it is Dota's own cell size —
   but inflate blockers by the *mover's* hull, so a siege creep and a ranged
   creep do not get the same route through a gap.
3. **Lane routes.** A path found between each pair of lane landmarks -- the
   spawner, the three own towers, the corner on a side lane, the three enemy
   towers, the enemy Ancient -- and stitched into one route, found once and
   shared because every match runs the same map. Landmarks are snapped to open
   ground, because a tower closes the cell it stands on.

   Pathing straight from the spawner to the enemy Ancient, as this section
   first said, is wrong: the shortest way across the map is the diagonal, so a
   side lane would cut through the jungle instead of following its road. The
   landmarks are what make a lane a lane; the pathfinder only decides how to
   get from one to the next.
4. **A creep marches; a hero walks.** The two are separate code paths:
   `creep/march.rs` for creeps and neutrals, `walk_step` in `movement.rs` for
   everything a player steers.

   A hero slides: for every unit whose hull overlaps its destination the step
   is projected onto the tangent of that hull, keeping only what the wanted
   direction had along it, with a floor of a quarter step. Square into a body
   loses the whole step, a graze loses almost nothing.

   A creep does not slide and does not plan. It aims at its next waypoint
   until the step ahead is shut, then picks one side and keeps it until the
   way is clear again. Within that side the aim swings an eighth of a turn
   off the line, then a quarter, then three eighths, and the last resort is
   straight back. Ground and bodies are tested the same way at every stage:
   a side that runs into a building is not a side.

   Turning is what all this costs: a creep does not move while it is more
   than `TURN_TOLERANCE_BRADS` off the way it wants to face, so one working
   round a body stands still for as long as the turn takes.

   A creep that has stood for `MARCH_SHOVE_TICKS` shoves: bodies count as
   passable, ground still does not, and separation parts it from whatever it
   walked into. The count falls back a tick at a time rather than clearing,
   so a creep jittering on the spot still reaches the point of shoving.
   Without it a hero can pin a creep against a tower for good, since the two
   hulls together leave no gap to work round.

   Measured on open ground over 400 ticks, as a share of the ground a free
   creep covers: a body parked once and walked past leaves it 99 per cent, a
   body walking the same way in front leaves it 98, and a body put back in the
   way every tick leaves it 14. That is the shape blocking has in Dota: one
   parked body is not a block, staying in front is. The last figure was under
   one per cent while creeps still slid along bodies; a creep that works its
   way round instead recovers about a seventh of its ground against a blocker
   that never misses a tick.

5. **Bodies are eased apart** — `separate.rs`, after movement. Two hulls that
   overlap are pushed along the line between them, each taking half the
   overlap and at most `SEPARATION_STEP` units in a tick, never into a closed
   cell. A structure does not move, so the whole correction falls on whatever
   walked into it.

   The march itself never creates an overlap: measured over 2400 ticks of a
   full match, not one pair of hulls meets. The one source is the spawner,
   which lays a wave on fixed offsets around the spawn point without asking
   whether anything stands there. A side pinned in its own base piles the
   remains of one wave on the spot the next one appears, and the two creeps
   land on the same point exactly. Without separation the pile never comes
   apart, because every step out of it is refused as a step into a hull.

6. **Turning.** The shipped `MovementTurnRate` is radians per 0.03 seconds,
   so a half -- what every lane creep and most heroes carry -- is 5795 brads
   over a tick of a thirtieth, and a half turn takes six ticks, a fifth of a
   second. That is fast enough to look instant, and it is what Dota quotes.

   A unit's facing is advanced in exactly one place, movement, at that
   unit's own rate. Attacking does not turn anything: it waits on the
   facing and swings when it is inside `TURN_TOLERANCE_BRADS`. A unit
   mid-swing or in backswing does not walk but still comes round to what it
   is hitting, and a tower, which cannot walk at all, comes round the same
   way.

7. **A flagbearer is a melee creep.** It carries the lane AI, marches the
   route and picks targets the same way; only its magic resistance and its
   exemption from upgrades differ. Anywhere the simulation names the creep
   kinds one by one, `CreepFlagbearer` belongs in the list.

8. **Formation is not modelled** — Dota has none. Ranged creeps end up behind
   melee because of the spawn offsets and equal speed, and the collision
   resolver keeps them there.

### 5.5 What creep timing gets without touching `combat.rs`

`attack_interval`, `attack_point`, `projectile_speed`, `attack_range`,
`radius`, `armor`, `magic_resist_pct` and `vision_radius` are already per-unit
fields, so all of these become real data in `units.rs`:

- melee and ranged interval `30 ticks` (BAT 1.0), siege `90 ticks` (BAT 3.0)
- attack point: melee 14 ticks (0.467 s), ranged 15 (0.5 s), siege 21 (0.7 s)
- projectile speed: ranged 900, siege 1100
- neutrals: BAT 2.0 → 60 ticks, per-type attack points

Damage stays a single number per unit; see §7.

### 5.6 Tick order

```
2.  orders            -> also fires the order-aggro/de-aggro event
3.  scheduled         -> waves (new composition), neutrals (new rosters)
4.  statuses          -> creep timers: chase_left, order_cooldown,
                         leash_left, reaggro_block, flee
5.  target choice     -> lane creeps, neutrals, towers, heroes
6.  movement          -> new locomotion
```

---

## 6. Implementation order

Each step compiles and its tests pass before the next starts.

1. **done** — `camp.rs`, the camp table straight out of `dota.vpk`. `stat.rs`
   moved to step 6, where the wave builder needs it.
2. **done** — hull radii and the per-unit timing data from section 5.5.
   Building hulls held back to step 9, see section 2 row 9a.
3. **done** — `acquire.rs` and the hostility function; `pick_target` deleted.
4. **done** — order aggro rewritten; `provoked_ticks`, `aggro_cooldown`,
   `shunned` removed from `Unit`.
5. **done** — `lane_ai.rs`: chase, anchor return, route following.
6. **done** — `wave.rs`: schedule, composition, flagbearer, upgrades. The
   stat table folded into it rather than a separate `stat.rs`; `CreepRank`
   lives there for step 9's barracks.
7. **done** — `neutral.rs` and `neutral_ai.rs`: 36 kinds straight out of
   `npc_units.txt`, 21 rosters, guard distance, the window and the re-aggro
   block. Fleeing from invisible damage is not built: bota has no invisibility.
8. **done** — lane routes found once at world build and shared, since every
   match runs the same map.
9. **done** — `steer.rs` deleted, `walk_step` in `movement.rs` resolves every
   contact by sliding. `stuck_ticks` and `moving` gone from `Unit`.
10. **done** — a second map, `map.rs`: one straight lane on open ground, for
    reading behaviour off without building the Dota grid.
11. **done** — creep movement split off into `creep/march.rs`, turn rate a
    per-unit field, and `separate.rs` easing overlapping bodies apart.
12. `DESIGN.md` updated, `CREEPS.md` deleted.

### Tests — `sim/tests/creep/`

In the tree:

- `acquire.rs` — class order; siege preference; the hero behaviour tie-break;
  distance beats behaviour past the tie band; lane creeps fight only the pull
  camps and towers never fight the jungle at all.
- `lane.rs` — the anchor is where the creep left the route; the 2.3 s
  out-of-range chase; the walk to the last sighting; the walk back and the
  return to the march.
- `lane.rs` also covers the routes: every waypoint stands on walkable ground,
  the march ends beside the enemy Ancient, and no waypoint strays more than 900
  from its lane's centreline.
- `block.rs` — a body in the way costs a creep ground; a body kept in front
  nearly stops it; a creep never stalls for long against one; the hulls the map
  is walked with.
- `wave.rs` — the wave numbering; the opening wave; the flagbearer from wave 5
  every second wave, replacing rather than adding; the siege creep from wave 11
  every tenth; the count growth at waves 31, 61, 71 and 81; the upgrade cadence
  and its cap; an upgraded wave's stats; the ranged rank behind the front one.
- `switching.rs` — a hero walking up steals nothing; a creep on a building
  drops it for an arriving unit; a hero in reach is kept over a nearer creep;
  a hero out of reach is not; the click switches outright and holds 3 s; an
  attacking hero keeps winning the tie until it stops.

Still to write, with their steps:

- `neutral.rs` — the 240 aggro radius against the longer acquisition range; the
  window running only beyond the guard distance; the leash break and what it
  sets; proximity going deaf on the way home; per-kind stats and upgrades; the
  roster counts per camp size.

Every test that asserts a target is *kept* must run longer than the longest
timer in the system, or it proves nothing: a 60-tick probe missed a bug that
dropped every target on a 69-tick clock.

- Determinism and release-mode runs as usual.

---

## 7. Known deviations from Dota 7.41e

Everything this plan will **not** reproduce, and why. Each is small and
isolated if you want it later.

1. **Attack classes are absent.** `creep_irresolute` (−25 % melee damage to
   heroes), `creep_piercing` (+50 % / −50 % / −50 % for ranged) and
   `creep_siege` (+150 % to buildings, −50 / −30 / −40 % incoming) all apply
   their multiplier at damage time, which lives in `combat.rs`. Consequence:
   creep damage against heroes and buildings is off by exactly those
   percentages.
2. **No per-swing damage roll.** `Unit::attack_damage` is one number, so the
   19–23 style ranges collapse to their midpoint. Rolling needs `combat.rs`.
3. **No attack speed.** Neutral upgrades grant +5 attack speed every 7:30;
   without an attack-speed stat that part of the upgrade is dropped. Health,
   armour, damage, gold and XP still apply.
4. **No super or mega creeps** — barracks are not modelled. `CreepRank` exists
   so they slot in later.
5. **No day and night**, so neutrals never sleep and their aggro range is
   always 240.
6. **Neutral abilities are inert.** Stats and rosters are real; auras, stuns,
   heals and the golem split are not. This is the piece that stands between
   this plan and full accuracy, and the data model is built so that adding
   them changes nothing else.
7. **Flagbearer aura is partial.** The 40 % magic resistance and the creep
   itself are real; the +3 health regen aura, the 1200-radius area gold and
   the growing magic resistance need an aura system.
8. **`AGGRO_TIE_RANGE`** is an approximation of "about equally close" — see
   §5.1. Valve has never published the real rule.
8a. **The 3 s provoke hold** is an approximation. Valve publishes the 3 s
   *cooldown* on the aggro check but no duration for how long the pulled creep
   stays pulled. Three seconds is used for both, which reproduces the observed
   loop: re-click every three seconds to keep a wave on you.
9. **Local avoidance is a model, not Valve's algorithm.** Valve has never
   published it. §5.4 is calibrated against observable behaviour: blocking
   works, creeps do not freeze on contact, waves meet where they should.
10. **No flooded camps.** §1.6.5 needs a river that knows which camps it
    covers and a 5-minute promotion clock. bota has no river state. The frog
    tiers are real data and go in the table, spawning nothing.
11. **Building hulls are still bota's**: towers 40, ancient 72, fountain 60,
    against Dota's 144 and 81. Raising them is not a pathing change alone:
    `in_attack_range` counts both hulls, so a 144 tower would also reach 104
    further, and reach is a combat question with `combat.rs` out of scope.
    Doing it properly means separating the collision hull from the range hull.
12. **One pathing grid, not one per hull.** Section 5.4 asks for static
    blockers inflated by the mover's own hull; the grid is inflated by the
    widest walker instead. A ranged creep therefore cannot squeeze through a
    gap a hero could not.
13. **Neutral vision is uniform.** Per-type day and night vision is in the
    stat table but bota has one vision number per unit and no night, so the
    day figure is used.
