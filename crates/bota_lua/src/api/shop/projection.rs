//! Bounded reservations for purchases issued before the next received snapshot.

use super::Purchase;
use crate::state::{Player, Snapshot};
use bota_core::{Berth, Counts, HeldItem, PurchaseCarrier, PurchaseContext, Shelf, ShopAccess};
use bota_proto::{Attribute, EntityId, ItemId, ItemView, SlotId, StockView, Team, UnitKind};

#[derive(Clone, Copy)]
struct StoredItem {
    counts: Counts,
    owner: SlotId,
    mode: Option<Attribute>,
    selling: bool,
    locked: bool,
}

impl StoredItem {
    fn observed(item: &ItemView) -> Self {
        Self {
            counts: Counts {
                id: item.id,
                charges: item
                    .dispenser
                    .map_or(item.charges.unwrap_or(0), |counts| counts.observers),
                sentries: item.dispenser.map_or(0, |counts| counts.sentries),
            },
            owner: item.owner,
            mode: item.mode,
            selling: item.selling == Some(true),
            locked: item.locked == Some(true),
        }
    }
    fn bought(id: ItemId, owner: SlotId) -> Option<Self> {
        let item = bota_core::item_def(id)?;
        Some(Self {
            counts: Counts {
                id,
                charges: item.charges,
                sentries: 0,
            },
            owner,
            mode: item.mode,
            selling: false,
            locked: false,
        })
    }
}

impl HeldItem for StoredItem {
    fn id(&self) -> ItemId {
        self.counts.id
    }
    fn charges(&self) -> u8 {
        self.counts.charges
    }
    fn sentries(&self) -> u8 {
        self.counts.sentries
    }
    fn owner(&self) -> SlotId {
        self.owner
    }
    fn mode(&self) -> Option<Attribute> {
        self.mode
    }
    fn for_sale(&self) -> bool {
        self.selling
    }
    fn locked(&self) -> bool {
        self.locked
    }
}

#[derive(Clone)]
struct Bag {
    slots: Vec<Option<StoredItem>>,
    portal: Option<StoredItem>,
}

impl Bag {
    fn observed(slots: &[Option<ItemView>], portal: Option<&ItemView>) -> Self {
        Self {
            slots: slots
                .iter()
                .map(|item| item.as_ref().map(StoredItem::observed))
                .collect(),
            portal: portal.map(StoredItem::observed),
        }
    }
    fn shelf(&self, portal: bool, combines: bool) -> Shelf<'_, StoredItem> {
        Shelf {
            slots: &self.slots,
            town_portal: portal.then_some(&self.portal),
            combines,
        }
    }
    fn put(&mut self, slot: usize, incoming: StoredItem, combines: bool) -> Option<()> {
        let held = if slot == bota_core::rules::TOWN_PORTAL_SLOT {
            &mut self.portal
        } else {
            self.slots.get_mut(slot)?
        };
        if let Some(previous) = held {
            let room = bota_core::merge_room(previous, &incoming, combines);
            if room == 0 {
                return None;
            }
            previous.counts = bota_core::merge_counts(previous.counts, incoming.counts, room).0;
        } else {
            *held = Some(incoming);
        }
        Some(())
    }
}

#[derive(Clone)]
struct Inventory {
    unit: EntityId,
    slot: SlotId,
    gold: i32,
    primary: Option<Attribute>,
    bag: Bag,
    stash: Bag,
    courier: Bag,
    hero_shop: Option<ShopAccess>,
    courier_shop: Option<ShopAccess>,
}

impl Inventory {
    fn observed(snapshot: &Snapshot, player: &Player) -> Option<Self> {
        let unit = player.unit?;
        let hero = snapshot.units.iter().find(|known| known.unit.id == unit);
        let courier = snapshot.units.iter().find(|known| {
            known.unit.kind == UnitKind::Courier && known.unit.owner == Some(player.slot)
        });
        let bag = player
            .kit
            .as_ref()
            .map(|kit| kit.items.as_slice())
            .or_else(|| hero.map(|known| known.unit.items.as_slice()))
            .unwrap_or_default();
        let portal = player
            .kit
            .as_ref()
            .map(|kit| &kit.town_portal)
            .or_else(|| hero.map(|known| &known.unit.town_portal))
            .and_then(Option::as_ref);
        Some(Self {
            unit,
            slot: player.slot,
            gold: player.gold?,
            primary: bota_core::hero_def(player.hero).and_then(|hero| hero.unit.primary),
            bag: Bag::observed(bag, portal),
            stash: Bag::observed(player.stash.as_deref()?, None),
            courier: Bag::observed(
                courier.map_or(&[], |known| known.unit.items.as_slice()),
                None,
            ),
            hero_shop: hero
                .filter(|known| known.visible && known.unit.hp > 0 && player.respawn_left == 0)
                .map(|known| super::carrier(snapshot.map, &known.unit).shops),
            courier_shop: courier
                .filter(|known| known.visible && known.unit.hp > 0)
                .map(|known| super::carrier(snapshot.map, &known.unit).shops),
        })
    }

    fn context(&self, universal_shop: bool) -> PurchaseContext<'_, StoredItem> {
        PurchaseContext {
            hero: self.hero_shop.map(|shops| PurchaseCarrier {
                bag: self.bag.shelf(true, true),
                shops,
            }),
            courier: self.courier_shop.map(|shops| PurchaseCarrier {
                bag: self.courier.shelf(false, true),
                shops,
            }),
            stash: self.stash.shelf(false, false),
            universal_shop,
        }
    }

    fn plan(&self, snapshot: &Snapshot, item: ItemId) -> Result<Plan, Purchase> {
        let held = bota_core::purchase_pool(
            self.slot,
            self.bag
                .slots
                .iter()
                .chain(&self.stash.slots)
                .chain(&self.courier.slots)
                .flatten(),
        );
        let parts = bota_core::missing_parts(item, &held, self.primary);
        let mut cost = 0i32;
        for &part in &parts {
            cost = cost.saturating_add(
                snapshot
                    .catalogue
                    .sold(part)
                    .ok_or(Purchase::Disallowed)?
                    .cost,
            );
        }
        let berths = bota_core::place_shop_purchase(
            self.slot,
            &parts,
            &self.context(snapshot.map.universal_shop),
        )
        .map_err(|reason| match reason {
            bota_proto::RejectReason::NotAtShop => Purchase::NotAtSecretShop,
            bota_proto::RejectReason::InventoryFull => Purchase::OutOfStock,
            _ => Purchase::Disallowed,
        })?;
        if cost > self.gold {
            return Err(Purchase::InsufficientGold);
        }
        Ok(Plan {
            parts,
            berths,
            cost,
        })
    }

    fn apply(&mut self, plan: Plan) -> Option<()> {
        for (part, berth) in plan.parts.into_iter().zip(plan.berths) {
            let item = StoredItem::bought(part, self.slot)?;
            match berth {
                Berth::Bag(slot) => self.bag.put(slot, item, true)?,
                Berth::Stash(slot) => self.stash.put(slot, item, false)?,
                Berth::Courier(slot) => self.courier.put(slot, item, true)?,
            }
        }
        // Assembly runs at upkeep after ordered purchases, so retain the parts for this turn.
        self.gold -= plan.cost;
        Some(())
    }
}

struct Plan {
    parts: Vec<ItemId>,
    berths: Vec<Berth>,
    cost: i32,
}

#[derive(Clone)]
pub(crate) struct Projection {
    inventories: Vec<Inventory>,
    stock: Vec<StockView>,
    team: Team,
}

impl Projection {
    pub(super) fn observed(snapshot: &Snapshot) -> Self {
        Self {
            inventories: snapshot
                .players
                .iter()
                .filter(|player| player.team == snapshot.team)
                .filter_map(|player| Inventory::observed(snapshot, player))
                .collect(),
            stock: snapshot.stock.clone(),
            team: snapshot.team,
        }
    }

    pub(crate) fn gold(&self, slot: SlotId) -> Option<i32> {
        self.inventories
            .iter()
            .find(|inventory| inventory.slot == slot)
            .map(|inventory| inventory.gold)
    }

    pub(super) fn buy(&mut self, snapshot: &Snapshot, unit: EntityId, item: ItemId) -> Purchase {
        if snapshot.catalogue.sold(item).is_none() {
            return Purchase::Disallowed;
        }
        if self
            .stock
            .iter()
            .find(|stock| stock.team == self.team && stock.item == item)
            .is_some_and(|stock| stock.left == 0)
        {
            return Purchase::OutOfStock;
        }
        let Some(inventory) = self
            .inventories
            .iter_mut()
            .find(|inventory| inventory.unit == unit)
        else {
            return Purchase::Disallowed;
        };
        let plan = match inventory.plan(snapshot, item) {
            Ok(plan) => plan,
            Err(result) => return result,
        };
        if inventory.apply(plan).is_none() {
            return Purchase::Disallowed;
        }
        if let Some(stock) = self
            .stock
            .iter_mut()
            .find(|stock| stock.team == self.team && stock.item == item)
        {
            stock.left -= 1;
        }
        Purchase::Success
    }
}

const _: () = assert!(
    4 * (10 * (std::mem::size_of::<Inventory>() + 22 * std::mem::size_of::<StoredItem>())
        + bota_proto::limits::MAX_VIEW_STOCK * std::mem::size_of::<StockView>())
        < 1 << 20
);
