//! Where bota's numbers are not the install's: the game bota plays is Dota 7.41f, and an
//! install moves on with Valve's patches, or holds a number the game itself does not play by.
//! Each override names its evidence. One the install already agrees with is refused, so the
//! table never outlives its reason.

use crate::dec;
use crate::item::Item;

/// The patch bota plays.
pub const PATCH: &str = "7.41f";

/// What an override sets.
#[derive(Clone, Copy, Debug)]
pub enum Field {
    /// `AbilityCooldown`, in seconds.
    Cooldown,
}

/// One number bota plays by instead of the install's.
pub struct Override {
    pub item: &'static str,
    pub field: Field,
    /// As a script writes it.
    pub value: &'static str,
    /// Where the number comes from: a Liquipedia page and what it says, or patch notes.
    pub evidence: &'static str,
}

/// Every override.
pub const OVERRIDES: &[Override] = &[Override {
    item: "item_ward_dispenser",
    field: Field::Cooldown,
    value: "1.0",
    evidence: "Liquipedia, Observer and Sentry Wards, changelog (2017): \"Fixed Plant cooldown \
               not working\"; the dispenser waits the one second the wards wait between plantings \
               (items.txt gives it none)",
}];

/// `item` with the overrides that name it applied.
pub fn apply(item: &mut Item) -> Result<(), String> {
    for o in OVERRIDES.iter().filter(|o| o.item == item.key) {
        let fail = |why: String| format!("override of {} {:?}: {why}", o.item, o.field);
        let changed = match o.field {
            Field::Cooldown => replace(&mut item.cooldown, dec::milli(o.value).map_err(fail)?),
        };
        if !changed {
            return Err(fail("the install agrees already: drop the override".into()));
        }
    }
    Ok(())
}

/// Sets `slot` to `value`; whether that changed it.
fn replace<T: PartialEq>(slot: &mut T, value: T) -> bool {
    let changed = *slot != value;
    *slot = value;
    changed
}
