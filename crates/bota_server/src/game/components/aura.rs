use bota_core::Aura;

/// Everything an entity hands out. Absent when it hands out nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Auras(pub &'static [Aura]);
