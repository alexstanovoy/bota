//! The shop as a player uses it: what a seat holds towards a build, and the
//! quick-buy item.

use bota_core::Catalogue;
use bota_proto::ItemId;

use crate::shop::{held_items, missing_for, price_for, primary_of, whole_price};
use crate::ui::app::{App, Phase};

/// Whether this seat can buy an item now, including its remaining components.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PurchaseAvailability {
    Available,
    NoBuyer,
    NotOffered,
    InsufficientGold,
    OutOfStock,
    /// The required shops or storage berths cannot receive all missing parts.
    NoPlacement,
    NoShop,
    NothingToBuy,
}

impl PurchaseAvailability {
    pub(crate) fn rejection(self) -> Option<bota_proto::RejectReason> {
        use bota_proto::RejectReason as Why;
        Some(match self {
            Self::Available | Self::NoBuyer => return None,
            Self::NotOffered => Why::UnknownItem,
            Self::InsufficientGold => Why::NotEnoughGold,
            Self::OutOfStock => Why::OutOfStock,
            Self::NoPlacement => Why::InventoryFull,
            Self::NoShop => Why::NotAtShop,
            Self::NothingToBuy => Why::NothingToBuy,
        })
    }
}

fn hero_primary(hero: bota_proto::HeroId) -> Option<bota_proto::Attribute> {
    bota_core::hero_def(hero).and_then(|def| def.unit.primary)
}

impl App {
    /// The hero may move items between its kit and stash at either base,
    /// or while awaiting normal respawn. A dead courier gains no access.
    pub fn stash_accessible(&self) -> bool {
        self.my_standing_hero()
            .is_some_and(|hero| self.unit_at_base_shop(hero))
            || self
                .my_slot
                .and_then(|slot| self.player(slot))
                .is_some_and(|player| {
                    player.respawn_left > 0
                        && player.kit.as_ref().is_some_and(|kit| !kit.reincarnating)
                })
    }
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

    /// Whether this living carrier physically reaches the match's home shop.
    pub fn unit_at_home_shop(&self, id: bota_proto::EntityId) -> bool {
        self.unit_carrier(id)
            .is_some_and(|carrier| carrier.shops.base)
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

    /// One prediction for purchase selection and presentation. Explicit Buy intents
    /// still reach the server, which authoritatively accepts or refuses them.
    pub fn purchase_availability(&self, item: u16) -> PurchaseAvailability {
        if !self.offers(item) {
            return PurchaseAvailability::NotOffered;
        }
        let (Some(view), Some(shops), Some(slot)) = (self.view(), self.statics(), self.my_slot)
        else {
            return PurchaseAvailability::NoBuyer;
        };
        let Some(player) = view.player(slot) else {
            return PurchaseAvailability::NoBuyer;
        };
        let parts = self.missing_for(item);
        if parts.is_empty() {
            return PurchaseAvailability::NothingToBuy;
        }
        if self.price_of(item).0 > self.spendable_gold() {
            return PurchaseAvailability::InsufficientGold;
        }
        if parts.iter().any(|part| !self.offers(part.0)) {
            return PurchaseAvailability::NotOffered;
        }
        if !bota_core::purchase_stock_available(&parts, |part| {
            self.stock_of(part.0).map(|stock| stock.left)
        }) {
            return PurchaseAvailability::OutOfStock;
        }
        if let Err(reason) =
            crate::shop::purchase_placement(view, player, shops.as_ref(), ItemId(item))
        {
            return match reason {
                bota_proto::RejectReason::NotAtShop => PurchaseAvailability::NoShop,
                bota_proto::RejectReason::NothingToBuy => PurchaseAvailability::NothingToBuy,
                _ => PurchaseAvailability::NoPlacement,
            };
        }
        PurchaseAvailability::Available
    }

    pub(crate) fn refuse_purchase(&mut self, availability: PurchaseAvailability) {
        let Some(reason) = availability.rejection() else {
            return;
        };
        self.reject = Some((crate::Refused::Purchase(availability), 2.5));
        self.cue(crate::CueKind::Refused(reason), 0.8, None);
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

    /// Buy the first available component already allocated to the requested copies.
    pub fn quick_buy_next(&self) -> Option<u16> {
        self.quick_buy_parts()
            .iter()
            .copied()
            .find(|item| self.purchase_availability(*item) == PurchaseAvailability::Available)
    }
}
