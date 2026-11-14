//! A travelling area effect, including the targets already reached by this cast.

use bota_proto::{ItemId, Team, Vec2};
use std::collections::BTreeSet;

use crate::engine::{Entity, state_hash};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AreaWave {
    pub owner: Entity,
    pub item: ItemId,
    pub side: Team,
    pub at: Vec2,
    pub elapsed: u32,
    pub aoe_bonus: i32,
    pub damage_amp_bp: i32,
    pub struck: BTreeSet<Entity>,
}

state_hash!(AreaWave {
    owner,
    item,
    side,
    at,
    elapsed,
    aoe_bonus,
    damage_amp_bp,
    struck
});
