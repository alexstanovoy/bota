//! FNV-1a over whatever a run is made of.
//!
//! What goes in and in which order is the caller's business; this only eats
//! bytes and gives a number, the same one on every platform.

use bota_proto::{Angle, Fixed, Team, UnitKind, Vec2};

use crate::engine::Entity;

/// FNV-1a over the state a tick depends on.
pub struct Fnv {
    /// What has been eaten so far.
    state: u64,
}

impl Default for Fnv {
    fn default() -> Self {
        Fnv::new()
    }
}

impl Fnv {
    /// A fingerprint of nothing.
    pub fn new() -> Fnv {
        Fnv {
            state: 0xcbf2_9ce4_8422_2325,
        }
    }

    /// Eats one byte.
    pub fn u8(&mut self, value: u8) {
        self.state ^= u64::from(value);
        self.state = self.state.wrapping_mul(0x0000_0100_0000_01b3);
    }

    /// Eats four bytes, low first.
    pub fn u32(&mut self, value: u32) {
        for byte in value.to_le_bytes() {
            self.u8(byte);
        }
    }

    /// Eats eight bytes, low first.
    pub fn u64(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.u8(byte);
        }
    }

    /// Eats a signed four bytes.
    pub fn i32(&mut self, value: i32) {
        self.u32(value as u32);
    }

    /// Eats a fixed-point number.
    pub fn fixed(&mut self, value: Fixed) {
        self.i32(value.raw);
    }

    /// Eats a position.
    pub fn vec2(&mut self, value: Vec2) {
        self.fixed(value.x);
        self.fixed(value.y);
    }

    /// Eats a facing.
    pub fn angle(&mut self, value: Angle) {
        self.u32(u32::from(value.brads));
    }

    /// Eats a side.
    pub fn team(&mut self, value: Team) {
        self.u8(match value {
            Team::Radiant => 0,
            Team::Dire => 1,
            Team::Neutral => 2,
        });
    }

    /// Eats an entity handle.
    pub fn entity(&mut self, value: Entity) {
        self.u32(value.index().0);
        self.u32(value.generation().0.get());
    }

    /// Eats a kind of unit.
    pub fn kind(&mut self, value: UnitKind) {
        self.u8(value as u8);
    }

    /// Eats what is present or not.
    pub fn some(&mut self, present: bool) {
        self.u8(u8::from(present));
    }

    /// The number so far.
    pub fn done(&self) -> u64 {
        self.state
    }
}
