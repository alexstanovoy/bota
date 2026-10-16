//! What a courier is in the middle of doing.

/// The errand a courier is on.
///
/// An errand outlives the tick it was given in: the courier keeps at it until
/// it is done or it is told something else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Errand {
    /// Nothing in particular. It stands or walks as it was last told.
    #[default]
    None,
    /// Carrying what it holds to its owner.
    ToOwner,
    /// Going to the stash to take what waits there.
    ToStash,
    /// Going to the stash to put back what it holds.
    PutBack,
    /// Going home and staying there.
    GoingHome,
}
