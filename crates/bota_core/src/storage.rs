//! The storage classes whose game rules decide whether carried items work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageClass {
    Inventory,
    Backpack,
    Stash,
    Dedicated,
}
/// O(1). Dedicated equipment slots are inventory slots; unsupported slots have no class.
pub const fn storage_class(slot: i32) -> Option<StorageClass> {
    match slot {
        0..=5 => Some(StorageClass::Inventory),
        6..=8 => Some(StorageClass::Backpack),
        9..=14 => Some(StorageClass::Stash),
        15 => Some(StorageClass::Dedicated),
        _ => None,
    }
}
