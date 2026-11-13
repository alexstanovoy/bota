use crate::{missing_parts, purchase_parts, purchase_requirements};
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

#[test]
fn a_buy_order_sets_a_complete_unbuilt_copy_aside_and_counts_the_spares_toward_the_next() {
    let band = ItemId::WRAITH_BAND;
    let (circlet, slippers, recipe) = (
        ItemId::CIRCLET,
        ItemId::SLIPPERS,
        ItemId::RECIPE_WRAITH_BAND,
    );
    let order = |owned: &[ItemId]| purchase_parts(band, owned, None);
    assert_eq!(
        order(&[]),
        [circlet, slippers, recipe],
        "nothing held buys a whole copy"
    );
    assert_eq!(
        order(&[circlet]),
        [slippers, recipe],
        "no copy complete yet: what is lacking"
    );
    assert_eq!(
        order(&[circlet, slippers, recipe]),
        Vec::<ItemId>::new(),
        "one copy waiting, nothing beyond it"
    );
    assert_eq!(
        order(&[circlet, circlet, slippers, recipe]),
        [slippers, recipe]
    );
    assert_eq!(
        order(&[
            circlet, circlet, circlet, slippers, slippers, recipe, recipe
        ]),
        [slippers, recipe]
    );
    assert_eq!(
        order(&[circlet, circlet, slippers, slippers, recipe, recipe]),
        Vec::<ItemId>::new()
    );
    assert_eq!(
        order(&[ItemId::BOOTS]),
        [circlet, slippers, recipe],
        "a part of nothing here counts toward nothing"
    );
}
