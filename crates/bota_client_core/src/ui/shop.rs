//! The shop as a player uses it: what a seat holds towards a build, the
//! groups and the search of the catalog, and the quick-buy item.

use bota_proto::ItemId;

use crate::catalog::{ITEMS, ItemFace};
use crate::shop::{held_items, missing_parts, price_for, primary_of, whole_price};
use crate::ui::app::App;

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
/// know goes last, and a retired one in none: past the last group.
pub fn group_of(id: u16) -> usize {
    if bota_core::item_def(ItemId(id)).is_none() {
        return SHOP_GROUPS.len();
    }
    match ItemId(id) {
        ItemId::CLARITY
        | ItemId::HEALING_SALVE
        | ItemId::IRON_BRANCH
        | ItemId::OBSERVER_WARD
        | ItemId::SENTRY_WARD
        | ItemId::TANGO
        | ItemId::TOWN_PORTAL_SCROLL
        | ItemId::MANGO
        | ItemId::BLOOD_GRENADE => 0,
        ItemId::CIRCLET
        | ItemId::GAUNTLETS
        | ItemId::SLIPPERS
        | ItemId::MANTLE
        | ItemId::BELT
        | ItemId::BAND
        | ItemId::ROBE
        | ItemId::OGRE_AXE
        | ItemId::BLADE_OF_ALACRITY
        | ItemId::STAFF_OF_WIZARDRY
        | ItemId::EAGLESONG => 1,
        ItemId::GLOVES
        | ItemId::BLADES_OF_ATTACK
        | ItemId::BROADSWORD
        | ItemId::QUARTERSTAFF
        | ItemId::RING_OF_PROTECTION
        | ItemId::CHAINMAIL
        | ItemId::CLAYMORE
        | ItemId::TALISMAN_OF_EVASION
        | ItemId::JAVELIN
        | ItemId::DEMON_EDGE
        | ItemId::BLITZ_KNUCKLES => 2,
        ItemId::BOOTS
        | ItemId::QUELLING_BLADE
        | ItemId::RING_OF_REGEN
        | ItemId::SAGES_MASK
        | ItemId::VITALITY_BOOSTER
        | ItemId::ENERGY_BOOSTER
        | ItemId::MAGIC_STICK
        | ItemId::WIND_LACE => 3,
        ItemId::POWER_TREADS
        | ItemId::PHASE_BOOTS
        | ItemId::BLINK_DAGGER
        | ItemId::BRACER
        | ItemId::WRAITH_BAND
        | ItemId::NULL_TALISMAN
        | ItemId::MAGIC_WAND
        | ItemId::BUTTERFLY
        | ItemId::MONKEY_KING_BAR => 4,
        ItemId::RECIPE_BRACER
        | ItemId::RECIPE_WRAITH_BAND
        | ItemId::RECIPE_NULL_TALISMAN
        | ItemId::RECIPE_MAGIC_WAND
        | ItemId::RECIPE_MONKEY_KING_BAR => 5,
        _ => 6,
    }
}

/// Whether an item answers a search, case aside: by its name, its stats or
/// its blurb. An empty search takes everything.
pub fn answers(face: &ItemFace, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    query.is_empty()
        || [face.name.to_string(), face.stats(), face.blurb()]
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

impl App {
    /// What this seat holds towards a build, and its hero's primary
    /// attribute.
    fn buyer(&self) -> (Vec<ItemId>, Option<bota_proto::Attribute>) {
        self.my_slot
            .zip(self.view())
            .and_then(|(slot, view)| {
                let p = view.players.iter().find(|p| p.slot == slot)?;
                Some((held_items(view, p), primary_of(p)))
            })
            .unwrap_or_default()
    }

    /// What this seat pays for an item now, and what it costs whole.
    pub fn price_of(&self, id: u16) -> (i32, i32) {
        let (held, primary) = self.buyer();
        (
            price_for(ItemId(id), &held, primary),
            whole_price(ItemId(id)),
        )
    }

    /// The parts this seat still lacks for an item, in build order.
    pub fn missing_for(&self, id: u16) -> Vec<ItemId> {
        let (held, primary) = self.buyer();
        missing_parts(ItemId(id), &held, primary)
    }

    /// What the quick-buy intent buys now: the quick-buy item whole when the
    /// gold covers it, else its first missing part the gold covers.
    pub fn quick_buy_next(&self) -> Option<u16> {
        let target = self.quick_buy?;
        let view = self.view()?;
        let p = view.players.iter().find(|p| Some(p.slot) == self.my_slot)?;
        let gold = p.gold.unwrap_or(0);
        if gold >= self.price_of(target).0 {
            return Some(target);
        }
        self.missing_for(target)
            .into_iter()
            .find(|part| whole_price(*part) <= gold)
            .map(|part| part.0)
    }
}
