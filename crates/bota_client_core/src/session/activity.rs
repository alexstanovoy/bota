//! Current action observations and clocks, independent of a renderer or script API.

use super::tracks::EntityTrack;
use bota_proto::{AbilityId, AbilitySlot, EntityId, Fixed, StatusFlags, UnitView};

/// A received move larger than this is a position discontinuity, not a run segment.
/// Front ends use the same cutoff when snapping between snapshots.
pub const INSTANT_MOVE_DISTANCE: u32 = 450;

/// Which observable activity a body is performing. These are not engine activity ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    IdleRare,
    Run,
    Attack {
        alternate: bool,
    },
    Cast {
        ability: Option<AbilityId>,
        slot: Option<AbilitySlot>,
    },
    Channel {
        ability: Option<AbilityId>,
        slot: Option<AbilitySlot>,
    },
    Flail,
    Stunned,
    Die,
    Spawn,
    Teleport,
}

/// The stage a renderer maps onto its clip's impact point or loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityPhase {
    Loop,
    Windup,
    Recovery,
    Channel,
    Transition,
}

/// Locally known loop durations in milliseconds. None is unknown, Some(0) is a static pose.
/// Clients provide these from their model catalogue, without rendering types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActivityProfile {
    pub idle_ms: Option<u32>,
    pub rare_idle_ms: Option<u32>,
    pub run_ms: Option<u32>,
    pub channel_ms: [Option<u32>; 6],
    pub flail_ms: Option<u32>,
    pub stunned_ms: Option<u32>,
    pub die_ms: Option<u32>,
    pub spawn_ms: Option<u32>,
    pub alternate_attacks: bool,
}

/// A sample at the latest received observation. Cycle, when known, is Q16.16 in [0,1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurrentActivity {
    pub activity: Activity,
    pub phase: ActivityPhase,
    pub cycle: Option<Fixed>,
    pub observed_at: u32,
    pub started_at: u32,
    pub duration_ticks: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObservedPhase {
    phase: bota_proto::UnitActionPhase,
    elapsed: u32,
}

/// Bounded per-body animation memory. Only visible observations advance it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActivityClock {
    tick: u32,
    rate: u16,
    id: Option<EntityId>,
    phase: Option<ObservedPhase>,
    forced: bool,
    stunned: bool,
    forced_since: u32,
    stunned_since: u32,
    ability: Option<AbilityId>,
    slot: Option<AbilitySlot>,
    point: u32,
    backswing: u32,
    attack_ordinal: u32,
    idle_since: u32,
    run_since: Option<u32>,
    run_distance: u64,
}

impl ActivityClock {
    pub(super) fn observe(&mut self, track: &EntityTrack, map: &bota_core::MapDef, rate: u16) {
        use bota_proto::UnitActionPhase as Kind;
        let unit = &track.unit;
        let tick = track.last_seen_tick;
        let phase = Some(
            unit.action_phase
                .map(|phase| ObservedPhase {
                    phase: phase.phase,
                    elapsed: phase.elapsed / bota_core::BEATS_PER_TICK,
                })
                .unwrap_or(ObservedPhase {
                    phase: Kind::IdleMove,
                    elapsed: 0,
                }),
        );
        let forced = unit.forced.is_some();
        let stunned = unit.statuses.bits & StatusFlags::STUNNED != 0;
        if forced && !self.forced {
            self.forced_since = tick;
        }
        if stunned && !self.stunned {
            self.stunned_since = tick;
        }
        let current = phase.map(|phase| phase.phase);
        let previous = self.phase.map(|phase| phase.phase);
        let attack = matches!(current, Some(Kind::AttackWindup | Kind::AttackBackswing));
        let was_attack = matches!(previous, Some(Kind::AttackWindup | Kind::AttackBackswing));
        let restarted = matches!((phase,self.phase),(Some(now),Some(old)) if now.phase==Kind::AttackWindup && old.phase==Kind::AttackWindup && now.elapsed<old.elapsed);
        if attack
            && (!was_attack
                || current == Some(Kind::AttackWindup) && previous == Some(Kind::AttackBackswing)
                || restarted)
        {
            self.attack_ordinal = self.attack_ordinal.wrapping_add(1);
        }
        if attack {
            let definition = crate::unit_definition(map, unit);
            self.point = definition.map_or_else(
                || ms_ticks(unit.attack_point, rate),
                |def| ms_ticks(def.attack_point, rate),
            );
            self.backswing = definition.map_or(0, |def| ms_ticks(def.attack_backswing, rate));

            self.ability = None;
            self.slot = None;
        } else if matches!(
            current,
            Some(Kind::CastPoint | Kind::CastBackswing | Kind::Channel)
        ) {
            // Recovery carries no current ability observation. Never reuse an
            // earlier spell for a new item recovery that arrives without an idle frame.
            let ability = unit
                .cast
                .and_then(|cast| cast.ability)
                .or_else(|| unit.channel.map(|channel| channel.ability));
            {
                self.point = ability
                    .and_then(bota_core::ability_def)
                    .map_or(0, |def| ms_ticks(def.point, rate));
                self.backswing = ability
                    .and_then(bota_core::ability_def)
                    .map_or(0, |def| ms_ticks(def.backswing, rate));
            }
            self.ability = ability;
            self.slot = ability.and_then(|ability| ability_slot(unit, ability));
        } else {
            self.ability = None;
            self.slot = None;
        }
        let busy = forced || stunned || current.is_some_and(|phase| phase != Kind::IdleMove);
        let was_busy =
            self.forced || self.stunned || previous.is_some_and(|phase| phase != Kind::IdleMove);
        let moving = !busy && is_running(track, rate) && !bota_core::kind_def(unit.kind).fixed;
        if self.id.is_none() || self.run_since.is_some() != moving || busy || was_busy {
            self.idle_since = tick;
            self.run_since = moving.then_some(tick);
            self.run_distance = 0;
        } else if moving
            && tick > self.tick
            && let Some(velocity) = track.velocity
        {
            self.run_distance = self
                .run_distance
                .saturating_add(distance_raw(velocity.delta));
        }
        self.tick = tick;
        self.rate = rate.max(1);
        self.id = Some(unit.id);
        self.phase = phase;
        self.forced = forced;
        self.stunned = stunned;
    }

    /// Samples received action progress, using a profile only for local loop timing.
    pub fn sample(&self, profile: ActivityProfile) -> Option<CurrentActivity> {
        use bota_proto::UnitActionPhase as Kind;
        let phase = self.phase?;
        if self.forced {
            return Some(self.loop_sample(Activity::Flail, profile.flail_ms, self.forced_since));
        }
        if self.stunned {
            return Some(self.loop_sample(
                Activity::Stunned,
                profile.stunned_ms,
                self.stunned_since,
            ));
        }
        let start = self.tick.saturating_sub(phase.elapsed);
        let alternate = profile.alternate_attacks && self.attack_ordinal.is_multiple_of(2);
        let sample = match phase.phase {
            Kind::AttackWindup | Kind::AttackBackswing => {
                let recovery = phase.phase == Kind::AttackBackswing;
                let elapsed = phase
                    .elapsed
                    .saturating_add(if recovery { self.point } else { 0 });
                self.timed(
                    Activity::Attack { alternate },
                    if recovery {
                        ActivityPhase::Recovery
                    } else {
                        ActivityPhase::Windup
                    },
                    start.saturating_sub(if recovery { self.point } else { 0 }),
                    self.point.saturating_add(self.backswing).max(1),
                    elapsed,
                    false,
                )
            }
            Kind::CastPoint | Kind::CastBackswing => {
                let recovery = phase.phase == Kind::CastBackswing;
                if self.ability.is_none() {
                    return Some(CurrentActivity {
                        activity: Activity::Cast {
                            ability: None,
                            slot: None,
                        },
                        phase: if recovery {
                            ActivityPhase::Recovery
                        } else {
                            ActivityPhase::Windup
                        },
                        cycle: None,
                        observed_at: self.tick,
                        started_at: start,
                        duration_ticks: None,
                    });
                }
                let elapsed = phase
                    .elapsed
                    .saturating_add(if recovery { self.point } else { 0 });
                self.timed(
                    Activity::Cast {
                        ability: self.ability,
                        slot: self.slot,
                    },
                    if recovery {
                        ActivityPhase::Recovery
                    } else {
                        ActivityPhase::Windup
                    },
                    start.saturating_sub(if recovery { self.point } else { 0 }),
                    self.point.saturating_add(self.backswing).max(1),
                    elapsed,
                    false,
                )
            }
            Kind::Channel => {
                let activity = Activity::Channel {
                    ability: self.ability,
                    slot: self.slot,
                };
                let clip = profile
                    .channel_ms
                    .get(usize::from(self.slot.map_or(0, |slot| slot.0)))
                    .copied()
                    .flatten();
                if let Some(ms) = clip {
                    self.timed(
                        activity,
                        ActivityPhase::Channel,
                        start,
                        ms_ticks(ms, self.rate).max(1),
                        phase.elapsed,
                        true,
                    )
                } else {
                    CurrentActivity {
                        activity,
                        phase: ActivityPhase::Channel,
                        cycle: None,
                        observed_at: self.tick,
                        started_at: start,
                        duration_ticks: None,
                    }
                }
            }

            Kind::IdleMove => return Some(self.rest_sample(profile)),
        };
        Some(sample)
    }

    fn rest_sample(&self, profile: ActivityProfile) -> CurrentActivity {
        if let Some(start) = self.run_since {
            let cycle = profile.run_ms.and_then(|duration| {
                if duration == 0 {
                    return Some(Fixed::ZERO);
                }
                let loop_raw = u64::from(duration).checked_mul(300)?.checked_mul(65536)? / 1000;
                (loop_raw > 0).then(|| fixed_cycle(self.run_distance, loop_raw, true))
            });
            return CurrentActivity {
                activity: Activity::Run,
                phase: ActivityPhase::Loop,
                cycle,
                observed_at: self.tick,
                started_at: start,
                duration_ticks: profile.run_ms.map(|ms| ms_ticks(ms, self.rate)),
            };
        }
        let start = self.idle_since.min(self.tick);
        if let (Some(idle), Some(rare), Some(id)) = (profile.idle_ms, profile.rare_idle_ms, self.id)
            && idle > 0
            && rare > 0
        {
            let idle = u64::from(ms_ticks(idle, self.rate).max(1));
            let rare = u64::from(ms_ticks(rare, self.rate).max(1));
            let loops = 4 + (id.idx ^ id.generation).wrapping_mul(0x9E37_79B9) % 4;
            let ordinary = idle * u64::from(loops);
            let phase = u64::from(self.tick.saturating_sub(start)) % (ordinary + rare);
            let is_rare = phase >= ordinary;
            let elapsed = u32::try_from(if is_rare {
                phase - ordinary
            } else {
                phase % idle
            })
            .unwrap_or(0);
            return self.timed(
                if is_rare {
                    Activity::IdleRare
                } else {
                    Activity::Idle
                },
                ActivityPhase::Loop,
                self.tick.saturating_sub(elapsed),
                u32::try_from(if is_rare { rare } else { idle }).unwrap_or(u32::MAX),
                elapsed,
                false,
            );
        }
        self.loop_sample(Activity::Idle, profile.idle_ms, start)
    }
    fn loop_sample(
        &self,
        activity: Activity,
        duration: Option<u32>,
        start: u32,
    ) -> CurrentActivity {
        match duration {
            Some(0) => CurrentActivity {
                activity,
                phase: ActivityPhase::Loop,
                cycle: Some(Fixed::ZERO),
                observed_at: self.tick,
                started_at: start,
                duration_ticks: Some(0),
            },
            Some(ms) => self.timed(
                activity,
                ActivityPhase::Loop,
                start,
                ms_ticks(ms, self.rate).max(1),
                self.tick.saturating_sub(start),
                true,
            ),
            None => CurrentActivity {
                activity,
                phase: ActivityPhase::Loop,
                cycle: None,
                observed_at: self.tick,
                started_at: start,
                duration_ticks: None,
            },
        }
    }
    fn timed(
        &self,
        activity: Activity,
        phase: ActivityPhase,
        start: u32,
        duration: u32,
        elapsed: u32,
        repeat: bool,
    ) -> CurrentActivity {
        CurrentActivity {
            activity,
            phase,
            cycle: Some(fixed_cycle(
                u64::from(elapsed),
                u64::from(duration.max(1)),
                repeat,
            )),
            observed_at: self.tick,
            started_at: start,
            duration_ticks: Some(duration),
        }
    }
}
fn ability_slot(unit: &UnitView, ability: AbilityId) -> Option<AbilitySlot> {
    unit.abilities
        .iter()
        .position(|entry| entry.id == ability)
        .and_then(|i| u8::try_from(i).ok())
        .map(AbilitySlot)
}
fn ms_ticks(ms: u32, rate: u16) -> u32 {
    u32::try_from((u64::from(ms) * u64::from(rate.max(1))).div_ceil(1000)).unwrap_or(u32::MAX)
}
fn fixed_cycle(elapsed: u64, duration: u64, repeat: bool) -> Fixed {
    if duration == 0 {
        return Fixed::ZERO;
    }
    let elapsed = if repeat {
        elapsed % duration
    } else {
        elapsed.min(duration)
    };
    Fixed {
        raw: i32::try_from((u128::from(elapsed) * 65536 / u128::from(duration)).min(65535))
            .unwrap_or(65535),
    }
}
fn distance_raw(delta: bota_proto::Vec2) -> u64 {
    let x = i128::from(delta.x.raw);
    let y = i128::from(delta.y.raw);
    u64::try_from((x * x + y * y).unsigned_abs().isqrt()).unwrap_or(u64::MAX)
}
fn is_running(track: &EntityTrack, rate: u16) -> bool {
    let Some(velocity) = track.velocity else {
        return false;
    };
    if velocity.elapsed_ticks == 0 {
        return false;
    }
    let distance = distance_raw(velocity.delta);
    if distance > u64::from(INSTANT_MOVE_DISTANCE) * 65536 {
        return false;
    }
    u128::from(distance) * u128::from(rate) > 25 * 65536 * u128::from(velocity.elapsed_ticks)
}

#[cfg(test)]
mod cycle_bounds {
    #[test]
    fn non_looping_cycles_exclude_one_at_and_after_the_endpoint() {
        for (elapsed, duration, expected) in [
            (0, 1, 0),
            (1, 1, 65535),
            (2, 1, 65535),
            (u64::MAX, 1, 65535),
        ] {
            assert_eq!(super::fixed_cycle(elapsed, duration, false).raw, expected);
        }
        assert_eq!(super::fixed_cycle(1, 1, true).raw, 0);
    }
}
