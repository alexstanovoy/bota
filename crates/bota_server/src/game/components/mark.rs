//! What an ability shows where it stands.

use bota_proto::AbilityId;

use crate::engine::Entity;

/// Something an ability leaves in the world to be seen: a burst where a
/// raze landed, the rot about its caster, a hold on what a dismember eats,
/// a link of a hook's chain. It takes no room and does nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    /// Which ability left it.
    pub ability: AbilityId,
    /// Who cast that ability.
    pub owner: Entity,
}
