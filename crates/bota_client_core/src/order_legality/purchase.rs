use super::{OrderContext, UnknownReason, Verdict};
use bota_core::{
    Berth, HeldItem, PurchaseContext, bounded_purchase_parts, item_def, place_shop_purchase_into,
    purchase_pool_iter, purchase_stock_available,
};
use bota_proto::{ItemId, ItemView, RejectReason};

impl OrderContext<'_> {
    pub(super) fn buy(&self, item: ItemId) -> Verdict {
        if self.catalogue.sold(item).is_none() {
            return Verdict::Illegal(RejectReason::UnknownItem);
        }
        if self.player.stash.is_none() || self.player.gold.is_none() {
            return Verdict::Unknown(UnknownReason::HiddenState);
        }
        let context = crate::shop::purchase_context_on(
            self.view,
            self.player,
            self.map,
            self.map.universal_shop,
        );
        if stacks(&context).any(|item| !purchase_facts_valid(item)) {
            return Verdict::Unknown(UnknownReason::MalformedView);
        }
        let mut held = [ItemId(0); crate::OWN_ITEM_SLOTS];
        let mut length = 0;
        for item in purchase_pool_iter(self.player.slot, stacks(&context)) {
            let Some(place) = held.get_mut(length) else {
                return Verdict::Unknown(UnknownReason::MalformedView);
            };
            *place = item;
            length += 1;
        }
        let primary = bota_core::hero_def(self.player.hero).and_then(|hero| hero.unit.primary);
        let Some(plan) =
            bounded_purchase_parts::<{ crate::OWN_ITEM_SLOTS }>(item, &held[..length], primary)
        else {
            return Verdict::Unknown(UnknownReason::MalformedView);
        };
        let parts = plan.wanted();
        let price = parts.iter().fold(0_i32, |sum, part| {
            sum.saturating_add(item_def(*part).map_or(0, |def| def.cost))
        });
        if self.player.gold.is_some_and(|gold| gold < price) {
            return Verdict::Illegal(RejectReason::NotEnoughGold);
        }
        let mut berths = [Berth::Bag(0); bota_core::MAX_PURCHASE_TREE];
        if let Err(reason) =
            place_shop_purchase_into(self.player.slot, parts, &context, &mut berths)
        {
            return Verdict::Illegal(reason);
        }
        self.purchase_stock(parts)
    }

    fn purchase_stock(&self, parts: &[ItemId]) -> Verdict {
        let shelf = |id| {
            self.view
                .stock
                .iter()
                .take(bota_proto::limits::MAX_VIEW_STOCK)
                .find(|stock| stock.team == self.player.team && stock.item == id)
        };
        if parts.iter().any(|part| {
            item_def(*part).is_some_and(|def| def.stock.is_some()) && shelf(*part).is_none()
        }) {
            return Verdict::Unknown(UnknownReason::HiddenState);
        }
        Verdict::from_check(
            purchase_stock_available(parts, |id| shelf(id).map(|stock| stock.left)),
            RejectReason::OutOfStock,
        )
    }
}

fn stacks<'a>(context: &'a PurchaseContext<'_, ItemView>) -> impl Iterator<Item = &'a ItemView> {
    context
        .hero
        .iter()
        .flat_map(|carrier| carrier.bag.slots.iter())
        .chain(context.stash.slots)
        .chain(
            context
                .courier
                .iter()
                .flat_map(|carrier| carrier.bag.slots.iter()),
        )
        .flatten()
}

fn purchase_facts_valid(item: &ItemView) -> bool {
    item_def(item.id).is_some_and(|def| {
        item.selling.is_some()
            && item.locked.is_some()
            && (def.stack_limit == 0 || item.charges() <= def.stack_limit)
    })
}
