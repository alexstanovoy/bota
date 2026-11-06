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

The NPC script supplies level 30. Valve [6.86](https://www.dota2.com/balanceofpower/)
establishes 2-second BAT and +100 attack speed (200 total). The attack-speed bonus is an
engine intrinsic, absent from the installed text row. The existing simulator rule
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


## Bash, Slam and Roar

The importer retains the installed `roshan_bash`, `roshan_slam` and `roshan_revengeroar`
numeric rows. Bash rolls 15% pseudo-random chance at the attack point, carries its 50 physical
bonus and 1.65-second piercing stun through impact, and is disabled by Break. The conditional
bonus follows critical/base-attack scaling, is mitigated and blocked once with the attack,
and contributes to actual-damage lifesteal, excluding the base used by cleave. Ward targets
do not roll Bash. Valve [7.38](https://www.dota2.com/patches/7.38) adds its distinct attack
animation; the client selects that follow-through from the reported attack proc, correlated
with the attack point rather than another cast.

Slam casts against a crowd of at least three enemies containing a hero. It has 350 radius,
0.47-second cast point, 10-second cooldown, 70 magic damage plus 10 per elapsed minute, and
50% movement/attack slow for four seconds. Valve [7.40](https://www.dota2.com/patches/7.40)
makes creep damage double while keeping the same slow duration. Spell-immune recipients
are excluded. The first damage increment is at 1:00: this follows the installed base/growth
and the documented 10-minute 170-damage table. Secondary prose describing an increment
at the horn conflicts with that table; no first-party minute-boundary implementation was found.

Valve [7.39](https://www.dota2.com/patches/7.39) makes Roar work through stun and silence,
strongly dispel on cast, and announce globally. It triggers below 85% health against a real
hero of the previous killing team within 800. Its imported half-second windup precedes
600-radius magic damage (50 plus 10 per elapsed minute), two-second disarm, and a 12-second
25% incoming-damage aura with half-second linger. Valve 7.40 makes the disarm undispellable.
The same minute-boundary reading as Slam is used. The windup is projected through the shared
cast view; its declared duration also bounds client validation. The global announcement
carries neither coordinates nor a hidden entity handle.

## Gameplay bug-fix regressions

The accompanying announcement's Bug Fixes list is read alongside the gameplay payload;
the payload alone omits several fixes. Relevant historical fixes retain their intent after
later balance changes. Tests use current values.

| Valve fix | Regression |
| --- | --- |
| [2011-12-01](https://store.steampowered.com/oldnews/6912): “Fixed Roshan bashing wards.” | `roshan_never_procs_bash_on_a_banner_or_other_ward` protects proc eligibility; normal attacks still work. |
| [2013-01-31](https://store.steampowered.com/oldnews/9872): Slam stopped slowing/damaging nearby units. | `roshan_slams_a_crowd_with_a_hero_and_deals_double_damage_to_creeps` observes autonomous casting, slow strength, equal four-second durations and expiration. |
| [2013-03-14](https://store.steampowered.com/oldnews/10145): Slam incorrectly dealt physical damage. | The same scenario requires magical damage; `slam_does_not_damage_or_slow_spell_immune_heroes_or_couriers` covers immunity. Visage's unsupported Familiar interaction remains deferred. |
| [2011-11-17](https://store.steampowered.com/oldnews/6812): Dismember lasted too long on Roshan. | `unblocked_dismember_releases_roshan_after_his_status_resisted_duration` goes through a normal cast order, checks target and channel release, and changes resistance during the hold. |
| [Spring Cleaning 2016](https://www.dota2.com/springcleaning2016): live health upgrades waited until respawn. | The living growth test checks both current and maximum health across minute boundaries; the dead/respawn test checks current strength on return. |
| [7.33b](https://store.steampowered.com/news/app/570/view/3703690760958334829): north pit allowed wards. | `ward_items_and_dispenser_cannot_be_placed_in_installed_exclusion_volumes` uses the installed convex volumes. Watcher capture from the same patch was removed in 7.34. |
| [7.39b](https://store.steampowered.com/news/app/570/view/536603447519085191): “Fixed Roshan's Roar not playing the fx/animation” | A real-server natural-respawn scenario observes the imported animation, particles, global sound and message. A separate snapshot test accepts the declared 15-tick windup and refuses a longer one. |
| [7.41c](https://store.steampowered.com/news/app/570/view/681874975890606523): Shard's mana cost was missing from its icon. | `refresher_shard_displays_its_mana_cost_on_the_item_and_plays_its_refresh_effect` observes “200” in that slot, then consumption and the imported refresh particle. Item costs use the shared item definition. |

Enemy Purge removes beneficial effects and selects only beneficial dispellable effects as a
reason to cast. Friendly cleansing removes harmful effects. The Slam/Purge and Illusion Rune
regressions protect both directions, so introducing a removable slow cannot make enemy
Purge help its victim.

The same 7.41c announcement fixes Orb/Shard cooldown sharing and Meepo cloning Banner.
Orb and Meepo are absent; those specific interactions remain deferred. Shard copies share
their physical cooldown now. Interrupted Grab position/facing (7.38c), Banner particle
cleanup, and remaining loot/pit bug-fix cases belong to their corresponding player paths.
The separate Block of Cheese shield is not a Roshan drop and is not modeled here; its
historical shield fixes must not be mistaken for standard Cheese's restoration behavior.

## Pit migration and Grab and Throw

Valve [7.38](https://www.dota2.com/patches/7.38) replaces Twin Gate travel with walking,
adds Grab and Throw, permits five shared Slam/Grab interruptions, and counts failed attempts
while silenced. The installed `roshan_grab_and_throw` and `roshan_inherent_buffs` rows supply
cast/channel timing, flight, damage, interruption count, speed increments/cap and movement
slow resistance. Permanent tree walking and forced-movement immunity follow the current
[Roshan mechanics](https://liquipedia.net/dota2/Roshan); tree walking does not destroy trees
or grant flying vision. [Valve's Meat Hook notes](https://www.dota2.com/datafeed/herodata?language=english&hero_id=14)
explicitly include Roshan among the units the Hook passes through. It neither damages nor
drags him; the Roshan-specific regression preserves that rule from the current base.

The natural journey owns its five-stop budget and launched-throw count separately. Each
attempt adds 70 speed, capped at 300 extra; the extra 25% slow resistance combines with
Devotion to 43.75%. Arrival clears these temporary bonuses. Artificial sky changes do not
reverse a natural journey. Moving contact deals 200 magical damage, knocks a victim 150
units over 0.6 seconds, cuts trees within 100 of that path, and has an independent two-second
cooldown per victim. This clock is separate from dispellable/evictable modifiers.

Grab has a 0.25-second cast point, 0.84-second channel, contact at 0.75 seconds, and a
1,500-unit, 0.8-second flight with a one-second landing stun. Existing tick quantization
maps these to 8/25/23/24 ticks. Contact uses the movement damage; landing deals pure damage:
200 plus 100 per earlier launched throw and a growing current-health term. The current
mechanics table gives 5.5% at ten minutes and 10.5% at twenty: this implementation uses
0.5% at the horn plus 0.5% per minute. Valve's original wording does not settle that initial
increment; this is the best-evidenced table/script reading, not an independently verified
engine boundary. Status resistance shortens the landing stun, not the grip or flight.

A cancelled grip releases immediately; a launched flight survives its caster's death or
cancellation. Victim death, including Aegis absence, removes forced motion. Hook can rescue
a carried ally; fear and ordinary movement resume after the carry and landing stun. The
priority between overlapping movement controllers is an explicit implementation choice
where first-party precedence is unspecified. Separate wire fields describe forced motion
and the active channel, so an intervening passive Spell Block event cannot replace the
Grab animation. Neither field exposes a hidden source handle.

Valve [7.38c](https://store.steampowered.com/news/app/570/view/536597739399545003) fixes heroes
getting their position or model angle stuck when Grab is interrupted. The migration tests
exercise cancellation before/between contact and launch, both deaths, Aegis, last-tick
disables, simultaneous landings, Hook rescue and movement/facing recovery. Headless player
paths additionally observe the model's release, flight height and resumed facing. The
passive-block-during-channel regression protects the distinct animation state. The legacy
Twin Gate/Night Stalker fix has no current gate sequence to reproduce; artificial daytime
is tested against the current walking journey instead.

## Pit placement, sight and ward targets

Valve [7.31d](https://www.dota2.com/patches/7.31d) makes Roshan attack wards when no other
unit is in melee reach and destroy susceptible wards with one attack. The current unit
mechanics designate Observer and Sentry Wards for destruction. Their unit data carries the
response; the attack record retains it through impact. Banner keeps its ordinary non-hero
hit count, and ward attacks never roll Bash. The
[7.36b Bug Fixes](https://store.steampowered.com/news/app/570/view/4176605502420155695)
explicitly exempt Serpent Wards from instant destruction and count Roshan as a hero against
them. Those units are absent; that specific interaction remains deferred, with a distinct
hero-hit response available in the unit policy.

[Current pit limitations](https://liquipedia.net/dota2/Roshan) also forbid Branch planting
and suppress courier sight inside either pit. Pit blindness applies to ground and flying
couriers, the snapshot's sight radius, and live point/event queries. It ends immediately
outside. Frontend point and fog-grid shading apply the same pit boundary; a real
metadata/snapshot test crosses each installed sloping boundary within one terrain cell,
then moves the viewer onto the target side as a positive control. The placement predicate belongs to the use definition and is shared by completion
and preview: Ward, Dispenser and Banner consult the installed ward-exclusion volumes;
Plant Tree consults the pit volumes. Invalid completion consumes nothing. The headless
placement test observes forbidden previews for all five items, then allowed previews for
the same items on ordinary ground.

The pit attack limitation still applies while Roshan walks: the attacker must stand in
either pit even when Roshan is outside. Abilities retain their separate targeting rules.

| Valve fix | Additional regression |
| --- | --- |
| [2011-12-15](https://store.steampowered.com/oldnews/7021): buying while dead must put the purchase in stash; the note explicitly names Aegis. | `shopping_during_aegis_reincarnation_keeps_the_purchase_in_stash_after_return` buys during the absence and checks it after the return away from shops. |
| [2013-03-14](https://store.steampowered.com/oldnews/10145): “Fixed Cheese not stacking in inventory” and its missing cooldown. | `cheese_from_two_separate_ground_pickups_combines_into_one_charged_stack` uses two physical arrivals; the existing Cheese-use scenario checks its independent cooldown. |
| [2012-04-12](https://store.steampowered.com/oldnews/7733): “Fixed Roshan not disabling Blink Dagger.” | `roshans_actual_attack_and_slam_stop_drinks_and_disable_blink` uses his actual autonomous attack and Slam, requiring positive damage and a disabled Blink. |
| [7.33b](https://store.steampowered.com/news/app/570/view/3703690760958334829): Roshan kill should capture Watchers. | Superseded by [7.34](https://www.dota2.com/patches/7.34). `killing_roshan_does_not_capture_any_watchers_after_734_removed_that_reward` enables Watchers and verifies their ownership stays unchanged. |

The [7.41f cave fix](https://store.steampowered.com/news/app/570/view/677383425371407609)
and [August 2025 pit fixes](https://store.steampowered.com/news/app/570/view/536611691635409378)
give no repaired coordinates. Those exact hiding spots remain unverified. The simulator
uses the installed 7.41f terrain and compiled pit volumes, with explicit attack and vision
boundary tests; that evidence establishes the current geometry source and rules, rather
than an exact reproduction of the undocumented old spots.

## Public state, timer and sounds

Valve [7.38](https://www.dota2.com/patches/7.38) adds the alive/dead/possibly-alive display
and public respawn window. Snapshots retain the last public death and each team's evidence
of life. Seeing Roshan or hearing his global Roar confirms life until the next public death;
respawning out of sight does not. Before eight minutes the timer counts to the first possible
return, then to the eleven-minute guarantee. At that guarantee, alive is public inference.
Time follows authoritative snapshots, so a paused match does not advance this timer.

A side receives only its own evidence. Full spectators receive both teams' evidence; the
client uses their union, matching the default spectator fog. Replay perspectives select the
corresponding team's bit. The selected random respawn time and hidden unit identity are not
in this record. Fresh snapshots therefore restore the display after reconnecting. The pit
label uses the naturally scheduled home from the map data; exact in-transit tooltip behavior
was not independently verified. Third Eye's exact timer requires the unsupported Templar
Assassin and remains deferred.

The installed sound events define `Roshan.RevengeRoar` with its simultaneous `.Layer`,
`Roshan.Death`, and the default `valve_dota_001.music.roshan_end` stinger. The sound player
keeps alternatives within each layer and throttles the logical cue once. Global death plays
the roar and stinger, alongside the existing team announcer, independent of camera position.
The stinger uses ordinary sound playback: the client has no background-music setting.
The full default clip is imported; dynamic music beat transitions are not modeled.

The headless status test crosses both public deadlines while outside pit vision and verifies
that the guarantee does not fabricate a sighting. The Roar regression requires both layers
simultaneously, and the death test moves the camera across the map before the killing attack.

Valve's [December 2023 Banner fix](https://store.steampowered.com/news/app/570/view/3902996144638678168)
states that its particles sometimes survived incorrectly. The real-server expiry and enemy
attack scenarios require the model and every owned particle instance, including children,
to finish. Simulator scenarios separately protect independent Banner and Glyph aura sources.

## Physical loot and illusion inventory

Valve's [March 2012 fix](https://store.steampowered.com/oldnews/7601) prohibits courier
pickup of Aegis. The script does not encode the recipient rule, so the catalogue declares
its real-hero eligibility explicitly. The same predicate is used by checked orders, direct
simulation errands and receipt, including handover targets. Rejection starts no pickup
lifetime and changes neither the ground stack nor its ownership. Ordinary courier loot
pickup remains available.

[Illusion inventory rules](https://liquipedia.net/dota2/Illusions) keep copied equipment
nonphysical. Illusions cannot pick up, drop, hand over, sell, or move those copies into the
real stash. Incoming item/charge handovers are refused too. Seat-owned stash operations
remain separate from the illusion's bag. Regressions cover validated commands and direct
simulation calls, with controls using the original hero's item and courier-carried Cheese.
Automatic sales also reject copies, including an inherited sale mark on the copy's final
lifetime tick. The guard uses durable copy identity as well as the active modifier, whose
expiry precedes automatic sales. The live-client regression accepts properly marked own
rune copies, refuses their attempted Banner drop, and then drops the original hero's item.
