//! What the current public view proves about server admission of an order.
//!
//! Admission is checked against this snapshot, before input delay, regeneration or other
//! orders change it. It neither promises eventual execution nor chooses useful actions.
//! Range, damage and deny thresholds commonly affect execution without barring admission.
//! Orders held behind a cast point return Unknown(QueueState): admission now does not
//! prove acceptance when the server validates them again on take-up. A ledger can prove
//! a full queue, but cannot prove that a held item or target will still exist.
//! [`Reservations`] separately tracks conservative future resource budgets.
//!
//! For a living carrier, a charged Town Portal Scroll illustrates why Unknown matters: the starting scroll's
//! free charges cannot be sold, but bought charges can. ItemView omits that provenance,
//! so an otherwise eligible sale is Unknown(PurchaseHistory). A caller that must preserve
//! every potentially legal choice should offer both Legal and Unknown orders, letting
//! the server decide. Unknown must not be relabeled Legal merely to make that choice.
//!
//! ```
//! use bota_client_core::order_legality::{OrderContext, UnknownReason, Verdict};
//! use bota_core::{Catalogue, map_of};
//! use bota_proto::{GlyphView, HeroId, MapId, Order, PlayerView, RejectReason, SlotId, Team, WorldView};
//! # let player = PlayerView {
//! #     slot: SlotId(0), team: Team::Radiant, hero: HeroId::PUDGE, unit: None,
//! #     level: 1, xp: 0, gold: Some(0), stash: Some(vec![None; 6]), kit: None,
//! #     kills: 0, deaths: 0, assists: 0, last_hits: 0, denies: 0, respawn_left: 0,
//! #     creep_score: None, seen_at: Some(1), backpack: None, bought_back_at: None, buyback: None,
//! # };
//! # let mut view = WorldView {
//! #     tick: 1, viewer: Some(Team::Radiant), units: vec![], projectiles: vec![],
//! #     players: vec![player.clone()], felled_trees: vec![], planted_trees: vec![],
//! #     loot: vec![], aggro: vec![], order_delays: vec![], skipped_ticks: vec![],
//! #     stock: vec![], talents: vec![], teleports: vec![], clock: None, runes: vec![],
//! #     pools: vec![], lairs: vec![],
//! #     glyphs: vec![GlyphView { team: Team::Radiant, ready_in: 0, active_for: 0, ready_after: Some(0) }],
//! # };
//! // Use the seat's received view and the match's public map/catalogue parameters.
//! let check = |view: &WorldView| OrderContext {
//!     view, player: &player, map: map_of(MapId(2)), catalogue: Catalogue::FULL, cheats: false,
//! }.check(None, Order::Glyph, false);
//! assert_eq!(check(&view), Verdict::Legal);
//! view.glyphs[0].ready_after = Some(1);
//! assert_eq!(check(&view), Verdict::Illegal(RejectReason::OnCooldown));
//! // Withheld cooldown evidence cannot establish either result.
//! view.glyphs[0].ready_after = None;
//! assert_eq!(check(&view), Verdict::Unknown(UnknownReason::HiddenState));
//! ```

mod inventory;
mod purchase;
mod readiness;
pub use inventory::InventoryContext;
pub mod pseudo_queue;
mod reservations;
pub use readiness::{
    ability_readiness, ability_resources, item_readiness, item_resources, mana_affordability,
    mana_deficit,
};
pub use reservations::{Reservations, item_in, order_mana_is_affordable};

use bota_core::{Catalogue, MapDef, aimed_right, item_def, kind_def};
use bota_proto::*;

/// A proof about admission at the state described by the view. An Illegal reason names
/// a failed constraint; the server may report another refusal first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Verdict {
    Legal,
    Illegal(RejectReason),
    Unknown(UnknownReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownReason {
    RoundedMana,
    RootState,
    HiddenState,
    DynamicGround,
    QueueState,
    HostilitySight,
    PurchaseHistory,
    MalformedView,
}

impl From<Result<(), RejectReason>> for Verdict {
    fn from(result: Result<(), RejectReason>) -> Self {
        match result {
            Ok(()) => Self::Legal,
            Err(reason) => Self::Illegal(reason),
        }
    }
}

impl Verdict {
    pub const fn is_legal(self) -> bool {
        matches!(self, Self::Legal)
    }
    /// Whether refusal is proven. An exploratory caller can offer every order for
    /// which this is false, preserving choices whose legality is unknown.
    pub const fn is_illegal(self) -> bool {
        matches!(self, Self::Illegal(_))
    }
    pub(super) const fn from_check(holds: bool, reason: RejectReason) -> Self {
        if holds {
            Self::Legal
        } else {
            Self::Illegal(reason)
        }
    }
    pub(super) fn and(self, next: Self) -> Self {
        match (self, next) {
            (Self::Illegal(reason), _) | (_, Self::Illegal(reason)) => Self::Illegal(reason),
            (Self::Unknown(reason), _) | (_, Self::Unknown(reason)) => Self::Unknown(reason),
            _ => Self::Legal,
        }
    }
}

/// A seat's filtered snapshot and the match's public rules. Sequence scans are capped at
/// wire limits, all slot reads are checked, and no query allocates. Use coherent snapshots
/// from a [`crate::ClientSession`]; malformed or withheld information yields Unknown.
pub struct OrderContext<'a> {
    pub view: &'a WorldView,
    pub player: &'a PlayerView,
    pub map: &'a MapDef,
    pub catalogue: Catalogue,
    pub cheats: bool,
}

impl OrderContext<'_> {
    /// Checks an order against the current snapshot, independently of pseudo-queue
    /// reservations. Known cast-point deferral yields Unknown(QueueState), even with a
    /// complete ledger: admission now cannot prove acceptance at take-up. A named unit
    /// must belong to this seat. Glyph and a buyback need no body.
    pub fn check(&self, unit: Option<EntityId>, order: Order, queue: bool) -> Verdict {
        if !self.bounded() {
            return Verdict::Unknown(UnknownReason::MalformedView);
        }
        if self
            .view
            .viewer
            .is_some_and(|side| side != self.player.team)
        {
            return Verdict::Unknown(UnknownReason::HiddenState);
        }
        if order == Order::Glyph {
            return self.glyph();
        }
        if order == Order::Buyback {
            return self.buyback(unit);
        }
        let id = unit.or(self.player.unit);
        let body = id.and_then(|id| self.unit(id));
        if self.player.respawn_left > 0 && unit.is_none_or(|named| self.player.unit == Some(named))
        {
            return self.dead(order);
        }
        let Some(body) = body else {
            return Verdict::Unknown(UnknownReason::HiddenState);
        };
        if body.owner != Some(self.player.slot) {
            return Verdict::Illegal(RejectReason::NotYourUnit);
        }
        if body.hp <= 0 {
            return Verdict::Illegal(RejectReason::HeroDead);
        }
        if queue
            && bota_core::order_queues(order)
            && body.queue.len() >= bota_core::rules::MAX_QUEUED_ORDERS
        {
            return Verdict::Illegal(RejectReason::QueueFull);
        }
        let verdict = self.body_order(body, order);
        if body.cast.is_some()
            && (queue && bota_core::order_queues(order) || self.waits_for_cast(body, order))
        {
            Verdict::Unknown(UnknownReason::QueueState).and(verdict)
        } else {
            verdict
        }
    }

    /// Adds complete, next-tick issued-order evidence for the selected body's pseudo-queue.
    /// Do not supply a ledger if input delay or another controller could make it incomplete.
    pub fn check_with_ledger(
        &self,
        unit: Option<EntityId>,
        order: Order,
        queue: bool,
        owed: &pseudo_queue::OwedOrders,
    ) -> Verdict {
        let verdict = self.check(unit, order, queue);
        if verdict != Verdict::Unknown(UnknownReason::QueueState) {
            return verdict;
        }
        if !queue
            && bota_core::order_is_cast(order)
            && owed.casts_waiting()
            && owed.len() >= bota_core::rules::MAX_QUEUED_ORDERS
        {
            Verdict::Illegal(RejectReason::QueueFull)
        } else {
            verdict
        }
    }

    fn waits_for_cast(&self, body: &UnitView, order: Order) -> bool {
        if bota_core::order_halts(order) {
            return false;
        }
        match order {
            Order::Cast { slot, .. } => body
                .abilities
                .get(usize::from(slot.0))
                .and_then(|ability| bota_core::ability_def(ability.id))
                .is_some_and(|def| !def.immediate),
            Order::Use { slot, .. } => item_in(body, slot)
                .and_then(|item| item_def(item.id))
                .is_some_and(|def| !def.immediate),
            _ => bota_core::order_queues(order),
        }
    }

    fn bounded(&self) -> bool {
        use bota_proto::limits::*;
        self.view.units.len() <= MAX_VIEW_UNITS
            && self.view.players.len() <= MAX_SEATS
            && self.view.loot.len() <= MAX_SNAPSHOT_LOOT
            && self.view.runes.len() <= MAX_SNAPSHOT_RUNES
            && self.view.glyphs.len() <= MAX_SEATS
            && self.view.talents.len() <= MAX_SEATS
            && self.view.stock.len() <= MAX_VIEW_STOCK
            && self
                .player
                .stash
                .as_ref()
                .is_none_or(|stash| stash.len() <= STASH_SLOTS)
            && self.player.kit.as_ref().is_none_or(|kit| {
                kit.items.len() <= HERO_ITEM_SLOTS && kit.abilities.len() <= MAX_ABILITY_SLOTS
            })
            && self
                .view
                .units
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id)
            && self.view.units.iter().all(|unit| {
                unit.abilities.len() <= MAX_ABILITY_SLOTS
                    && unit.items.len() <= HERO_ITEM_SLOTS
                    && unit.effects.len() <= MAX_EFFECTS_PER_UNIT
                    && unit.queue.len() <= MAX_QUEUED_ORDERS
                    && [unit.pos.x.raw, unit.pos.y.raw, unit.bound.raw]
                        .into_iter()
                        .all(|raw| {
                            (0..=Fixed::from_int(bota_core::rules::MAP_SIZE).raw).contains(&raw)
                        })
            })
    }

    fn unit(&self, id: EntityId) -> Option<&UnitView> {
        self.view
            .units
            .iter()
            .take(bota_proto::limits::MAX_VIEW_UNITS)
            .find(|unit| unit.id == id)
    }

    fn seen(&self, id: EntityId) -> bool {
        self.unit(id).is_some()
            || self
                .view
                .loot
                .iter()
                .take(bota_proto::limits::MAX_SNAPSHOT_LOOT)
                .any(|loot| loot.id == id)
            || self
                .view
                .runes
                .iter()
                .take(bota_proto::limits::MAX_SNAPSHOT_RUNES)
                .any(|rune| rune.id == id)
    }

    fn body_order(&self, body: &UnitView, order: Order) -> Verdict {
        match order {
            Order::Move { target } | Order::Attack { target } => self.navigation(target),
            Order::MoveDirection { .. } | Order::Learn { .. } => Verdict::Legal,
            Order::Cast { slot, target } => self.cast(body, slot, target),
            Order::Use { slot, target } => self.use_item(body, slot, target),
            Order::Put { slot, target } => self.put(body, slot, target),
            Order::Take { target } => self.take(body, target),
            Order::Buy { item } => self.buy(item),
            Order::Sell { slot } => self.sell(body, slot),
            Order::Swap { from, to } => self.swap(body, from, to),
            Order::SwitchWard { slot } => self.switch(body, slot),
            Order::Disassemble { slot } => self.disassemble(body, slot),
            Order::SetCombineLock { slot, .. } => self.lock(body, slot),
            Order::LearnTalent { talent } => self.talent(body, talent),
            Order::LearnAttributeBonus => self.attribute_bonus(body),
            Order::Cheat { cheat } => self.cheat(cheat, true),
            Order::Glyph => self.glyph(),
            Order::Buyback => self.buyback(Some(body.id)),
        }
    }

    fn navigation(&self, target: Target) -> Verdict {
        let Target::Unit(id) = target else {
            return Verdict::Legal;
        };
        if !self.seen(id) {
            return Verdict::Illegal(RejectReason::UnknownTarget);
        }
        // Objective interactions use hidden ability slots that the public book omits.
        if self
            .unit(id)
            .is_some_and(|unit| kind_def(unit.kind).interact.is_some())
        {
            Verdict::Unknown(UnknownReason::HiddenState)
        } else {
            Verdict::Legal
        }
    }

    fn cast(&self, body: &UnitView, slot: AbilitySlot, target: Target) -> Verdict {
        let Some(ability) = body.abilities.get(usize::from(slot.0)) else {
            return Verdict::Illegal(RejectReason::EmptySlot);
        };
        let ready = ability_readiness(body, ability);
        let Some(def) = bota_core::ability_def(ability.id) else {
            return ready;
        };
        if !aimed_right(def.aim, &target) {
            return Verdict::Illegal(RejectReason::WrongTargetKind);
        }
        if let Target::Unit(id) = target {
            if !self.seen(id) {
                return Verdict::Illegal(RejectReason::UnknownTarget);
            }
            let Some(mark) = self.unit(id) else {
                return ready.and(Verdict::Unknown(UnknownReason::HiddenState));
            };
            let allied = mark.team == body.team;
            if (def.at_an_enemy
                && !bota_core::hostile_body(
                    Some(body.team),
                    Some(mark.team),
                    mark.hp > 0,
                    mark.statuses.bits & StatusFlags::INVULNERABLE != 0,
                ))
                || !bota_core::ability_target_kind(def, Some(mark.kind), allied)
            {
                return Verdict::Illegal(RejectReason::WrongTargetKind);
            }
            // Addressability uses shows(), but hostile spells also require can_see().
            // A unit moved after the sight pass can be shown by the former while the
            // cached latter still excludes it. The snapshot does not carry that cache.
            if def.at_an_enemy {
                return ready.and(Verdict::Unknown(UnknownReason::HostilitySight));
            }
        }
        ready
    }

    fn use_item(&self, body: &UnitView, slot: ItemSlot, target: Target) -> Verdict {
        let Some(item) = item_in(body, slot) else {
            return Verdict::Illegal(RejectReason::EmptySlot);
        };
        let Some(def) = item_def(item.id) else {
            return Verdict::Illegal(RejectReason::EmptySlot);
        };
        let ally = def
            .ally
            .filter(|_| matches!(target, Target::Unit(id) if id != body.id));
        let gives = matches!(ally, Some(bota_core::AllyUse::Give { .. }));
        let ready = readiness::item_ready(body, slot, item, gives);
        let Some(aim) = def.aim else {
            return ready;
        };
        if let Target::Unit(id) = target
            && !self.seen(id)
        {
            return Verdict::Illegal(RejectReason::UnknownTarget);
        }
        if let Some(ally) = ally.filter(|_| aim != Aim::Unit) {
            let mark = match target {
                Target::Unit(id) => self.unit(id),
                _ => None,
            };
            let fits = mark.is_some_and(|mark| {
                mark.id != body.id
                    && mark.team == body.team
                    && mark.hp > 0
                    && match ally {
                        bota_core::AllyUse::Give { .. } => {
                            matches!(mark.kind, UnitKind::Hero | UnitKind::Courier)
                        }
                        bota_core::AllyUse::Effect { .. } => mark.kind == UnitKind::Hero,
                    }
            });
            if !fits {
                return Verdict::Illegal(RejectReason::WrongTargetKind);
            }
        } else {
            let fits = if def.mana_deficit {
                target == Target::None || target == Target::Unit(body.id)
            } else {
                aimed_right(aim, &target) || (def.ally_target && matches!(target, Target::Unit(_)))
            };
            if !fits {
                return Verdict::Illegal(RejectReason::WrongTargetKind);
            }
        }
        let ready = if def.mana_deficit {
            let receiver = match target {
                Target::Unit(id) if id != body.id => self.unit(id),
                _ => Some(body),
            };
            ready.and(receiver.map_or(
                Verdict::Unknown(UnknownReason::HiddenState),
                readiness::mana_deficit,
            ))
        } else {
            ready
        };
        if aim == Aim::Building {
            let destination = if def.scroll().is_some() {
                self.scroll_target(body, target)
            } else {
                Verdict::Illegal(RejectReason::WrongTargetKind)
            };
            return ready.and(destination);
        }
        ready
    }

    fn scroll_target(&self, body: &UnitView, target: Target) -> Verdict {
        let fits = match target {
            Target::Unit(id) => self.unit(id).is_some_and(|mark| {
                mark.team == body.team && mark.hp > 0 && kind_def(mark.kind).takes_scrolls
            }),
            Target::None => bota_core::may_scroll_to_fountain(
                body.pos,
                bota_core::fountain_pos(self.map, body.team),
            ),
            Target::Pos(_) => true,
        };
        if !fits {
            return Verdict::Illegal(RejectReason::WrongTargetKind);
        }
        Verdict::Unknown(UnknownReason::DynamicGround)
    }

    fn glyph(&self) -> Verdict {
        if self.player.team == Team::Neutral {
            return Verdict::Illegal(RejectReason::NotYourUnit);
        }
        self.view
            .glyphs
            .iter()
            .take(bota_proto::limits::MAX_SEATS)
            .find(|glyph| glyph.team == self.player.team)
            .and_then(|glyph| glyph.ready_after)
            .map_or(Verdict::Unknown(UnknownReason::HiddenState), |left| {
                bota_core::glyph_refusal(self.player.team, left).into()
            })
    }

    /// A buyback by what the seat's own row tells: whether the hero lies
    /// dead, its gold, the cost, the cooldown, and from what the dead hero
    /// left whether it is rising by a reincarnation.
    fn buyback(&self, named: Option<EntityId>) -> Verdict {
        if let Some(named) = named.filter(|named| self.player.unit != Some(*named)) {
            return match self.unit(named).map(|unit| unit.owner) {
                Some(owner) if owner == Some(self.player.slot) => {
                    Verdict::Illegal(RejectReason::NotAHero)
                }
                Some(_) => Verdict::Illegal(RejectReason::NotYourUnit),
                None => Verdict::Unknown(UnknownReason::HiddenState),
            };
        }
        let (Some(gold), Some(buyback)) = (self.player.gold, self.player.buyback) else {
            return Verdict::Unknown(UnknownReason::HiddenState);
        };
        let dead = self.player.respawn_left > 0;
        let reincarnating = self.player.kit.as_ref().map(|kit| kit.reincarnating);
        let told = |reincarnating| {
            bota_core::buyback_refusal(
                dead,
                gold,
                buyback.cost,
                self.view.tick,
                buyback.ready_at,
                reincarnating,
            )
        };
        match (reincarnating, told(reincarnating.unwrap_or(false))) {
            (_, Err(reason)) => Verdict::Illegal(reason),
            (Some(_), Ok(())) => Verdict::Legal,
            (None, Ok(())) => Verdict::Unknown(UnknownReason::HiddenState),
        }
    }

    fn talent(&self, body: &UnitView, slot: TalentSlot) -> Verdict {
        if self.player.unit != Some(body.id) {
            return Verdict::Unknown(UnknownReason::HiddenState);
        }
        self.view
            .talents
            .iter()
            .take(bota_proto::limits::MAX_SEATS)
            .find(|talent| talent.slot == self.player.slot)
            .and_then(|talent| talent.may_learn)
            .map_or(Verdict::Unknown(UnknownReason::HiddenState), |slots| {
                bota_core::learning_refusal(slot.0 < TalentSlot::COUNT && slots.has(slot)).into()
            })
    }

    fn attribute_bonus(&self, body: &UnitView) -> Verdict {
        if self.player.unit != Some(body.id) {
            return Verdict::Unknown(UnknownReason::HiddenState);
        }
        self.view
            .talents
            .iter()
            .take(bota_proto::limits::MAX_SEATS)
            .find(|talent| talent.slot == self.player.slot)
            .and_then(|talent| talent.may_learn_attribute_bonus)
            .map_or(Verdict::Unknown(UnknownReason::HiddenState), |ready| {
                bota_core::learning_refusal(ready).into()
            })
    }

    fn cheat(&self, cheat: Cheat, has_body: bool) -> Verdict {
        if let Err(reason) = bota_core::cheat_refusal(
            self.cheats,
            cheat,
            self.catalogue,
            !self.map.rune_spots.is_empty(),
        ) {
            return Verdict::Illegal(reason);
        }
        if !has_body
            && matches!(
                cheat,
                Cheat::ApplyModifier { .. } | Cheat::ClearModifiers { .. } | Cheat::Teleport { .. }
            )
        {
            return Verdict::Illegal(RejectReason::HeroDead);
        }
        // Whether this seat was created by a cheat is not in the scoreboard.
        Verdict::Unknown(UnknownReason::HiddenState)
    }

    fn dead(&self, order: Order) -> Verdict {
        if let Order::Buy { item } = order {
            return self.buy(item);
        }
        if let Order::Cheat { cheat } = order {
            return self.cheat(cheat, false);
        }
        let Some(kit) = self.player.kit.as_ref() else {
            return Verdict::Unknown(UnknownReason::HiddenState);
        };
        let inventory =
            InventoryContext::fallen(kit, self.player.stash.as_deref(), self.player.slot);
        match order {
            Order::Sell { slot } => Verdict::from_check(
                u32::from(slot.0) < u32::BITS && kit.sellable_slots & (1_u32 << slot.0) != 0,
                RejectReason::HeroDead,
            ),
            Order::Swap { from, to } => inventory.swap(from, to),
            Order::Disassemble { slot } => inventory.disassemble(slot),
            Order::SetCombineLock { slot, .. } => inventory.lock(slot),
            _ => Verdict::Illegal(RejectReason::HeroDead),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn order_legality_a_proven_refusal_wins_over_uncertainty_in_either_order() {
        let unknown = Verdict::Unknown(UnknownReason::QueueState);
        let illegal = Verdict::Illegal(RejectReason::WrongTargetKind);
        assert_eq!(unknown.and(illegal), illegal);
        assert_eq!(illegal.and(unknown), illegal);
        assert_eq!(unknown.and(Verdict::Legal), unknown);
        assert_eq!(Verdict::Legal.and(unknown), unknown);
    }
}
