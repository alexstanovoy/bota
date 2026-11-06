//! A map boss's life, held across the entities each respawn stands up.
use crate::{engine::state_hash, game::Entity};
use bota_proto::Vec2;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lair {
    pub unit: Option<Entity>,
    pub falls: u32,
    pub last_killer: Option<bota_proto::Team>,
    pub respawn_at: u32,
    pub home: Vec2,
    /// A natural pit change is under way; damage never calls it off.
    pub travelling: bool,
}
state_hash!(Lair {
    unit,
    falls,
    last_killer,
    respawn_at,
    home,
    travelling
});
