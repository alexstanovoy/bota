//! What a creep keeps about the fight it is in and the ground it holds.

use bota_proto::Vec2;

use crate::game::Entity;

/// What a lane creep keeps while it is chasing or coming back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneAi {
    /// Where its target was last seen. Absent when it never lost one.
    pub last_seen: Option<Vec2>,
    /// The tick until which its target is kept whatever else turns up. What
    /// an attack order puts on it.
    pub keep_until: u32,
    /// Whoever last roused it with an attack order, until the choosing has
    /// taken account of it.
    pub roused_by: Option<Entity>,
    /// Whether that order was aimed at one of the orderer's own, which puts
    /// the orderer last rather than first.
    pub roused_at_own: bool,
    /// The tick by which the chase is given up unless it lands a blow first.
    /// Pushed back every time the target comes into reach.
    pub chase_until: u32,
}

/// Which camp a neutral belongs to and where in it it stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CampHome {
    /// The camp's number on the map.
    pub camp: u8,
    /// The spot it walks back to.
    pub home: Vec2,
}

/// What a neutral keeps about being drawn away from its camp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeutralAi {
    /// Ticks it may still be led before it turns for home.
    pub leash_left: u32,
    /// Ticks before it will take a target again.
    pub reaggro_block: u32,
    /// Ticks until its next chance to notice anybody.
    pub next_window: u32,
    /// Whether it is walking home and taking nothing on.
    pub going_home: bool,
    /// Who struck the camp last, until the mind takes it up.
    pub roused_by: Option<Entity>,
    /// Whether it is awake. Asleep it takes nothing on, however near.
    pub awake: bool,
}
