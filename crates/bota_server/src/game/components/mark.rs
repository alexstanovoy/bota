use bota_proto::{AbilityId, Vec2};

use crate::engine::Entity;
use crate::engine::state_hash;

/// Something an ability leaves in the world to be seen: a burst where a
/// raze landed, the rot about its caster, a hold on what a dismember eats,
/// a link of a hook's chain. It takes no room and does nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub ability: AbilityId,
    pub owner: Entity,
}

state_hash!(Mark { ability, owner });

/// A boulder released toward a fixed impact point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoulderFlight {
    pub origin: Vec2,
    pub aim: Vec2,
    pub launched: u32,
    pub ticks: u32,
    pub level: u8,
}

state_hash!(BoulderFlight {
    origin,
    aim,
    launched,
    ticks,
    level
});
