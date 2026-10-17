use bota_core::UnitDef;

/// Which kind of unit an entity is. Points into the table, never a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Def(pub &'static UnitDef);
