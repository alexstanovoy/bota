//! What a stack is worth to the net worth a buyback is priced by: the whole division of a
//! consumable's cost by its charges, and the items that are worth their cost at any charge.

use bota_proto::ItemId;

use crate::{item_def, stack_worth};

fn worth(item: ItemId, charges: u8, free: u8) -> i32 {
    stack_worth(item_def(item).expect("in the catalogue"), charges, free)
}

#[test]
fn a_consumable_is_worth_the_whole_share_of_its_cost_its_paid_charges_are() {
    // A Tango costs 90 and comes with three charges.
    assert_eq!(
        [3, 2, 1, 0].map(|left| worth(ItemId::TANGO, left, 0)),
        [90, 60, 30, 0]
    );
    // A Raindrop costs 225 and comes with six: 225 * 5 / 6 is 187 and a half.
    assert_eq!(
        [6, 5, 1].map(|left| worth(ItemId::INFUSED_RAINDROP, left, 0)),
        [225, 187, 37]
    );
    // A stack past what one purchase holds is worth every charge; a free charge nothing.
    assert_eq!(worth(ItemId::TOWN_PORTAL_SCROLL, 5, 0), 500);
    assert_eq!(worth(ItemId::TOWN_PORTAL_SCROLL, 3, 1), 200);
    assert_eq!(worth(ItemId::TOWN_PORTAL_SCROLL, 1, 1), 0);
}

#[test]
fn what_stays_when_empty_and_what_comes_empty_is_worth_its_cost_at_any_charge() {
    for charges in [0, 1, 3] {
        assert_eq!(worth(ItemId::BOTTLE, charges, 0), 675);
    }
    for charges in [0, 10, 20] {
        assert_eq!(worth(ItemId::MAGIC_STICK, charges, 0), 200);
    }
}
