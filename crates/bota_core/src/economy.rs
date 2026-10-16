//! What a death pays and what may be put out by its own side.

use bota_proto::{Fixed, ItemId, UnitKind};

use crate::{kind_def, rules};

/// Whether one of a side's own units is worn down far enough for that side
/// to put it out: below its kind's
/// [`deny_below_pct`](crate::KindDef::deny_below_pct) of what it can hold.
#[inline]
pub fn deniable(kind: UnitKind, hp: Fixed, max_hp: Fixed) -> bool {
    let Some(share) = kind_def(kind).deny_below_pct else {
        return false;
    };
    i64::from(hp.raw) * 100 < i64::from(max_hp.raw) * i64::from(share)
}

/// How long a seat waits before its hero comes back, by its level.
#[inline]
pub fn respawn_wait(level: u8) -> u32 {
    let at = usize::from(level.max(1) - 1).min(rules::RESPAWN_SECONDS.len() - 1);
    rules::RESPAWN_SECONDS[at] * rules::TICKS_PER_SECOND
}

/// What bringing down a hero pays in experience: a base, a share of what
/// the fallen had earned, and a bonus for the streak its death ends.
#[inline]
pub fn hero_kill_xp(victim_xp: i32, streak: i32, level: i32) -> i32 {
    let streak = streak.min(rules::STREAK_XP_CAP);
    let streak_bonus = if streak < rules::STREAK_XP_FROM {
        0
    } else {
        (5 * streak * streak - 10 * streak + 40) * level / 4
    };
    rules::HERO_KILL_XP_BASE + victim_xp * rules::HERO_KILL_XP_SHARE_PCT / 100 + streak_bonus
}

/// What bringing down a hero on a streak of so many pays.
#[inline]
pub fn hero_bounty(streak: i32) -> i32 {
    rules::HERO_KILL_BOUNTY_BASE
        + rules::HERO_KILL_BOUNTY_PER_STREAK * streak.min(rules::HERO_KILL_STREAK_CAP)
}

/// The experience a lane creep denied by its own side pays the other side.
#[inline]
pub fn denied_xp(bounty_xp: i32) -> i32 {
    bounty_xp * rules::DENIED_XP_PCT / 100
}

/// What buying `item` still takes, as the items to buy, given what the buyer
/// holds that counts: which stacks count (the buyer's own, not for sale) is
/// for the caller to say by passing only those.
///
/// An item without parts is itself the answer, so asking for a second one buys
/// a second one. One that is built answers with the parts it lacks, each of
/// those asked the same question in turn; a part already held is spent on the
/// build and so is bought no second time. `components_of` names what an item
/// is built from, empty for one bought whole.
pub fn missing_parts<'a>(
    item: ItemId,
    held: &[ItemId],
    components_of: impl Fn(ItemId) -> &'a [ItemId],
) -> Vec<ItemId> {
    let mut spare = held.to_vec();
    let mut wanted = Vec::new();
    match components_of(item) {
        [] => wanted.push(item),
        parts => {
            for part in parts {
                parts_beyond(*part, &mut spare, &mut wanted, &components_of);
            }
        }
    }
    wanted
}

/// Lays out what one part still costs, spending `held` as it goes.
fn parts_beyond<'a>(
    item: ItemId,
    held: &mut Vec<ItemId>,
    wanted: &mut Vec<ItemId>,
    components_of: &impl Fn(ItemId) -> &'a [ItemId],
) {
    if let Some(at) = held.iter().position(|id| *id == item) {
        held.remove(at);
        return;
    }
    match components_of(item) {
        [] => wanted.push(item),
        parts => {
            for part in parts {
                parts_beyond(*part, held, wanted, components_of);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bota_proto::ItemId;

    use crate::{ITEMS, missing_parts};

    fn parts_of(item: ItemId) -> &'static [ItemId] {
        ITEMS[usize::from(item.0)].components
    }

    #[test]
    fn a_held_part_is_spent_once_on_the_build() {
        let held = [ItemId::BOOTS];
        assert_eq!(
            missing_parts(ItemId::PHASE_BOOTS, &held, parts_of),
            [
                ItemId::BLADES_OF_ATTACK,
                ItemId::BLADES_OF_ATTACK,
                ItemId::RECIPE_PHASE_BOOTS
            ]
        );
        assert_eq!(
            missing_parts(ItemId::BOOTS, &held, parts_of),
            [ItemId::BOOTS]
        );
    }

    #[test]
    fn only_the_parts_not_held_are_bought() {
        let held = [ItemId::CIRCLET];
        let wanted = missing_parts(ItemId::BRACER, &held, parts_of);
        assert_eq!(wanted, [ItemId::GAUNTLETS, ItemId::RECIPE_BRACER]);
    }
}
