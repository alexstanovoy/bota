use super::readiness::illusion;
use super::{OrderContext, UnknownReason, Verdict, item_in};
use bota_proto::{ItemId, ItemSlot, ItemView, RejectReason, Target, UnitKind, UnitView};

use bota_core::{BAG_SLOTS, in_stash};

fn stash(slot: ItemSlot) -> bool {
    in_stash(usize::from(slot.0))
}

fn facts(item: &ItemView) -> bota_core::OrderItemFacts {
    bota_core::OrderItemFacts {
        id: item.id,
        owner: item.owner,
        rune: item.rune.is_some(),
        free_charges: 0,
    }
}

impl OrderContext<'_> {
    fn inventory<'a>(&'a self, body: &'a UnitView) -> InventoryContext<'a> {
        InventoryContext::new(
            body,
            self.player.stash.as_deref(),
            self.player.slot,
            bota_core::shop_access(self.map, body.pos, body.bound),
            self.player.respawn_left > 0,
        )
    }

    pub(super) fn switch(&self, body: &UnitView, slot: ItemSlot) -> Verdict {
        self.inventory(body).switch(slot)
    }
    pub(super) fn swap(&self, body: &UnitView, from: ItemSlot, to: ItemSlot) -> Verdict {
        self.inventory(body).swap(from, to)
    }
    pub(super) fn sell(&self, body: &UnitView, slot: ItemSlot) -> Verdict {
        self.inventory(body).sell(slot)
    }
    pub(super) fn disassemble(&self, body: &UnitView, slot: ItemSlot) -> Verdict {
        self.inventory(body).disassemble(slot)
    }
    pub(super) fn lock(&self, body: &UnitView, slot: ItemSlot) -> Verdict {
        self.inventory(body).lock(slot)
    }

    pub(super) fn put(&self, body: &UnitView, slot: ItemSlot, target: Target) -> Verdict {
        if let Err(reason) = bota_core::put_item_refusal(
            usize::from(slot.0),
            illusion(body),
            item_in(body, slot).map(facts),
        ) {
            return Verdict::Illegal(reason);
        }
        match target {
            Target::None => Verdict::Legal,
            Target::Pos(_) => Verdict::Unknown(UnknownReason::DynamicGround),
            Target::Unit(id) => {
                if !self.seen(id) {
                    return Verdict::Illegal(RejectReason::UnknownTarget);
                }
                let mark = self.unit(id);
                bota_core::put_recipient_refusal(
                    id == body.id,
                    mark.is_some_and(|mark| mark.team == self.player.team),
                    mark.zip(item_in(body, slot))
                        .is_some_and(|(mark, item)| self.receives(mark, item.id)),
                )
                .into()
            }
        }
    }

    fn receives(&self, body: &UnitView, item: ItemId) -> bool {
        bota_core::may_receive_item(
            matches!(body.kind, UnitKind::Hero | UnitKind::Courier) && !illusion(body),
            body.kind == UnitKind::Hero && !illusion(body),
            item,
        )
    }

    pub(super) fn take(&self, body: &UnitView, target: Target) -> Verdict {
        let Target::Unit(id) = target else {
            return Verdict::Illegal(RejectReason::WrongTargetKind);
        };
        if !self.seen(id) {
            return Verdict::Illegal(RejectReason::UnknownTarget);
        }
        bota_core::take_refusal(
            self.view
                .runes
                .iter()
                .take(bota_proto::limits::MAX_SNAPSHOT_RUNES)
                .any(|rune| rune.id == id),
            body.kind == UnitKind::Hero && !illusion(body),
            self.view
                .loot
                .iter()
                .take(bota_proto::limits::MAX_SNAPSHOT_LOOT)
                .find(|loot| loot.id == id)
                .is_some_and(|loot| self.receives(body, loot.item)),
        )
        .into()
    }
}

/// Inventory admission when a caller already knows physical shop access.
/// The selected unit and stash must be from the seat's own side. Shop booleans are
/// facts from the match's shop geometry, not predictions of where the unit can walk.
pub struct InventoryContext<'a> {
    items: &'a [Option<ItemView>],
    // The outer option is slot presence; the inner option is its contents.
    town_portal: Option<&'a Option<ItemView>>,
    copied: bool,
    stash: Option<&'a [Option<ItemView>]>,
    owner: bota_proto::SlotId,
    shops: bota_core::ShopAccess,
    seat_dead: bool,
    respawning: bool,
}

impl<'a> InventoryContext<'a> {
    /// `seat_dead` describes the seat's hero, even when this selected unit is a living courier.
    pub fn new(
        unit: &'a UnitView,
        stash: Option<&'a [Option<ItemView>]>,
        owner: bota_proto::SlotId,
        shops: bota_core::ShopAccess,
        seat_dead: bool,
    ) -> Self {
        Self {
            items: &unit.items,
            town_portal: (unit.kind == UnitKind::Hero).then_some(&unit.town_portal),
            copied: illusion(unit),
            stash,
            owner,
            shops,
            seat_dead,
            respawning: false,
        }
    }

    pub(super) fn fallen(
        kit: &'a bota_proto::Kit,
        stash: Option<&'a [Option<ItemView>]>,
        owner: bota_proto::SlotId,
    ) -> Self {
        Self {
            items: &kit.items,
            town_portal: Some(&kit.town_portal),
            copied: false,
            stash,
            owner,
            shops: Default::default(),
            seat_dead: true,
            respawning: !kit.reincarnating,
        }
    }

    fn held(&self, slot: ItemSlot) -> Option<&ItemView> {
        if stash(slot) {
            self.stash?.get(usize::from(slot.0) - BAG_SLOTS)?.as_ref()
        } else if slot == ItemSlot::TOWN_PORTAL {
            self.town_portal.and_then(|slot| slot.as_ref())
        } else {
            self.items.get(usize::from(slot.0))?.as_ref()
        }
    }

    fn unknown_stash(&self, slot: ItemSlot) -> bool {
        stash(slot) && self.stash.is_none()
    }

    pub fn switch(&self, slot: ItemSlot) -> Verdict {
        bota_core::switch_refusal(usize::from(slot.0), self.held(slot).map(|item| item.id)).into()
    }

    pub fn swap(&self, from: ItemSlot, to: ItemSlot) -> Verdict {
        if self.unknown_stash(from) || self.unknown_stash(to) {
            return Verdict::Unknown(UnknownReason::HiddenState);
        }
        bota_core::swap_refusal(
            usize::from(from.0),
            usize::from(to.0),
            self.held(from).map(facts),
            self.held(to).map(facts),
            bota_core::SwapConditions {
                copied: self.copied,
                stash_slots: self.stash.map_or(0, <[Option<ItemView>]>::len),
                stash_access: self.respawning || self.shops.base,
                has_portal: self.town_portal.is_some(),
            },
        )
        .into()
    }

    pub fn sell(&self, slot: ItemSlot) -> Verdict {
        if self.unknown_stash(slot) {
            return Verdict::Unknown(UnknownReason::HiddenState);
        }
        let held = self.held(slot);
        let checked: Verdict = bota_core::sale_refusal(
            usize::from(slot.0),
            self.copied,
            held.map(facts),
            self.owner,
            self.shops.base || self.shops.secret,
        )
        .into();
        if checked.is_legal()
            && held.is_some_and(|item| item.charges.is_some_and(|count| count > 0))
        {
            Verdict::Unknown(UnknownReason::PurchaseHistory)
        } else {
            checked
        }
    }

    pub fn disassemble(&self, slot: ItemSlot) -> Verdict {
        let owned = self.owned_physical(slot);
        if !owned.is_legal() {
            return owned;
        }
        bota_core::disassembly_refusal(
            stash(slot),
            self.seat_dead,
            self.shops.base,
            self.held(slot)
                .is_some_and(|item| item.disassembly.is_some()),
        )
        .into()
    }

    pub fn lock(&self, slot: ItemSlot) -> Verdict {
        let owned = self.owned_physical(slot);
        if !owned.is_legal() {
            return owned;
        }
        self.held(slot)
            .map_or(Verdict::Illegal(RejectReason::EmptySlot), |item| {
                bota_core::lock_refusal(item.id).into()
            })
    }

    fn owned_physical(&self, slot: ItemSlot) -> Verdict {
        if self.unknown_stash(slot) {
            return Verdict::Unknown(UnknownReason::HiddenState);
        }
        bota_core::owned_item(
            usize::from(slot.0),
            self.copied,
            self.held(slot).map(facts),
            self.owner,
        )
        .map(|_| ())
        .into()
    }
}
