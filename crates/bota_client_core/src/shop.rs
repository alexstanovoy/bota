//! The shop as a player uses it: what a seat holds towards a build, the
//! groups and the search of the catalog, and the quick-buy item.

use bota_proto::{ItemId, PlayerView, WorldView};

use crate::catalog::{ITEMS, ItemFace, missing_parts, price_for, whole_price};
use crate::state::App;

/// The groups the shop shows its items in, in order.
pub const SHOP_GROUPS: [&str; 7] = [
    "Consumables",
    "Attributes",
    "Equipment",
    "Supports",
    "Upgrades",
    "Recipes",
    "Other",
];

/// Which of [`SHOP_GROUPS`] an item is shown in; an item the list does not
/// know goes last.
pub fn group_of(id: u16) -> usize {
    match id {
        1..=4 | 6..=8 | 42 | 53 => 0,
        9..=18 | 43 => 1,
        19..=24 | 44 | 45 | 47..=49 => 2,
        0 | 5 | 25..=28 | 35 | 52 => 3,
        29..=34 | 36 | 46 | 51 => 4,
        37..=41 | 50 => 5,
        _ => 6,
    }
}

/// Whether an item answers a search, case aside: by its name, its stats or
/// its blurb. An empty search takes everything.
pub fn answers(face: &ItemFace, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    query.is_empty()
        || [face.name, face.stats, face.blurb]
            .iter()
            .any(|text| text.to_ascii_lowercase().contains(&query))
}

/// The items a search answers, group by group: each group's index into
/// [`SHOP_GROUPS`] and its items in id order. Groups with none are left out.
pub fn shop_groups(query: &str) -> Vec<(usize, Vec<u16>)> {
    (0..SHOP_GROUPS.len())
        .filter_map(|group| {
            let ids: Vec<u16> = ITEMS
                .iter()
                .filter(|face| group_of(face.id) == group && answers(face, query))
                .map(|face| face.id)
                .collect();
            (!ids.is_empty()).then_some((group, ids))
        })
        .collect()
}

/// The first item a search answers, for buying with one key.
pub fn first_answer(query: &str) -> Option<u16> {
    shop_groups(query)
        .into_iter()
        .find_map(|(_, ids)| ids.first().copied())
}

/// Every item in a seat's hero's bag and in its stash that the seat bought
/// and has not marked for sale, for [`price_for`].
pub fn held_items(view: &WorldView, p: &PlayerView) -> Vec<ItemId> {
    let bag = p
        .unit
        .and_then(|id| view.units.iter().find(|u| u.id == id))
        .map(|u| u.items.as_slice())
        .unwrap_or_default();
    bag.iter()
        .chain(p.stash.iter().flatten())
        .flatten()
        .filter(|item| item.owner == p.slot && !item.for_sale)
        .map(|item| item.id)
        .collect()
}

impl App {
    /// What this seat pays for an item now, and what it costs whole.
    pub fn price_of(&self, id: u16) -> (i32, i32) {
        let held = self
            .my_slot
            .zip(self.view.as_ref())
            .and_then(|(slot, view)| {
                let p = view.players.iter().find(|p| p.slot == slot)?;
                Some(held_items(view, p))
            })
            .unwrap_or_default();
        (
            price_for(&self.shop, ItemId(id), &held),
            whole_price(&self.shop, ItemId(id)),
        )
    }

    /// The parts this seat still lacks for an item, in build order.
    pub fn missing_for(&self, id: u16) -> Vec<ItemId> {
        let held = self
            .my_slot
            .zip(self.view.as_ref())
            .and_then(|(slot, view)| {
                let p = view.players.iter().find(|p| p.slot == slot)?;
                Some(held_items(view, p))
            })
            .unwrap_or_default();
        missing_parts(&self.shop, ItemId(id), &held)
    }

    /// What the quick-buy intent buys now: the quick-buy item whole when the
    /// gold covers it, else its first missing part the gold covers.
    pub fn quick_buy_next(&self) -> Option<u16> {
        let target = self.quick_buy?;
        let view = self.view.as_ref()?;
        let p = view.players.iter().find(|p| Some(p.slot) == self.my_slot)?;
        let gold = p.gold.unwrap_or(0);
        if gold >= self.price_of(target).0 {
            return Some(target);
        }
        self.missing_for(target)
            .into_iter()
            .find(|part| whole_price(&self.shop, *part) <= gold)
            .map(|part| part.0)
    }
}
