//! The shop as a player uses it: what a seat holds towards a build, and the
//! quick-buy item.

use bota_core::Catalogue;
use bota_proto::ItemId;

use crate::shop::{held_items, missing_for, price_for, primary_of, whole_price};
use crate::ui::app::{App, Phase};

fn hero_primary(hero: bota_proto::HeroId) -> Option<bota_proto::Attribute> {
    bota_core::hero_def(hero).and_then(|def| def.unit.primary)
}

impl App {
    /// Physical access by one's hero or personal courier to either neutral secret shop.
    pub fn at_secret_shop(&self) -> bool {
        self.shop_context().is_some_and(|context| {
            context
                .hero
                .iter()
                .chain(&context.courier)
                .any(|carrier| carrier.shops.secret)
        })
    }

    pub(crate) fn shop_context(
        &self,
    ) -> Option<bota_core::PurchaseContext<'_, bota_proto::ItemView>> {
        Some(crate::purchase_context(
            self.view()?,
            self.player(self.my_slot?)?,
            self.statics()?,
        ))
    }

    pub fn unit_at_secret_shop(&self, id: bota_proto::EntityId) -> bool {
        self.unit_carrier(id)
            .is_some_and(|carrier| carrier.shops.secret)
    }

    pub(crate) fn unit_carrier(
        &self,
        id: bota_proto::EntityId,
    ) -> Option<bota_core::PurchaseCarrier<'_, bota_proto::ItemView>> {
        self.view()
            .and_then(|view| view.unit(id))
            .zip(self.statics())
            .and_then(|(unit, statics)| crate::shop::purchase_carrier(unit, statics))
    }

    pub fn secret_shop_accessible(&self) -> bool {
        self.session()
            .is_some_and(|session| session.statics().metadata().universal_shop)
            || self.at_secret_shop()
    }
    /// What this seat holds towards a build, and its hero's primary
    /// attribute. In the tactical phase it holds what it has queued.
    fn buyer(&self) -> (Vec<ItemId>, Option<bota_proto::Attribute>) {
        let buyer = || {
            let p = self.player(self.my_slot?)?;
            Some((held_items(self.view()?, p), primary_of(p)))
        };
        if self.in_tactical_phase() {
            let held = self.queued_buys.iter().map(|id| ItemId(*id)).collect();
            return (held, self.picked_hero().and_then(hero_primary));
        }
        buyer().unwrap_or_default()
    }

    /// Whether the picks are final and the match has not begun: the shop is
    /// open for choosing what to buy once it does.
    pub fn in_tactical_phase(&self) -> bool {
        self.phase == Phase::Lobby
            && self.my_slot.is_some()
            && matches!(self.lobby.pick, bota_proto::PickPhase::Tactical { .. })
    }

    /// The hero this seat has picked, as the lobby tells it.
    fn picked_hero(&self) -> Option<bota_proto::HeroId> {
        let slot = self.my_slot?;
        self.lobby.slots.iter().find(|s| s.slot == slot)?.hero
    }

    /// Gold left of the starting gold once the queued items are paid for.
    pub fn gold_after_queue(&self) -> i32 {
        let mut held: Vec<ItemId> = Vec::new();
        let primary = self.picked_hero().and_then(hero_primary);
        let mut spent = 0;
        for id in &self.queued_buys {
            spent += price_for(ItemId(*id), &held, primary);
            held.push(ItemId(*id));
        }
        bota_core::rules::STARTING_GOLD - spent
    }

    /// The gold a purchase can use now: what the queued items leave in the tactical phase, the
    /// seat's own gold in play, none to a watcher or before the seat has any.
    pub fn spendable_gold(&self) -> i32 {
        if self.in_tactical_phase() {
            return self.gold_after_queue();
        }
        self.my_slot
            .and_then(|slot| self.player(slot)?.gold)
            .unwrap_or(0)
    }

    /// The items the shop offers: the match's once it began, the lobby's before (what a match
    /// started now plays with), this build's whole catalogue until either told.
    pub fn catalogue(&self) -> Catalogue {
        self.statics()
            .map(|statics| statics.catalogue())
            .or(self.lobby.catalogue)
            .unwrap_or(Catalogue::FULL)
    }

    /// Whether the shop offers an item: one sold, in the match's catalogue.
    pub fn offers(&self, item: u16) -> bool {
        self.catalogue().sold(ItemId(item)).is_some()
    }

    /// Chooses an item to buy once the match begins, if the shop offers it and the starting
    /// gold still covers it after the others.
    pub fn queue_buy(&mut self, item: u16) {
        if !self.offers(item) {
            self.refuse(bota_proto::RejectReason::UnknownItem);
            return;
        }
        if self.price_of(item).0 > self.gold_after_queue() {
            self.refuse(bota_proto::RejectReason::NotEnoughGold);
            return;
        }
        self.queued_buys.push(item);
    }

    /// Takes the last choice of an item off the queue.
    pub fn unqueue_buy(&mut self, item: u16) {
        if let Some(at) = self.queued_buys.iter().rposition(|queued| *queued == item) {
            self.queued_buys.remove(at);
        }
    }

    /// Sends the first queued item as a buy order: a tick takes one order a
    /// seat, so the rest wait for the next snapshots. A refusal is shown as
    /// any other and the rest go on.
    pub fn send_next_queued_buy(&mut self) {
        if self.queued_buys.is_empty() || self.view().is_none() {
            return;
        }
        let item = self.queued_buys.remove(0);
        self.send_order(bota_proto::Order::Buy { item: ItemId(item) });
    }

    /// What this seat pays for an item now, and what it costs whole.
    pub fn price_of(&self, id: u16) -> (i32, i32) {
        let (held, primary) = self.buyer();
        (
            price_for(ItemId(id), &held, primary),
            whole_price(ItemId(id)).unwrap_or(0),
        )
    }

    /// The parts this seat still lacks for an item, in build order.
    pub fn missing_for(&self, id: u16) -> Vec<ItemId> {
        let (held, primary) = self.buyer();
        missing_for(ItemId(id), &held, primary)
    }

    /// Replace the queue, or append a requested copy, reserving ownership once.
    pub fn set_quick_buy(&mut self, item: u16, append: bool) {
        if !self.offers(item) {
            return;
        }
        let (held, primary) = self.buyer();
        if !self.quick_buy.add(item, append, &held, primary) {
            self.reject = Some((crate::Refused::QuickBuyFull, 2.5));
            self.cue(crate::CueKind::QuickBuyFull, 0.8, None);
        }
    }

    /// Unbought components in queue order, including entries beyond the visible rows.
    pub fn quick_buy_parts(&self) -> &[u16] {
        self.quick_buy.parts()
    }

    pub fn quick_buy_next(&self) -> Option<u16> {
        let gold = self.player(self.my_slot?)?.gold.unwrap_or(0);
        self.quick_buy_parts().iter().copied().find(|id| {
            self.offers(*id) && whole_price(ItemId(*id)).is_some_and(|price| price <= gold)
        })
    }
}
