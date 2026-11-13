use super::{UnknownReason, Verdict};
use bota_core::{CastConditions, cast_refusal, item_def};
use bota_proto::{AbilityView, EffectId, ItemSlot, ItemView, StatusFlags, UnitView};

/// What whole-point wire mana proves about paying a nonnegative cost.
/// A positive fractional pool is shown as one, so one cannot prove a one-point cost.
pub fn mana_affordability(mana: i32, cost: i32) -> Verdict {
    if cost < 0 {
        return Verdict::Unknown(UnknownReason::MalformedView);
    }
    if mana == 1 && cost == 1 {
        return Verdict::Unknown(UnknownReason::RoundedMana);
    }
    Verdict::from_check(mana >= cost, bota_proto::RejectReason::NotEnoughMana)
}

pub(super) fn illusion(unit: &UnitView) -> bool {
    unit.effects
        .iter()
        .take(bota_proto::limits::MAX_EFFECTS_PER_UNIT)
        .any(|effect| effect.id == EffectId::ILLUSION)
}

fn hidden_control(unit: &UnitView) -> bool {
    unit.abilities
        .iter()
        .take(bota_proto::limits::MAX_ABILITY_SLOTS)
        .filter_map(|ability| bota_core::ability_def(ability.id))
        .any(|def| def.hidden_caster_control())
}

pub(super) fn disabled(unit: &UnitView) -> bool {
    unit.statuses.bits & (StatusFlags::STUNNED | StatusFlags::FEARED) != 0
}

/// Admission readiness of an ability, before checking a target. Read from the caster's
/// own side (or a full view): another side is not told which units are illusions.
/// Root-sensitive abilities remain unknown because the wire does not expose root state.
/// Channeling and silence do not bar admission. This does not promise an effect will occur.
pub fn ability_readiness(unit: &UnitView, ability: &AbilityView) -> Verdict {
    use bota_proto::RejectReason;
    if unit.hp <= 0 {
        return Verdict::Illegal(RejectReason::HeroDead);
    }
    let Some(def) = bota_core::ability_def(ability.id).filter(|def| !def.hidden) else {
        return Verdict::Illegal(RejectReason::EmptySlot);
    };
    if let Some(reason) = cast_refusal(
        def,
        CastConditions {
            illusion: illusion(unit),
            level: ability.level,
            cooldown: ability.cooldown_left,
            held: unit.statuses.bits & StatusFlags::STUNNED != 0,
            feared: unit.statuses.bits & StatusFlags::FEARED != 0,
            rooted: false,
        },
    ) {
        return Verdict::Illegal(reason);
    }
    let mana = ability_resources(ability, unit.mana);
    if def.root_disables && mana.is_legal() {
        return Verdict::Unknown(UnknownReason::RootState);
    }
    if mana.is_legal() && hidden_control(unit) {
        Verdict::Unknown(UnknownReason::HiddenState)
    } else {
        mana
    }
}

/// Item readiness for its ordinary use, before choosing a target. As with
/// [`ability_readiness`], the view must be from its own side. Giving an item to an ally
/// may bypass cooldown and mana and is handled by the complete order check instead.
pub fn item_readiness(unit: &UnitView, slot: ItemSlot, item: &ItemView) -> Verdict {
    let ready = item_ready(unit, slot, item, false);
    if item_def(item.id).is_some_and(|def| def.mana_deficit) {
        ready.and(mana_deficit(unit))
    } else {
        ready
    }
}

pub(super) fn item_ready(unit: &UnitView, slot: ItemSlot, item: &ItemView, gives: bool) -> Verdict {
    use bota_proto::RejectReason;
    if unit.hp <= 0 {
        return Verdict::Illegal(RejectReason::HeroDead);
    }
    if usize::from(slot.0) >= bota_core::rules::INVENTORY_SLOTS && slot != ItemSlot::TOWN_PORTAL {
        return Verdict::Illegal(RejectReason::WrongTargetKind);
    }
    let Some(def) = item_def(item.id) else {
        return Verdict::Illegal(RejectReason::EmptySlot);
    };
    if let Err(reason) = bota_core::item_usability(def.aim, item.mute_left, illusion(unit)) {
        return Verdict::Illegal(reason);
    }
    let needs_charge = def.charges > 0 || def.cast_charges > 0;
    if needs_charge && item.charges.is_none() {
        return Verdict::Unknown(UnknownReason::MalformedView);
    }
    let resources = item_resource_check(item, slot, unit.mana, gives);
    if disabled(unit) {
        return resources.and(Verdict::Illegal(RejectReason::Disabled));
    }
    let mana = resources;
    if mana.is_legal() && hidden_control(unit) {
        Verdict::Unknown(UnknownReason::HiddenState)
    } else {
        mana
    }
}

/// Whether the shown pool proves a positive mana deficit. At a rounded maximum
/// it cannot distinguish a full pool from a small remaining deficit.
pub fn mana_deficit(unit: &UnitView) -> Verdict {
    use bota_proto::RejectReason;
    if unit.max_mana < 0 || unit.mana < 0 {
        return Verdict::Illegal(RejectReason::NotReady);
    }
    if unit.mana < unit.max_mana {
        return Verdict::Legal;
    }
    // Maximum mana is floored, while any positive current pool shows at least one.
    // Thus (shown current, maximum) = (1, 0) can still have a positive deficit.
    if unit.mana == unit.max_mana || (unit.mana == 1 && unit.max_mana == 0) {
        return Verdict::Unknown(UnknownReason::RoundedMana);
    }
    Verdict::Illegal(RejectReason::NotReady)
}

/// Resource-only readiness for a slot hint or a projected mana budget. Caster state and
/// targets need the full readiness/order APIs. Unknown mana is deliberately retained.
pub fn ability_resources(ability: &AbilityView, mana: i32) -> Verdict {
    if let Some(reason) =
        bota_core::ability_resource_refusal(ability.passive, ability.level, ability.cooldown_left)
    {
        return Verdict::Illegal(reason);
    }
    mana_affordability(mana, ability.mana_cost)
}

/// Resource-only ordinary item use. A stored rune needs no charge. This says nothing
/// about caster control, inventory copies or the recipient's mana deficit.
pub fn item_resources(item: &ItemView, slot: ItemSlot, mana: i32) -> Verdict {
    item_resource_check(item, slot, mana, false)
}

fn item_resource_check(item: &ItemView, slot: ItemSlot, mana: i32, gives: bool) -> Verdict {
    use bota_proto::RejectReason;
    if !bota_core::in_inventory(usize::from(slot.0)) && slot != ItemSlot::TOWN_PORTAL {
        return Verdict::Illegal(RejectReason::WrongTargetKind);
    }
    if let Err(reason) = bota_core::item_usability(item.aim, item.mute_left, false) {
        return Verdict::Illegal(reason);
    }
    if let Err(reason) = bota_core::item_readiness(
        item.cooldown_left > 0,
        gives,
        item.charges.is_some(),
        item.charges.unwrap_or(0),
        item.rune.is_some(),
    ) {
        return Verdict::Illegal(reason);
    }
    if gives {
        Verdict::Legal
    } else {
        mana_affordability(mana, item.mana_cost)
    }
}
