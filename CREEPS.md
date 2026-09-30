# Creeps — the Dota reference

The Dota 2 **7.41e** rules (Summer Scrub, August 2026) for lane and neutral creeps
that the simulation follows: the numbers, the behaviour, and where they came from.
What of it is not modelled yet is listed at the end.

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

Attack modifiers, for the record — see *Not yet modelled* for which of these ship now:

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
- the switch **holds for 3 s** — **[approximation]**: Valve publishes the 3 s
  cooldown but no hold duration — and this hold is
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
- Night: aggro range 0. bota has no day cycle — see *Not yet modelled*.

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
- Arriving home does **not** restore health.
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

- The camp flag ships as a single `pullable: bool` per camp in
  `game/config/camp.rs`, read in one place — `pullable_camp` in
  `game/systems/target.rs`.
- Setting every camp `pullable: true` reduces the behaviour to your model
  exactly, with the guard distance doing all the work.

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
state at all. See *Not yet modelled*.

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

## Not yet modelled

1. **Attack classes.** `creep_irresolute`, `creep_piercing` and `creep_siege`
   (§1.1) are not applied: mitigation reads only the damage kind, so creep damage
   against heroes and buildings, and damage into siege creeps, is off by exactly
   those percentages.
2. **No per-swing damage roll.** A unit has one damage number, so the 19–23 style
   ranges collapse to one value.
3. **No day and night.** Neutrals never sleep, their aggro range is always 240,
   and each uses its day vision.
4. **Neutral abilities are inert.** Stats and rosters are real; auras, stuns,
   heals and the golem split are not.
5. **`AGGRO_TIE_RANGE`** is an approximation of "about equally close" (§1.5).
   Valve has never published the real rule.
6. **No flooded camps.** §1.6.5 needs a river that knows which camps it covers
   and a 5-minute promotion clock. The camp table carries a `flooded` flag that
   nothing reads.
