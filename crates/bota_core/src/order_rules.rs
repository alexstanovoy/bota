//! Order checks on exact facts, shared by world validation and view readers.

use crate::AbilityDef;
use bota_proto::{Aim, RejectReason, Target};

/// Exact conditions that can bar an ability before its target or mana is checked.
#[derive(Clone, Copy, Debug, Default)]
pub struct CastConditions {
    pub illusion: bool,
    pub level: u8,
    pub cooldown: u32,
    pub held: bool,
    pub feared: bool,
    pub rooted: bool,
}

/// The first refusal of an ability, independent of its target and mana.
/// Silence affects execution, rather than admission, and is deliberately not an input.
pub fn cast_refusal(def: &AbilityDef, facts: CastConditions) -> Option<RejectReason> {
    if facts.illusion {
        return Some(RejectReason::NotCastable);
    }
    if let Some(reason) = ability_resource_refusal(def.passive, facts.level, facts.cooldown) {
        return Some(reason);
    }
    // A channel is no bar to admission: a new order can interrupt it.
    // Roshan's roar can be cast while stunned or feared; its data carries that exception.
    if !def.cast_while_stunned && (facts.held || facts.feared) {
        return Some(RejectReason::Disabled);
    }
    if def.root_disables && facts.rooted {
        return Some(RejectReason::Disabled);
    }
    None
}

/// Whether an order has the target shape its definition takes. Trees are positions;
/// building uses additionally resolve a destination in the world.
#[must_use]
pub const fn aimed_right(aim: Aim, target: &Target) -> bool {
    match aim {
        Aim::Own => matches!(target, Target::None),
        Aim::Point | Aim::Tree => matches!(target, Target::Pos(_)),
        Aim::Building => true,
        Aim::Unit => matches!(target, Target::Unit(_)),
    }
}

/// Whether a carried item can be used at all, before its cooldown or charges.
pub fn item_usability(aim: Option<Aim>, mute: u32, illusion: bool) -> Result<Aim, RejectReason> {
    if mute > 0 {
        return Err(RejectReason::NotReady);
    }
    let aim = aim.ok_or(RejectReason::NotCastable)?;
    if illusion {
        return Err(RejectReason::NotCastable);
    }
    Ok(aim)
}

/// Cooldown and charge admission, including giving items and releasing stored runes.
pub fn item_readiness(
    cooldown: bool,
    gives: bool,
    needs_charge: bool,
    charges: u8,
    rune: bool,
) -> Result<(), RejectReason> {
    if !gives && cooldown {
        return Err(RejectReason::OnCooldown);
    }
    if needs_charge && charges == 0 && !rune {
        return Err(RejectReason::NoCharges);
    }
    Ok(())
}

/// Facts about a stack relevant to inventory admission. Views omit `free_charges`;
/// a view reader must retain that uncertainty after checking the other constraints.
#[derive(Clone, Copy, Debug)]
pub struct OrderItemFacts {
    pub id: bota_proto::ItemId,
    pub owner: bota_proto::SlotId,
    pub rune: bool,
    pub free_charges: u8,
}

pub const BAG_SLOTS: usize = crate::rules::INVENTORY_SLOTS + crate::rules::BACKPACK_SLOTS;
const _: () = assert!(BAG_SLOTS + crate::rules::STASH_SLOTS == crate::rules::TOWN_PORTAL_SLOT);

pub const fn in_inventory(slot: usize) -> bool {
    slot < crate::rules::INVENTORY_SLOTS
}
pub const fn in_backpack(slot: usize) -> bool {
    slot >= crate::rules::INVENTORY_SLOTS && slot < BAG_SLOTS
}
pub const fn in_stash(slot: usize) -> bool {
    slot >= BAG_SLOTS && slot < BAG_SLOTS + crate::rules::STASH_SLOTS
}
pub const fn in_town_portal(slot: usize) -> bool {
    slot == crate::rules::TOWN_PORTAL_SLOT
}

/// Inventory copies cannot transact their carried stacks; their owner's stash is physical.
pub fn physical_item_refusal(at: usize, copied: bool) -> Result<(), RejectReason> {
    if !in_stash(at) && copied {
        Err(RejectReason::ItemRestricted)
    } else {
        Ok(())
    }
}

pub fn owned_item(
    at: usize,
    copied: bool,
    held: Option<OrderItemFacts>,
    owner: bota_proto::SlotId,
) -> Result<OrderItemFacts, RejectReason> {
    physical_item_refusal(at, copied)?;
    let held = held.ok_or(RejectReason::EmptySlot)?;
    if held.owner != owner {
        return Err(RejectReason::NotYourItem);
    }
    Ok(held)
}

pub fn sale_refusal(
    at: usize,
    copied: bool,
    held: Option<OrderItemFacts>,
    owner: bota_proto::SlotId,
    at_shop: bool,
) -> Result<(), RejectReason> {
    let held = owned_item(at, copied, held, owner)?;
    if crate::item_def(held.id).is_some_and(|def| !def.limits.sellable) || held.free_charges > 0 {
        return Err(RejectReason::Unsellable);
    }
    if held.rune {
        return Err(RejectReason::HoldsRune);
    }
    if in_town_portal(at) && !at_shop {
        return Err(RejectReason::NotAtShop);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct SwapConditions {
    pub copied: bool,
    pub stash_slots: usize,
    pub stash_access: bool,
    pub has_portal: bool,
}

pub fn swap_refusal(
    from: usize,
    to: usize,
    source: Option<OrderItemFacts>,
    target: Option<OrderItemFacts>,
    facts: SwapConditions,
) -> Result<(), RejectReason> {
    if !(in_stash(from) && in_stash(to)) && facts.copied {
        return Err(RejectReason::ItemRestricted);
    }
    let total = BAG_SLOTS.saturating_add(facts.stash_slots);
    let named = |at| at < total || in_town_portal(at);
    if from == to || !named(from) || !named(to) {
        return Err(RejectReason::EmptySlot);
    }
    let source = source.ok_or(RejectReason::EmptySlot)?;
    if (in_stash(from) || in_stash(to)) && !facts.stash_access {
        return Err(RejectReason::NotAtShop);
    }
    let leaving = [(Some(source), to), (target, from)];
    if leaving
        .iter()
        .any(|(held, at)| held.is_some_and(|held| held.rune) && !in_inventory(*at))
    {
        return Err(RejectReason::HoldsRune);
    }
    if leaving.iter().any(|(held, at)| {
        held.and_then(|held| crate::item_def(held.id))
            .is_some_and(|def| !def.limits.backpack && !in_inventory(*at))
    }) {
        return Err(RejectReason::ItemRestricted);
    }
    if in_town_portal(from)
        || (in_town_portal(to)
            && (!facts.has_portal || source.id != bota_proto::ItemId::TOWN_PORTAL_SCROLL))
    {
        return Err(RejectReason::WrongTargetKind);
    }
    Ok(())
}

pub fn put_item_refusal(
    at: usize,
    copied: bool,
    held: Option<OrderItemFacts>,
) -> Result<(), RejectReason> {
    if copied {
        return Err(RejectReason::ItemRestricted);
    }
    if at >= BAG_SLOTS {
        return Err(RejectReason::NotInBag);
    }
    let held = held.ok_or(RejectReason::EmptySlot)?;
    if crate::item_def(held.id).is_some_and(|def| !def.limits.droppable) {
        return Err(RejectReason::ItemRestricted);
    }
    if held.rune {
        return Err(RejectReason::HoldsRune);
    }
    Ok(())
}

pub fn put_recipient_refusal(
    same_unit: bool,
    allied: bool,
    receives: bool,
) -> Result<(), RejectReason> {
    if same_unit || !allied || !receives {
        Err(RejectReason::WrongTargetKind)
    } else {
        Ok(())
    }
}

pub fn may_receive_item(physical_inventory: bool, real_hero: bool, id: bota_proto::ItemId) -> bool {
    physical_inventory
        && crate::item_def(id).is_some_and(|def| match def.pickup {
            crate::PickupEligibility::InventoryCarrier => true,
            crate::PickupEligibility::RealHero => real_hero,
        })
}

pub fn take_refusal(rune: bool, real_hero: bool, receives_loot: bool) -> Result<(), RejectReason> {
    let takes = if rune { real_hero } else { receives_loot };
    if takes {
        Ok(())
    } else {
        Err(RejectReason::WrongTargetKind)
    }
}

pub fn switch_refusal(at: usize, held: Option<bota_proto::ItemId>) -> Result<(), RejectReason> {
    if in_stash(at) {
        return Err(RejectReason::NotInBag);
    }
    if held.ok_or(RejectReason::EmptySlot)? != bota_proto::ItemId::WARD_DISPENSER {
        return Err(RejectReason::NotCastable);
    }
    Ok(())
}

pub fn disassembly_refusal(
    stash: bool,
    seat_dead: bool,
    at_base: bool,
    eligible: bool,
) -> Result<(), RejectReason> {
    if stash && !seat_dead && !at_base {
        return Err(RejectReason::NotAtShop);
    }
    if !eligible {
        return Err(RejectReason::NotDisassemblable);
    }
    Ok(())
}

pub fn lock_refusal(id: bota_proto::ItemId) -> Result<(), RejectReason> {
    if crate::is_part_of_a_recipe(id) {
        Ok(())
    } else {
        Err(RejectReason::CannotLockCombining)
    }
}

pub fn glyph_refusal(team: bota_proto::Team, cooldown_left: u32) -> Result<(), RejectReason> {
    crate::side_index(team).ok_or(RejectReason::NotYourUnit)?;
    if cooldown_left > 0 {
        Err(RejectReason::OnCooldown)
    } else {
        Ok(())
    }
}

/// The world and views already publish talent eligibility; admission consumes that fact.
pub fn learning_refusal(eligible: bool) -> Result<(), RejectReason> {
    if eligible {
        Ok(())
    } else {
        Err(RejectReason::CannotLearn)
    }
}

pub fn ability_target_kind(
    def: &AbilityDef,
    kind: Option<bota_proto::UnitKind>,
    allied: bool,
) -> bool {
    kind.map_or_else(
        || def.targets.any_kind(),
        |kind| def.targets.takes(kind, allied),
    )
}

/// Static cheat constraints, before world-only target, landing and seat-capacity queries.
pub fn cheat_refusal(
    enabled: bool,
    cheat: bota_proto::Cheat,
    catalogue: crate::Catalogue,
    has_rune_spots: bool,
) -> Result<(), RejectReason> {
    use bota_proto::Cheat;
    cheat_access(enabled)?;
    let valid = match cheat {
        Cheat::ApplyModifier { spec, ticks, .. } => {
            spec.is_bounded() && bota_proto::modifier_ticks_bounded(ticks)
        }
        Cheat::Item { item } => catalogue.item(item).is_some(),
        Cheat::CreateHero { hero, .. } => crate::hero_def(hero).is_some(),
        Cheat::SpawnRune { .. } => has_rune_spots,
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(RejectReason::BadCheat)
    }
}

pub fn ability_resource_refusal(passive: bool, level: u8, cooldown: u32) -> Option<RejectReason> {
    if passive {
        Some(RejectReason::NotCastable)
    } else if level == 0 {
        Some(RejectReason::NotLearned)
    } else if cooldown > 0 {
        Some(RejectReason::OnCooldown)
    } else {
        None
    }
}

/// The shared, visible prerequisites of hostility. Sight and neutral-camp rules
/// additionally depend on world facts which a filtered view may not expose.
pub fn hostile_body(
    mine: Option<bota_proto::Team>,
    theirs: Option<bota_proto::Team>,
    alive: bool,
    invulnerable: bool,
) -> bool {
    mine.zip(theirs)
        .is_some_and(|(mine, theirs)| mine != theirs)
        && alive
        && !invulnerable
}

pub fn cheat_access(enabled: bool) -> Result<(), RejectReason> {
    if enabled {
        Ok(())
    } else {
        Err(RejectReason::NoCheats)
    }
}
