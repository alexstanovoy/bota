//! The shop as a player uses it: what a seat holds towards a build, the
//! groups and the search of the catalog, and the quick-buy item.

use bota_proto::ItemId;

use crate::catalog::{ITEMS, ItemFace};
use bota_core::missing_parts;

use crate::shop::{held_items, price_for, primary_of, whole_price};
use crate::ui::app::{App, Phase};

fn hero_primary(hero: bota_proto::HeroId) -> Option<bota_proto::Attribute> {
    bota_core::hero_def(hero).and_then(|def| def.unit.primary)
}

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

    /// Chooses an item to buy once the match begins, if the starting gold
    /// still covers it after the others.
    pub fn queue_buy(&mut self, item: u16) {
        if self.price_of(item).0 > self.gold_after_queue() {
            self.reject = Some((
                crate::refusal(bota_proto::RejectReason::NotEnoughGold).to_string(),
                2.5,
            ));
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
        let gold = self.player(self.my_slot?)?.gold.unwrap_or(0);
        if gold >= self.price_of(target).0 {
            return Some(target);
        }
        self.missing_for(target)
            .into_iter()
            .find(|part| whole_price(*part) <= gold)
            .map(|part| part.0)
    }
}
