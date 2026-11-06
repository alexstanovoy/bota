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

## Banner

Valve [7.36](https://www.dota2.com/patches/7.36) replaces the old health/damage bonuses with
invulnerability for allied lane creeps while within the aura, with 0.1-second linger and a
120-second banner lifetime. The old bonus fields remaining in the script are unused.
Valve [7.40](https://www.dota2.com/patches/7.40) adds two upgrades on successive drops. The
installed `item_roshans_banner` row supplies the three radii (600/900/1200), health
(24/32/40), melee hero hit counts (6/8/10), cast range (1200), duration and level cap.
[Banner unit mechanics](https://liquipedia.net/dota2/Roshan%27s_Banner_%28unit%29) give ranged
hero and non-hero hit counts, hull sizes, and spell immunity. The installed NPC row supplies
vision, bounty, armor and ancient classification. A physical item retains its drop level.

The aura uses `modifier_fountain_glyph`, independently for each banner and for the team
Glyph, so neither lifetime erases the other. Only allied lane creeps receive it.
`tools/import_items` imports the map's `trigger_no_wards` volumes from each model's convex
physics hulls, preserving their union. They exclude both pits, fountains and their ramps,
and one additional map brush. Ward and Banner placement checks the volumes again when
the use completes; a failed placement spends nothing.

## Strength and growth

Valve [7.39](https://www.dota2.com/patches/7.39) establishes 7 attack damage and 0.4 armor per
minute. [Strength of the Immortal](https://liquipedia.net/dota2/Roshan) supplies the separate
first increments: armor at the horn, health (+130), damage and experience (+20) from 1:00.
The count is derived from the authoritative game clock on updates and respawns. The armor
is a separate aura: radius 5 from the installed `roshan_devotion` row, 0.5-second linger,
allied living units only, excluding invulnerable units. Only armor spreads.

The NPC script supplies level 30; current Roshan mechanics document 200 attack speed with
2-second BAT. That attack speed is an engine intrinsic, absent from the installed text row;
it has not been independently confirmed from engine source. The existing simulator rule
that maximum-health changes preserve the filled fraction is retained; no first-party
Roshan-specific exception was found. The shared Q16.16 health representation saturates near
32,768 HP (around 206 minutes of this growth); no extra growth cap is added.

## Defensive passives

The installed `roshan_spell_block` supplies its 15-second cooldown and 25% status resistance;
`roshan_devotion` supplies 25% slow resistance. Neither passive has `IsBreakable` set.
[Slow resistance](https://liquipedia.net/dota2/Slow_Resistance) stacks multiplicatively,
reduces negative movement-speed changes, and increases minimum speed since 7.38. It is
separate from status resistance, which shortens eligible disables. Resistance is gathered
before movement changes, with one basis-point rounding independent of modifier order.

[Strength of the Immortal](https://liquipedia.net/dota2/Roshan) documents permanent phased
movement and 80% reduction of illusion attack damage, excluding spell damage. The attack's
illusion classification is captured in the attack record and survives projectile flight.
The same mechanics document hero-class attacks against couriers. Neutral targeting policy
allows Roshan to acquire couriers while leaving ordinary autonomous neutrals' policy intact.

Spell absorption occurs at the cast point or at a tracking projectile's impact, according to
ability data. A valid blocked cast spends mana and cooldown but never starts its channel;
an invalid target (including lost pit sight) consumes neither. Projectile allegiance belongs
to the projectile even after its caster dies. One ready block absorbs one simultaneous
eligible impact, including zero-damage stun/root projectiles.

Valve [7.40](https://www.dota2.com/patches/7.40) makes lifesteal treat Roshan as a creep. The
shared target factors are 60% for attack lifesteal and 20% for spell lifesteal, from the
[current mechanics](https://liquipedia.net/dota2/Lifesteal). The attack consumer uses health
actually removed, after mitigation, and excludes ward-type targets including Banner.
