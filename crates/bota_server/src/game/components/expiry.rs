use crate::engine::state_hash;

/// The time an entity has left in the world; it is taken out when this runs
/// out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Expiry {
    /// Ticks before it goes.
    pub ticks_left: u32,
}

state_hash!(Expiry { ticks_left });
