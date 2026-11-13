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
        ability: AbilityId,
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
struct CastClock {
    ability: Option<AbilityId>,
    slot: Option<AbilitySlot>,
    started_at: u32,
    point: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CastRecovery {
    ability: AbilityId,
    slot: Option<AbilitySlot>,
    at: u32,
    point: u32,
    backswing: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObservedCast {
    clock: CastClock,
    seen_at: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChannelClock {
    ability: AbilityId,
    slot: Option<AbilitySlot>,
    elapsed: u32,
}

/// Bounded action/clock memory for one tracked entity. Only visible observations advance it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActivityClock {
    tick: u32,
    rate: u16,
    id: Option<EntityId>,
    forced: bool,
    stunned: bool,
    forced_since: u32,
    stunned_since: u32,
    cast: Option<CastClock>,
    channel: Option<ChannelClock>,
    swing: Option<(EntityId, u32)>,
    windup: u32,
    recovery: u32,
    ordinal: u32,
    pending_attack: bool,
    windup_due: u32,
    launch: Option<u32>,
    cast_completed: Option<CastRecovery>,
    previous_cast: Option<ObservedCast>,
    idle_since: u32,
    run_since: Option<u32>,
    run_distance: u64,
    initialized: bool,
    was_busy: bool,
}

impl ActivityClock {
    pub(super) fn observe(&mut self, track: &EntityTrack, map: &bota_core::MapDef, rate: u16) {
        let tick = track.last_seen_tick;
        let unit = &track.unit;
        let definition = crate::unit_definition(map, unit);
        let windup = ms_ticks(unit.attack_point, rate);
        let recovery = definition.map_or(0, |d| {
            scaled_ticks(d.attack_backswing, unit.attack_speed, rate)
        });
        let swing = track
            .fight
            .and_then(|fight| fight.swing)
            .map(|s| (s.target, s.strike_in));
        let new_swing = swing.is_some()
            && (self.swing.is_none() || self.swing.map(|s| s.0) != swing.map(|s| s.0));
        if new_swing {
            self.ordinal = self.ordinal.wrapping_add(1);
            self.pending_attack = true;
        }
        if let Some((_, left)) = swing {
            self.windup_due = tick.saturating_add(left);
        }
        if swing.is_none() && tick > self.windup_due {
            self.pending_attack = false;
        }
        let launch = track.last_attack_launched.map(|launch| launch.tick);
        if launch != self.launch && launch.is_some() {
            if !self.pending_attack {
                self.ordinal = self.ordinal.wrapping_add(1);
            }
            self.pending_attack = false;
        }
        if let Some(cast) = track
            .last_ability_cast
            .filter(|cast| self.cast_completed.is_none_or(|old| old.at != cast.tick))
            && let Some(definition) = bota_core::ability_def(cast.ability)
            && !definition.passive
            && !definition.immediate
            && definition.point > 0
        {
            let previous = self.previous_cast.filter(|old| {
                old.clock.ability == Some(cast.ability)
                    && old.seen_at.saturating_add(1) >= cast.tick
            });
            self.cast_completed = Some(CastRecovery {
                ability: cast.ability,
                slot: ability_slot(unit, cast.ability),
                at: cast.tick,
                point: previous.map_or(ms_ticks(definition.point, rate), |old| old.clock.point),
                backswing: ms_ticks(definition.backswing, rate),
            });
        }
        let elapsed = tick.saturating_sub(self.tick);
        let moving = is_running(track, rate) && !bota_core::kind_def(unit.kind).fixed;
        let forced = unit.forced.is_some();
        let stunned = unit.statuses.bits & StatusFlags::STUNNED != 0;
        if forced && !self.forced {
            self.forced_since = tick;
        }
        if stunned && !self.stunned {
            self.stunned_since = tick;
        }
        let cast = unit.cast.map(|cast| {
            let start = track.casting_since.unwrap_or(tick);
            CastClock {
                ability: cast.ability,
                slot: cast.ability.and_then(|id| ability_slot(unit, id)),
                started_at: start,
                point: tick
                    .saturating_sub(start)
                    .saturating_add(cast.goes_off_in)
                    .max(1),
            }
        });
        if let Some(clock) = cast.filter(|clock| clock.ability.is_some()) {
            self.previous_cast = Some(ObservedCast {
                clock,
                seen_at: tick,
            });
        }
        let channel = unit.channel.map(|channel| ChannelClock {
            ability: channel.ability,
            slot: ability_slot(unit, channel.ability),
            elapsed: channel.elapsed,
        });
        let busy = forced || stunned || cast.is_some() || channel.is_some() || swing.is_some();
        if !self.initialized || self.run_since.is_some() != moving || busy || self.was_busy {
            self.idle_since = tick;
            self.run_since = moving.then_some(tick);
            self.run_distance = 0;
        } else if moving
            && elapsed > 0
            && let Some(velocity) = track.velocity
        {
            self.run_distance = self
                .run_distance
                .saturating_add(distance_raw(velocity.delta));
        }
        self.tick = tick;
        self.rate = rate.max(1);
        self.id = Some(unit.id);
        self.forced = forced;
        self.stunned = stunned;
        self.cast = cast;
        self.channel = channel;
        self.swing = swing;
        self.windup = windup;
        self.recovery = recovery;
        self.launch = launch;
        self.initialized = true;
        self.was_busy = busy;
    }

    /// Samples the same observation for any consumer; profiles supply only local clip timing.
    pub fn sample(&self, profile: ActivityProfile) -> Option<CurrentActivity> {
        if !self.initialized {
            return None;
        }
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
        if let Some(channel) = self.channel {
            return Some(self.loop_sample(
                Activity::Channel {
                    ability: channel.ability,
                    slot: channel.slot,
                },
                channel.slot.and_then(|slot| {
                    profile
                        .channel_ms
                        .get(usize::from(slot.0))
                        .copied()
                        .flatten()
                }),
                self.tick.saturating_sub(channel.elapsed),
            ));
        }
        if let Some(cast) = self.cast {
            let back = cast
                .ability
                .and_then(bota_core::ability_def)
                .map_or(0, |d| ms_ticks(d.backswing, self.rate));
            return Some(self.timed(
                Activity::Cast {
                    ability: cast.ability,
                    slot: cast.slot,
                },
                ActivityPhase::Windup,
                cast.started_at,
                cast.point.saturating_add(back).max(1),
                self.tick.saturating_sub(cast.started_at),
                false,
            ));
        }
        let alternate = profile.alternate_attacks && self.ordinal.is_multiple_of(2);
        if let Some((_, left)) = self.swing {
            let elapsed = self.windup.saturating_sub(left);
            return Some(self.timed(
                Activity::Attack { alternate },
                ActivityPhase::Windup,
                self.tick.saturating_sub(elapsed),
                self.windup.saturating_add(self.recovery).max(1),
                elapsed,
                false,
            ));
        }
        if self.run_since.is_none() {
            if let Some(at) = self
                .launch
                .filter(|&at| self.tick.saturating_sub(at) < self.recovery)
            {
                return Some(self.timed(
                    Activity::Attack { alternate },
                    ActivityPhase::Recovery,
                    at.saturating_sub(self.windup),
                    self.windup.saturating_add(self.recovery).max(1),
                    self.windup.saturating_add(self.tick.saturating_sub(at)),
                    false,
                ));
            }
            if let Some(cast) = self
                .cast_completed
                .filter(|old| self.tick.saturating_sub(old.at) < old.backswing)
            {
                return Some(self.timed(
                    Activity::Cast {
                        ability: Some(cast.ability),
                        slot: cast.slot,
                    },
                    ActivityPhase::Recovery,
                    cast.at.saturating_sub(cast.point),
                    cast.point.saturating_add(cast.backswing).max(1),
                    cast.point.saturating_add(self.tick.saturating_sub(cast.at)),
                    false,
                ));
            }
        }
        if let Some(start) = self.run_since {
            let cycle = profile.run_ms.and_then(|duration| {
                if duration == 0 {
                    return Some(Fixed::ZERO);
                }
                let loop_raw = u64::from(duration).checked_mul(300)?.checked_mul(65536)? / 1000;
                (loop_raw > 0).then(|| fixed_cycle(self.run_distance, loop_raw, true))
            });
            return Some(CurrentActivity {
                activity: Activity::Run,
                phase: ActivityPhase::Loop,
                cycle,
                observed_at: self.tick,
                started_at: start,
                duration_ticks: profile.run_ms.map(|ms| ms_ticks(ms, self.rate)),
            });
        }
        let start = self
            .idle_since
            .max(self.launch.map_or(0, |at| at.saturating_add(self.recovery)))
            .max(
                self.cast_completed
                    .map_or(0, |old| old.at.saturating_add(old.backswing)),
            );
        let start = start.min(self.tick);
        if let (Some(idle), Some(rare), Some(id)) = (profile.idle_ms, profile.rare_idle_ms, self.id)
            && idle > 0
            && rare > 0
        {
            let idle = u64::from(ms_ticks(idle, self.rate).max(1));
            let rare = u64::from(ms_ticks(rare, self.rate).max(1));
            let loops = 4 + (id.idx ^ id.generation).wrapping_mul(0x9E37_79B9) % 4;
            let ordinary = idle * u64::from(loops);
            let phase = u64::from(self.tick.saturating_sub(start)) % (ordinary + rare);
            if phase >= ordinary {
                let elapsed = u32::try_from(phase - ordinary).unwrap_or(u32::MAX);
                return Some(self.timed(
                    Activity::IdleRare,
                    ActivityPhase::Loop,
                    self.tick.saturating_sub(elapsed),
                    u32::try_from(rare).unwrap_or(u32::MAX),
                    elapsed,
                    false,
                ));
            }
            let elapsed = u32::try_from(phase % idle).unwrap_or(0);
            return Some(self.timed(
                Activity::Idle,
                ActivityPhase::Loop,
                self.tick.saturating_sub(elapsed),
                idle as u32,
                elapsed,
                false,
            ));
        }
        Some(self.loop_sample(Activity::Idle, profile.idle_ms, start))
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
fn scaled_ticks(ms: u32, attack_speed: i32, rate: u16) -> u32 {
    let Ok(speed) = u64::try_from(attack_speed) else {
        return 0;
    };
    if speed == 0 {
        return 0;
    }
    u32::try_from((u64::from(ms) * 100 * u64::from(rate.max(1))).div_ceil(speed * 1000))
        .unwrap_or(u32::MAX)
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
