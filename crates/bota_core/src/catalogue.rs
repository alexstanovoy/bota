//! Which items a match plays with. Item ids only grow, so a build that holds
//! more items still meets one that holds fewer: the two play on the first
//! ids both hold, when they agree on what those items are. What an item is
//! (every number and rule a match plays it by) is its digest; a prefix of the
//! catalogue is told by its length and the digest of its items.

use bota_proto::{Aim, Attribute, Attributes, CataloguePrefix, Fixed, ItemId};

use crate::{
    AllyUse, Carried, DisassembleRule, ITEM_COUNT, ITEMS, ItemDef, Mend, Odds, Pool, Recipe,
    Spends, Stock, UseSpec, item_def, sold, sold_items,
};

/// The items a match plays with: the first so many ids of this build's
/// catalogue. Only ever one this build holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Catalogue {
    items: u16,
}

impl Catalogue {
    /// Every item this build holds.
    pub const FULL: Catalogue = Catalogue {
        items: ITEM_COUNT as u16,
    };

    /// The first `items` ids of this build's catalogue; none past its end.
    pub const fn first(items: u16) -> Option<Catalogue> {
        if items as usize <= ITEM_COUNT {
            Some(Catalogue { items })
        } else {
            None
        }
    }

    /// The catalogue a peer told: this build's prefix of the same length,
    /// when the two agree on every item in it. None when the peer holds
    /// items this build does not, or one of the items differs.
    pub fn agreeing(told: CataloguePrefix) -> Option<Catalogue> {
        let catalogue = Catalogue::first(told.items)?;
        (catalogue.prefix() == told).then_some(catalogue)
    }

    /// How many item ids it holds.
    pub const fn items(self) -> u16 {
        self.items
    }

    /// Whether it holds `item`.
    pub const fn holds(self, item: ItemId) -> bool {
        item.0 < self.items
    }

    /// The shorter of two catalogues: the one both hold.
    pub const fn shared(self, other: Catalogue) -> Catalogue {
        if other.items < self.items {
            other
        } else {
            self
        }
    }

    /// As the wire tells it.
    pub const fn prefix(self) -> CataloguePrefix {
        CataloguePrefix {
            items: self.items,
            digest: PREFIX_DIGESTS[self.items as usize],
        }
    }

    /// What it holds under `item`, unless the item is retired.
    pub fn item(self, item: ItemId) -> Option<&'static ItemDef> {
        item_def(item).filter(|_| self.holds(item))
    }

    /// What a shop of a match on it sells under `item`, as [`sold`] says.
    pub fn sold(self, item: ItemId) -> Option<&'static ItemDef> {
        sold(item).filter(|_| self.holds(item))
    }

    /// Every item a shop of a match on it sells, in id order.
    pub fn sold_items(self) -> impl Iterator<Item = ItemId> {
        sold_items().filter(move |item| self.holds(*item))
    }
}

/// The digest of every prefix of the catalogue, by its length: each the
/// digest of the one before it continued by its last item's.
const PREFIX_DIGESTS: [u64; ITEM_COUNT + 1] = {
    let mut digests = [0; ITEM_COUNT + 1];
    digests[0] = Fnv::NEW.0;
    let mut at = 0;
    while at < ITEM_COUNT {
        digests[at + 1] = Fnv(digests[at]).u64(item_digest(&ITEMS[at])).0;
        at += 1;
    }
    digests
};

/// FNV-1a, written to by value so it runs at compile time.
#[derive(Clone, Copy)]
struct Fnv(u64);

impl Fnv {
    const NEW: Fnv = Fnv(0xcbf2_9ce4_8422_2325);

    const fn bytes(self, bytes: &[u8]) -> Fnv {
        let mut hash = self.0;
        let mut at = 0;
        while at < bytes.len() {
            hash ^= bytes[at] as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
            at += 1;
        }
        Fnv(hash)
    }

    const fn u64(self, value: u64) -> Fnv {
        self.bytes(&value.to_le_bytes())
    }

    const fn int(self, value: i64) -> Fnv {
        self.u64(value as u64)
    }

    const fn flag(self, value: bool) -> Fnv {
        self.u64(value as u64)
    }

    /// A string, closed by its length so two strings never run together.
    const fn text(self, text: &str) -> Fnv {
        self.bytes(text.as_bytes()).u64(text.len() as u64)
    }

    const fn fixed(self, value: Fixed) -> Fnv {
        self.int(value.raw as i64)
    }

    const fn attributes(self, value: Attributes) -> Fnv {
        self.fixed(value.strength)
            .fixed(value.agility)
            .fixed(value.intelligence)
    }

    const fn odds(self, value: Odds) -> Fnv {
        self.u64(value.get() as u64)
    }

    const fn attribute(self, value: Option<Attribute>) -> Fnv {
        match value {
            None => self.u64(0),
            Some(which) => self.u64(1 + which as u64),
        }
    }

    const fn ally(self, value: Option<AllyUse>) -> Fnv {
        match value {
            None => self.u64(0),
            Some(AllyUse::Effect { percent }) => self.u64(1).int(percent as i64),
            Some(AllyUse::Give { item: None }) => self.u64(2),
            Some(AllyUse::Give { item: Some(item) }) => self.u64(3).u64(item.0 as u64),
        }
    }

    const fn aim(self, value: Option<Aim>) -> Fnv {
        match value {
            None => self.u64(0),
            Some(aim) => self.u64(1 + aim as u64),
        }
    }

    const fn ids(self, ids: &[ItemId]) -> Fnv {
        let mut fnv = self.u64(ids.len() as u64);
        let mut at = 0;
        while at < ids.len() {
            fnv = fnv.u64(ids[at].0 as u64);
            at += 1;
        }
        fnv
    }

    const fn recipes(self, recipes: &[Recipe<'_>]) -> Fnv {
        let mut fnv = self.u64(recipes.len() as u64);
        let mut at = 0;
        while at < recipes.len() {
            fnv = fnv.ids(recipes[at].parts).attribute(recipes[at].mode);
            at += 1;
        }
        fnv
    }

    const fn stock(self, value: Option<Stock>) -> Fnv {
        match value {
            None => self.u64(0),
            Some(stock) => self
                .u64(1)
                .u64(stock.initial as u64)
                .u64(stock.max as u64)
                .u64(stock.restock_ticks as u64),
        }
    }

    const fn carried(self, value: Carried) -> Fnv {
        let Carried {
            attributes,
            move_speed,
            lace_speed,
            damage,
            attack_speed,
            armor,
            hp,
            mana,
            hp_regen,
            mana_regen,
            damage_to_creeps,
            base_attack_speed_pct,
            evasion,
            pierce,
            pierce_damage,
            pierce_unique,
            melee_range,
            mana_pct,
        } = value;
        self.attributes(attributes)
            .int(move_speed as i64)
            .int(lace_speed as i64)
            .int(damage as i64)
            .int(attack_speed as i64)
            .fixed(armor)
            .int(hp as i64)
            .int(mana as i64)
            .fixed(hp_regen)
            .fixed(mana_regen)
            .int(damage_to_creeps as i64)
            .int(base_attack_speed_pct as i64)
            .odds(evasion)
            .odds(pierce)
            .int(pierce_damage as i64)
            .flag(pierce_unique)
            .int(melee_range as i64)
            .int(mana_pct as i64)
    }

    const fn mend(self, value: Mend) -> Fnv {
        let Mend {
            pool,
            total,
            ticks,
            eats_a_tree,
            breaks,
        } = value;
        let pool = match pool {
            Pool::Health => 0,
            Pool::Mana => 1,
        };
        self.u64(pool)
            .int(total as i64)
            .u64(ticks as u64)
            .flag(eats_a_tree)
            .flag(breaks)
    }

    /// What a use does, each kind under a tag of its own.
    const fn effect(self, value: UseSpec) -> Fnv {
        match value {
            UseSpec::None => self.u64(0),
            UseSpec::Mend(mend) => self.u64(1).mend(mend),
            UseSpec::PlantTree { ticks } => self.u64(2).u64(ticks as u64),
            UseSpec::FellTree => self.u64(3),
            UseSpec::Ward { unit, ticks } => self.u64(4).u64(unit.kind as u64).u64(ticks as u64),
            UseSpec::Dispense => self.u64(5),
            UseSpec::Teleport(scroll) => self
                .u64(6)
                .int(scroll.min_distance as i64)
                .int(scroll.max_distance as i64)
                .int(scroll.sight_radius as i64),
            UseSpec::SwitchMode => self.u64(7),
            UseSpec::Phase {
                pct,
                ranged_pct,
                ticks,
            } => self
                .u64(8)
                .int(pct as i64)
                .int(ranged_pct as i64)
                .u64(ticks as u64),
            UseSpec::Blink { range, overshoot } => {
                self.u64(9).int(range as i64).int(overshoot as i64)
            }
            UseSpec::Restore {
                hp_per_charge,
                mana_per_charge,
                charge_radius,
            } => self
                .u64(10)
                .int(hp_per_charge as i64)
                .int(mana_per_charge as i64)
                .int(charge_radius as i64),
            UseSpec::Replenish { mana } => self.u64(11).int(mana as i64),
            UseSpec::Grenade(grenade) => self
                .u64(12)
                .int(grenade.speed as i64)
                .int(grenade.radius as i64)
                .int(grenade.impact_damage as i64)
                .int(grenade.damage_per_second as i64)
                .int(grenade.slow_pct as i64)
                .u64(grenade.debuff_ticks as u64),
            UseSpec::Eat { health, mana } => self.u64(14).int(health as i64).int(mana as i64),
            UseSpec::Bottle(drink) => self
                .u64(13)
                .int(drink.health as i64)
                .int(drink.mana as i64)
                .u64(drink.ticks as u64)
                .int(drink.range as i64),
        }
    }

    const fn values(self, values: &[(&str, &[crate::Dec])]) -> Fnv {
        let mut fnv = self.u64(values.len() as u64);
        let mut at = 0;
        while at < values.len() {
            fnv = fnv.text(values[at].0).u64(values[at].1.len() as u64);
            let mut level = 0;
            while level < values[at].1.len() {
                fnv = fnv.int(values[at].1[level].0 as i64);
                level += 1;
            }
            at += 1;
        }
        fnv
    }
}

/// What an item is to a match: every field of its row but the words that
/// name it. Taking the row apart field by field makes a field added to
/// [`ItemDef`] fail to build here until it is digested too.
const fn item_digest(def: &ItemDef) -> u64 {
    let ItemDef {
        id,
        key,
        name: _,
        cost,
        charges,
        stack_limit,
        cast_charges,
        cooldown,
        shared_wait,
        mana_cost,
        health_cost,
        stock,
        breaks_on_damage,
        mode,
        mode_bonus,
        components,
        alternatives,
        carried,
        ranged,
        retired,
        aim,
        cast_range,
        behavior,
        values,
        spends,
        point,
        backswing,
        duration,
        mana_deficit,
        immediate,
        ignores_channel,
        unbuyable,
        kept_empty,
        ally_target,
        wait_key,
        charge_recipes,
        take_range,
        effect,
        ally,
        lifetime,
        disassemble,
    } = *def;
    let fnv = Fnv::NEW
        .u64(id.0 as u64)
        .text(key)
        .int(cost as i64)
        .u64(charges as u64)
        .u64(stack_limit as u64)
        .u64(cast_charges as u64)
        .u64(cooldown as u64)
        .flag(shared_wait)
        .int(mana_cost as i64)
        .int(health_cost as i64)
        .stock(stock)
        .u64(breaks_on_damage as u64)
        .attribute(mode)
        .int(mode_bonus as i64)
        .ids(components)
        .recipes(alternatives)
        .carried(carried);
    let fnv = fnv.u64(match disassemble {
        DisassembleRule::Window => 0,
        DisassembleRule::Always => 1,
        DisassembleRule::Never => 2,
    });
    let fnv = match ranged {
        None => fnv.u64(0),
        Some(ranged) => fnv.u64(1).carried(ranged),
    };
    let spends = match spends {
        Spends::Nothing => 0,
        Spends::One => 1,
        Spends::All => 2,
    };
    fnv.flag(retired)
        .aim(aim)
        .int(cast_range as i64)
        .u64(behavior.bits() as u64)
        .values(values)
        .u64(spends)
        .u64(point as u64)
        .u64(backswing as u64)
        .u64(duration as u64)
        .flag(mana_deficit)
        .flag(immediate)
        .flag(ignores_channel)
        .flag(unbuyable)
        .flag(kept_empty)
        .flag(ally_target)
        .effect(effect)
        .ally(ally)
        .u64(lifetime as u64);
    // What only a few items have adds to the digest only where it is: an item
    // without it digests as it did before the field was.
    let fnv = match wait_key {
        None => fnv,
        Some(key) => fnv.u64(0xA1).u64(key.0 as u64),
    };
    let fnv = if take_range == 0 {
        fnv
    } else {
        fnv.u64(0xA2).int(take_range as i64)
    };
    let mut fnv = fnv;
    let mut at = 0;
    while at < charge_recipes.len() {
        fnv = fnv.u64(0xA3).ids(charge_recipes[at]);
        at += 1;
    }
    fnv.0
}
