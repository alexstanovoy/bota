use crate::{missing_parts, purchase_requirements};
use bota_proto::{Attribute, ItemId};
#[test]
fn allocation_preserves_duplicate_components_and_the_selected_recipe() {
    let owned = [
        ItemId::SLIPPERS,
        ItemId::SLIPPERS,
        ItemId::SLIPPERS,
        ItemId::CIRCLET,
    ];
    let first = purchase_requirements(ItemId::WRAITH_BAND, &owned, Some(Attribute::Agility));
    assert_eq!(
        first.wanted,
        missing_parts(ItemId::WRAITH_BAND, &owned, Some(Attribute::Agility))
    );
    assert_eq!(
        first.remaining_owned,
        vec![ItemId::SLIPPERS, ItemId::SLIPPERS]
    );
    let owned = [ItemId::BOOTS, ItemId::GLOVES, ItemId::BAND, ItemId::BELT];
    let first = purchase_requirements(ItemId::POWER_TREADS, &owned, Some(Attribute::Agility));
    assert!(first.wanted.is_empty());
    assert_eq!(first.remaining_owned, vec![ItemId::BELT]);
}
