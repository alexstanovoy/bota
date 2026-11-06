//! A bounded queue of requested copies; owned components are allocated once in order.
use bota_proto::{Attribute, ItemId};

/// Internal bound on requested copies. Further requests are refused with feedback;
/// components beyond the visible rows remain queued.
pub const MAX_QUICK_BUY_TARGETS: usize = 64;

#[derive(Clone, Debug, Default)]
pub struct QuickBuyQueue {
    targets: Vec<u16>,
    parts: Vec<u16>,
    held: Vec<ItemId>,
    credited: Vec<ItemId>,
    primary: Option<Attribute>,
    dirty: bool,
}

impl QuickBuyQueue {
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
    pub fn targets(&self) -> &[u16] {
        &self.targets
    }
    pub fn parts(&self) -> &[u16] {
        &self.parts
    }
    pub fn clear(&mut self) {
        self.targets.clear();
        self.parts.clear();
        self.credited.clear();
    }

    pub(crate) fn add(
        &mut self,
        item: u16,
        append: bool,
        held: &[ItemId],
        primary: Option<Attribute>,
    ) -> bool {
        if !append {
            self.clear();
            self.held = held.to_vec();
        }
        self.sync(held, primary);
        if self.targets.len() < MAX_QUICK_BUY_TARGETS {
            self.targets.push(item);
            self.rebuild(held, primary);
            return true;
        }
        false
    }

    /// Basic purchases may merge into a stack, producing no new inventory slot.
    pub(crate) fn bought(&mut self, item: ItemId) {
        if bota_core::item_def(item).is_some_and(|def| def.recipes().next().is_none())
            && let Some(at) = self.targets.iter().position(|id| *id == item.0)
        {
            self.targets.remove(at);
            self.credited.push(item);
            self.dirty = true;
        }
    }

    pub(crate) fn sync(&mut self, held: &[ItemId], primary: Option<Attribute>) {
        if !self.dirty && self.held == held && self.primary == primary {
            return;
        }
        let mut arrived = held.to_vec();
        for item in self.held.iter().chain(&self.credited) {
            if let Some(at) = arrived.iter().position(|id| id == item) {
                arrived.remove(at);
            }
        }
        for item in arrived {
            if let Some(at) = self.targets.iter().position(|id| *id == item.0) {
                self.targets.remove(at);
            }
        }
        self.held = held.to_vec();
        self.credited.clear();
        self.dirty = false;
        self.rebuild(held, primary);
    }

    fn rebuild(&mut self, held: &[ItemId], primary: Option<Attribute>) {
        self.primary = primary;
        self.parts.clear();
        let mut spare = held.to_vec();
        for target in &self.targets {
            let plan = bota_core::purchase_requirements(ItemId(*target), &spare, primary);
            self.parts.extend(plan.wanted.iter().map(|id| id.0));
            spare = plan.remaining_owned;
            // A later build can use an earlier queued build, without duplicating its parts.
            spare.push(ItemId(*target));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_ownership_arrival_completes_one_copy_and_a_carrier_transfer_does_not_complete_another() {
        let mut queue = QuickBuyQueue::default();
        queue.add(ItemId::WRAITH_BAND.0, true, &[], None);
        queue.add(ItemId::WRAITH_BAND.0, true, &[], None);
        queue.sync(&[ItemId::WRAITH_BAND, ItemId::SLIPPERS], None);
        assert_eq!(queue.targets(), &[ItemId::WRAITH_BAND.0]);
        // The ownership pool may be reordered as inventory moves between carriers.
        queue.sync(&[ItemId::SLIPPERS, ItemId::WRAITH_BAND], None);
        assert_eq!(queue.targets(), &[ItemId::WRAITH_BAND.0]);
        assert_eq!(queue.parts().len(), 2);
    }
    #[test]
    fn queued_copies_reserve_components_once_and_complete_individually() {
        let held = [
            ItemId::SLIPPERS,
            ItemId::SLIPPERS,
            ItemId::SLIPPERS,
            ItemId::CIRCLET,
        ];
        let mut queue = QuickBuyQueue::default();
        for _ in 0..3 {
            queue.add(ItemId::WRAITH_BAND.0, true, &held, Some(Attribute::Agility));
        }
        assert_eq!(
            queue
                .parts()
                .iter()
                .filter(|id| **id == ItemId::RECIPE_WRAITH_BAND.0)
                .count(),
            3
        );
        assert_eq!(
            queue
                .parts()
                .iter()
                .filter(|id| **id == ItemId::CIRCLET.0)
                .count(),
            2
        );
        queue.sync(
            &[ItemId::WRAITH_BAND, ItemId::SLIPPERS, ItemId::SLIPPERS],
            Some(Attribute::Agility),
        );
        assert_eq!(queue.targets().len(), 2);
        assert_eq!(
            queue
                .parts()
                .iter()
                .filter(|id| **id == ItemId::RECIPE_WRAITH_BAND.0)
                .count(),
            2
        );
    }
    #[test]
    fn a_purchase_merging_into_an_existing_stack_completes_only_one_requested_copy() {
        let held = [ItemId::TANGO];
        let mut queue = QuickBuyQueue::default();
        queue.add(ItemId::TANGO.0, true, &held, None);
        queue.add(ItemId::TANGO.0, true, &held, None);
        queue.bought(ItemId::TANGO);
        queue.sync(&held, None);
        assert_eq!(queue.targets().len(), 1);
        queue.bought(ItemId::TANGO);
        queue.sync(&held, None);
        assert!(queue.is_empty());
    }
}
