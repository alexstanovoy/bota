//! What an ability shows where it stands.

use bota_proto::AbilityId;

use crate::engine::Entity;

/// Something an ability leaves in the world to be seen: a burst where a
/// raze landed, a ring where a requiem went off, a hold on what a dismember
/// eats, a link of a hook's chain.
///
/// It takes no room and does nothing, and is seen like anything else
/// standing where it stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    /// Which ability left it.
    pub ability: AbilityId,
    /// Who cast that ability.
    pub owner: Entity,
}
