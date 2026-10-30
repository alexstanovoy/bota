//! Items as Dota's `scripts/npc/items.txt` gives them, in its own units: the shape of the
//! generated table `data/items.rs`, and the one conversion of its numbers into bota's.

use bota_proto::ItemId;

use crate::rules;

/// A number as Dota's scripts write it, in thousandths: `Dec::of("0.25")` is 250.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dec(pub i32);

impl Dec {
    /// The number a script writes as `text`: an optional minus, digits, and at most three
    /// decimal places. Anything else fails the build.
    pub const fn of(text: &str) -> Dec {
        let bytes = text.as_bytes();
        let negative = !bytes.is_empty() && bytes[0] == b'-';
        let mut at = if negative { 1 } else { 0 };
        assert!(at < bytes.len(), "an empty number");
        let mut value: i32 = 0;
        let mut places: i32 = -1;
        while at < bytes.len() {
            let b = bytes[at];
            if b == b'.' {
                assert!(places < 0, "two decimal points");
                places = 0;
            } else {
                assert!(b.is_ascii_digit(), "not a digit");
                value = value * 10 + (b - b'0') as i32;
                if places >= 0 {
                    places += 1;
                }
            }
            at += 1;
        }
        assert!(places != 0, "a decimal point without places");
        assert!(places <= 3, "more than three decimal places");
        let mut scale = if places < 0 { 0 } else { places };
        while scale < 3 {
            value *= 10;
            scale += 1;
        }
        Dec(if negative { -value } else { value })
    }

    /// The whole number it is. Fails the build for one with a fraction.
    pub const fn int(self) -> i32 {
        assert!(self.0 % 1000 == 0, "not a whole number");
        self.0 / 1000
    }

    /// The number in hundredths. Fails the build for one finer than that.
    pub const fn hundredths(self) -> i32 {
        assert!(self.0 % 10 == 0, "finer than hundredths");
        self.0 / 10
    }

    /// The number as seconds.
    pub const fn secs(self) -> Ms {
        assert!(self.0 >= 0, "a negative span");
        Ms(self.0 as u32)
    }
}

/// A span of game time in milliseconds: Dota's seconds, kept exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ms(pub u32);

impl Ms {
    /// The span a script writes as `text` seconds.
    pub const fn secs(text: &str) -> Ms {
        Dec::of(text).secs()
    }

    /// The span in ticks, to the nearest tick (a half rounds up): the one way a span of
    /// Dota's becomes the server's.
    pub const fn ticks(self) -> u32 {
        ((self.0 as u64 * rules::TICKS_PER_SECOND as u64 + 500) / 1000) as u32
    }
}

/// Dota's `AbilityBehavior` flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Behavior(u32);

impl Behavior {
    pub const NONE: Behavior = Behavior(0);
    pub const AOE: Behavior = Behavior(1 << 0);
    pub const CHANNELLED: Behavior = Behavior(1 << 1);
    pub const DIRECTIONAL: Behavior = Behavior(1 << 2);
    pub const DONT_CANCEL_CHANNEL: Behavior = Behavior(1 << 3);
    pub const DONT_PROC_OTHER_ABILITIES: Behavior = Behavior(1 << 4);
    pub const DONT_RESUME_ATTACK: Behavior = Behavior(1 << 5);
    pub const IGNORE_BACKSWING: Behavior = Behavior(1 << 6);
    pub const IGNORE_CHANNEL: Behavior = Behavior(1 << 7);
    pub const IMMEDIATE: Behavior = Behavior(1 << 8);
    pub const NO_TARGET: Behavior = Behavior(1 << 9);
    pub const OPTIONAL_UNIT_TARGET: Behavior = Behavior(1 << 10);
    /// Aimed past its cast range, it goes off without walking in: Blink Dagger's.
    pub const OVERSHOOT: Behavior = Behavior(1 << 11);
    pub const PASSIVE: Behavior = Behavior(1 << 12);
    pub const POINT: Behavior = Behavior(1 << 13);
    pub const ROOT_DISABLES: Behavior = Behavior(1 << 14);
    pub const SUPPRESS_ASSOCIATED_CONSUMABLE: Behavior = Behavior(1 << 15);
    pub const UNIT_TARGET: Behavior = Behavior(1 << 16);

    /// Both sets of flags.
    pub const fn and(self, other: Behavior) -> Behavior {
        Behavior(self.0 | other.0)
    }

    /// Whether every flag of `flags` is set.
    pub const fn has(self, flags: Behavior) -> bool {
        self.0 & flags.0 == flags.0
    }
}

/// How a shop restocks an item, as the script gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StockData {
    /// `ItemStockInitial`.
    pub initial: u8,
    /// `ItemStockMax`.
    pub max: u8,
    /// `ItemStockTime`: the wait for one more.
    pub every: Ms,
}

/// One item as the script has it: numbers and Dota's internal names, nothing to show.
#[derive(Clone, Copy, Debug)]
pub struct ItemData {
    /// Its id, its line of the id registry.
    pub id: ItemId,
    /// Its Dota key.
    pub key: &'static str,
    /// Whether the registry has retired it.
    pub retired: bool,
    /// `ItemCost`.
    pub cost: i32,
    /// `AbilityBehavior`.
    pub behavior: Behavior,
    /// `AbilityCastRange`, in world units.
    pub cast_range: i32,
    /// `AbilityCastPoint`.
    pub cast_point: Ms,
    /// `AbilityChannelTime`.
    pub channel: Ms,
    /// `AbilityCooldown`.
    pub cooldown: Ms,
    /// `AbilityManaCost`.
    pub mana: i32,
    /// `AbilityHealthCost`.
    pub health: i32,
    /// `ItemInitialCharges`.
    pub charges: u8,
    /// How many charges one stack holds: `ItemStackableMax`, every one a byte holds for a
    /// stackable item without it, none for one that does not stack.
    pub stack: u8,
    /// `ItemStockInitial`, `ItemStockMax` and `ItemStockTime`, for an item sold from a stock.
    pub stock: Option<StockData>,
    /// `ItemPurchasable`.
    pub purchasable: bool,
    /// The ways it is built, each the parts a recipe asks for and the recipe scroll where that
    /// costs gold. Empty for one bought whole.
    pub builds: &'static [&'static [ItemId]],
    /// `AbilityValues`: each a number a level, by Dota's key.
    pub values: &'static [(&'static str, &'static [Dec])],
}

impl ItemData {
    /// The value of `key`, one number a level. Fails the build when the item has none.
    pub const fn value(&self, key: &str) -> &'static [Dec] {
        let mut at = 0;
        while at < self.values.len() {
            if str_eq(self.values[at].0, key) {
                return self.values[at].1;
            }
            at += 1;
        }
        panic!("the item has no such value");
    }

    /// The first level of the value of `key`.
    pub const fn first(&self, key: &str) -> Dec {
        self.value(key)[0]
    }
}

/// Whether two strings are the same, in a const context.
pub(crate) const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut at = 0;
    while at < a.len() {
        if a[at] != b[at] {
            return false;
        }
        at += 1;
    }
    true
}
