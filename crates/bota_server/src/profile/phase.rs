//! The regions of a tick the phase profile tells apart.

/// Timed native regions; nested regions report inclusive durations.
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum Phase {
    Tick,
    Upkeep,
    Effects,
    Stats,
    Targeting,
    Movement,
    Visibility,
    Actions,
    Damage,
    Projection,
    Settle,
    TargetQuery,
    MovementIntent,
    Walk,
    Separation,
    BodyIndex,
    Route,
    PathQuery,
    LocalPlan,
    LocalSearch,
}
