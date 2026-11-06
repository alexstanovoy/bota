# Roshan: Dota 2 7.41f sources

The installed `scripts/npc/npc_units.txt` supplies the unit's numeric row. `tools/import_items`
imports it with the item catalogue and checks it against the install. The numbers below that
are not in that row have explicit sources.

| Rule | Evidence |
| --- | --- |
| Two river pits, one boss; initial spawn in the top pit | Valve [7.38](https://www.dota2.com/patches/7.38) and [7.41](https://www.dota2.com/patches/7.41) |
| Present during pregame; no blocked camp spawn | The unit spawner, and [Roshan](https://liquipedia.net/dota2/Roshan), Respawning |
| From 15:00, walks between pits on natural day/night changes; top by day, bottom by night | Valve 7.38 and 7.41 |
| Respawns 8–11 minutes after death | Valve 7.38 |
| 6,000 health, 20 regeneration, 75 damage, 30 armor, 55% magic resistance, 270 speed, 1,400 vision | Installed `npc_dota_roshan` row |
| Approach aggro at 140, damage/spell aggro at 1,800; ordinary neutral leash behavior, awake at night | [Roshan](https://liquipedia.net/dota2/Roshan), Behavior; [Neutral creeps](https://liquipedia.net/dota2/Neutral_creeps), Behavior |
| Attacks reach him only from a pit | Installed map's `trigger_boss_attackable` brush volumes; Roshan, Roshan Pit |
| Flying sight obeys pit boundaries | Valve [7.41c](https://www.dota2.com/patches/7.41c) |

Pit coordinates and attack volumes are the 7.41f map's `default_ents.vents` and brush hulls,
shifted by 9,216 like all map coordinates. Their origins and hull names are beside the rows.
The demo map has no pit; the full map and the full-geometry Solo Mid map have both. Solo Mid
keeps objectives disabled unless the host enables them.

The respawn distribution is not specified by Valve. The simulator draws uniformly among
whole seconds in the inclusive 480–660 second window, independently for each death.

## Loot and reincarnation

The installed item scripts supply Aegis' 5-second delay and 300-second pickup lifetime,
Cheese's restoration, Shard's 200 mana cost and 180-second cooldown, and carry restrictions.
Valve [7.41](https://www.dota2.com/patches/7.41) and the script's `refresh_items=0` make Shard
refresh abilities only. Its stack cooldown pauses outside active inventory; held copies share
one use's cooldown, while a future fresh drop does not inherit a destroyed item's clock.

Valve [7.37](https://www.dota2.com/patches/7.37) establishes Banner from death two. The current
[Roshan drop table](https://liquipedia.net/dota2/Roshan) establishes Cheese and Shard together
from death three, independent of day/night; Valve 7.38 removes Blessing. The earlier 7.37
night-only Blessing text is superseded by that removal and the current table.

[Aegis, Reincarnation](https://liquipedia.net/dota2/Aegis_of_the_Immortal) supplies the details
not exposed in the item script: return at the death position, preserved cooldowns, full pools,
no ordinary death rewards, 1800/800 ground vision during the delay, denial before pickup,
reclaim on Roshan's return, and expiry/reclaim recovery of both maxima over five seconds.
Other players' positive damage or Roshan interrupts that recovery; self damage does not.
The recovery values and ghost vision are documented mechanics rather than script fields.

Aegis' timer is an appended `ItemView` field. New wire builds negotiate the registry/rules
fingerprint. Every pre-existing 62-item catalogue prefix digest remains unchanged; a test
pins the digest recorded from the preceding implementation.
