//! Controller intent belongs to the client that issues it, independently of unit animation.
use bota_proto::{
    AbilityId, EntityId, EventKind, Fixed, Inflictor, ItemId, Order, StatusFlags, Target, UnitView,
};

/// Maximum waiting entries per controlled body.
pub const BOT_ACTION_CAPACITY: usize = bota_core::rules::MAX_QUEUED_ORDERS;
/// Safety bound for an action whose effect cannot be identified from the received sheet.
pub const UNKNOWN_EFFECT_TIMEOUT_TICKS: u32 = 5 * bota_core::rules::TICKS_PER_SECOND;
/// Safety bound for an item request with no observed application or refusal.
pub const ITEM_ACTION_TIMEOUT_TICKS: u32 = 5 * bota_core::rules::TICKS_PER_SECOND;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotActionType {
    Idle,
    MoveTo,
    MoveDirectly,
    Attack,
    AttackMove,
    UseAbility,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotActionMode {
    Replace,
    Queue,
    Push,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BotAction {
    pub kind: BotActionType,
    pub order: Order,
    pub once: bool,
    pub effect: Option<BotActionEffect>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunningBotAction {
    pub action: BotAction,
    pub started_at: u32,
    pub cast_observed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BotActionQueueFull;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ItemActionEnd {
    Applied,
    Failed,
}

/// A bounded waiting FIFO and one current action. A pending entry becomes current
/// in the same controller step, after the callback finishes.
/// All mutation is O(capacity), independent of world size; no allocation after construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BotActions {
    owner: Option<EntityId>,
    waiting: [Option<BotAction>; BOT_ACTION_CAPACITY],
    len: usize,
    current: Option<RunningBotAction>,
    deaths: Option<u16>,
}
impl Default for BotActions {
    fn default() -> Self {
        Self {
            owner: None,
            waiting: [None; BOT_ACTION_CAPACITY],
            len: 0,
            current: None,
            deaths: None,
        }
    }
}
impl BotActions {
    /// Replacing a generational body clears its old controller, including queued intentions.
    pub fn bind(&mut self, owner: Option<EntityId>) {
        if self.owner != owner {
            *self = Self {
                owner,
                ..Self::default()
            };
        }
    }
    pub const fn owner(&self) -> Option<EntityId> {
        self.owner
    }
    pub const fn running(&self) -> Option<RunningBotAction> {
        self.current
    }
    /// O(1). Effective current includes the first waiting entry before the controller steps.
    pub fn current_type(&self) -> BotActionType {
        self.current
            .map(|current| current.action.kind)
            .or_else(|| self.waiting[0].map(|action| action.kind))
            .unwrap_or(BotActionType::Idle)
    }
    /// Waiting entries only, excluding the running action.
    pub const fn queued_len(&self) -> usize {
        self.len
    }
    pub fn queued_type(&self, index: usize) -> Option<BotActionType> {
        self.waiting
            .get(index)
            .copied()
            .flatten()
            .map(|action| action.kind)
    }
    /// A same-target Action_AttackUnit is a complete no-op, including when its attack-once option changes.
    /// Overflow is rejected before any entry changes. Returns whether it changed the controller.
    pub fn request(
        &mut self,
        action: BotAction,
        mode: BotActionMode,
    ) -> Result<bool, BotActionQueueFull> {
        if mode == BotActionMode::Replace && action.kind == BotActionType::Attack {
            let effective = self.current.map(|entry| entry.action).or(self.waiting[0]);
            if effective
                .is_some_and(|old| old.kind == BotActionType::Attack && old.order == action.order)
            {
                return Ok(false);
            }
        }
        let needed = 1 + usize::from(mode == BotActionMode::Push && self.current.is_some());
        if mode != BotActionMode::Replace && self.len + needed > BOT_ACTION_CAPACITY {
            return Err(BotActionQueueFull);
        }
        match mode {
            BotActionMode::Replace => {
                self.clear(false);
                self.waiting[0] = Some(action);
                self.len = 1;
            }
            BotActionMode::Queue => {
                self.waiting[self.len] = Some(action);
                self.len += 1;
            }
            BotActionMode::Push => {
                if let Some(current) = self.current.take() {
                    self.push_front(current.action);
                }
                self.push_front(action);
            }
        }
        Ok(true)
    }
    fn push_front(&mut self, action: BotAction) {
        self.waiting.copy_within(0..self.len, 1);
        self.waiting[0] = Some(action);
        self.len += 1;
    }
    /// O(capacity). Clearing never manufactures a stop unless explicitly requested.
    pub fn clear(&mut self, stop: bool) -> Option<Order> {
        self.waiting.fill(None);
        self.len = 0;
        self.current = None;
        stop.then_some(Order::Move {
            target: Target::None,
        })
    }
    /// Start the waiting head once; the caller transmits the returned ordinary order.
    pub fn start(&mut self, tick: u32) -> Option<Order> {
        if self.current.is_some() || self.len == 0 {
            return None;
        }
        let action = self.waiting[0].take().expect("occupied waiting head");
        self.waiting.copy_within(1..self.len, 0);
        self.len -= 1;
        self.waiting[self.len] = None;
        self.current = Some(RunningBotAction {
            action,
            started_at: tick,
            cast_observed: false,
        });
        Some(action.order)
    }
    /// Completion is fed by observed facts, never inferred from a script-writable table.
    pub fn complete(&mut self) {
        self.current = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotActionEffect {
    Ability(AbilityId),
    Item(ItemId),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BotActionImpact {
    pub tick: u32,
    pub source: EntityId,
    pub effect: BotActionEffect,
}
impl BotActionImpact {
    pub fn observed(tick: u32, event: &EventKind) -> Option<Self> {
        let (source, effect) = match event {
            EventKind::Damaged {
                source: Some(source),
                inflictor: Some(inflictor),
                removal: false,
                ..
            } => {
                let effect = match inflictor {
                    Inflictor::Ability(id) => BotActionEffect::Ability(*id),
                    Inflictor::Item(id) => BotActionEffect::Item(*id),
                };
                (*source, effect)
            }
            EventKind::ItemUsed {
                caster: Some(source),
                item,
                ..
            } => (*source, BotActionEffect::Item(*item)),
            _ => return None,
        };
        Some(Self {
            tick,
            source,
            effect,
        })
    }
}
/// Only received observations enter completion queries. Unit lookup must exclude fog memory.
pub trait BotActionView {
    fn tick(&self) -> u32;
    fn visible_unit(&self, id: EntityId) -> Option<&UnitView>;
    fn damage_dealt_at(&self, id: EntityId) -> Option<u32>;
    fn impacts(&self) -> impl Iterator<Item = BotActionImpact>;
    fn item_use_observation(&self, _id: EntityId) -> Option<crate::ItemUseObservation> {
        None
    }
    fn item_use_refusal(&self, _id: EntityId) -> Option<crate::ItemUseRefusal> {
        None
    }
    fn deaths(&self, _id: EntityId) -> Option<u16> {
        None
    }
}
impl BotActionView for crate::ClientSession {
    fn item_use_refusal(&self, id: EntityId) -> Option<crate::ItemUseRefusal> {
        self.item_use_refusals()
            .into_iter()
            .flatten()
            .find(|fact| fact.source == id)
    }
    fn item_use_observation(&self, id: EntityId) -> Option<crate::ItemUseObservation> {
        self.item_use_observations()
            .into_iter()
            .flatten()
            .find(|fact| fact.source == id)
    }
    fn deaths(&self, id: EntityId) -> Option<u16> {
        let unit = self.current()?.unit(id)?;
        if unit.kind != bota_proto::UnitKind::Hero {
            return None;
        }
        self.current()?
            .player(unit.owner?)
            .map(|player| player.deaths)
    }
    fn tick(&self) -> u32 {
        self.current().map_or(0, |view| view.tick)
    }
    fn visible_unit(&self, id: EntityId) -> Option<&UnitView> {
        self.current()?.unit(id)
    }
    fn damage_dealt_at(&self, id: EntityId) -> Option<u32> {
        let index = self
            .entities()
            .binary_search_by_key(&id, |track| track.id)
            .ok()?;
        self.entities()[index]
            .last_damage_dealt
            .map(|damage| damage.tick)
    }
    fn impacts(&self) -> impl Iterator<Item = BotActionImpact> {
        self.recent_events()
            .iter()
            .filter_map(|event| BotActionImpact::observed(event.tick, &event.kind))
    }
}
impl BotActions {
    /// O(visible-unit lookup + received impacts + capacity). Advance the current action.
    /// Some engine-specific endings and bot waypoint planning remain unsupported.
    pub fn update(&mut self, view: &impl BotActionView) -> Option<Order> {
        let source = self.owner?;
        if view.visible_unit(source).is_none_or(|unit| unit.hp <= 0) {
            self.clear(false);
            return None;
        }
        let deaths = view.deaths(source);
        if self.deaths.is_some() && deaths.is_some() && self.deaths != deaths {
            self.clear(false);
        }
        self.deaths = deaths;
        if let Some(current) = &mut self.current
            && current.action.kind == BotActionType::UseAbility
            && view
                .visible_unit(source)
                .is_some_and(|unit| unit.cast.is_some() || unit.channel.is_some())
        {
            current.cast_observed = true;
        }
        let mut stop = false;
        if let Some(current) = self.current {
            let item_end = self.item_ending(view, source, current);
            if item_end.is_some() || self.finished(view, source, current) {
                stop = (current.action.kind == BotActionType::Attack && current.action.once)
                    || item_end == Some(ItemActionEnd::Failed);
                self.complete();
            }
        }
        self.start_ready(view).or_else(|| {
            stop.then_some(Order::Move {
                target: Target::None,
            })
        })
    }
    /// O(capacity * unit lookup). Start newly requested work after a callback, without
    /// consuming the same received completion evidence a second time.
    pub fn start_ready(&mut self, view: &impl BotActionView) -> Option<Order> {
        if self.current.is_some() || self.len == 0 {
            return None;
        }
        let source = self.owner?;
        if view.visible_unit(source).is_none_or(|unit| unit.hp <= 0) {
            self.clear(false);
            return None;
        }
        for _ in 0..BOT_ACTION_CAPACITY {
            let next = self.waiting[0]?;
            let order = self.start(view.tick()).expect("waiting action starts");
            if self.invalid_target(view, next) {
                self.complete();
                continue;
            }
            return Some(order);
        }
        None
    }
    fn invalid_target(&self, view: &impl BotActionView, action: BotAction) -> bool {
        match action.order {
            Order::Attack {
                target: Target::Unit(target),
            }
            | Order::Cast {
                target: Target::Unit(target),
                ..
            }
            | Order::Use {
                target: Target::Unit(target),
                ..
            } => view.visible_unit(target).is_none_or(|unit| unit.hp <= 0),
            _ => false,
        }
    }
    /// Application and failed prerequisites both release the head, but only failures cancel body work.
    fn item_ending(
        &self,
        view: &impl BotActionView,
        source: EntityId,
        current: RunningBotAction,
    ) -> Option<ItemActionEnd> {
        let Order::Use { slot, .. } = current.action.order else {
            return None;
        };
        if let Some(ending) = Self::observed_item_ending(view, source, current) {
            return Some(ending);
        }
        let unit = view.visible_unit(source)?;
        if self.invalid_target(view, current.action)
            || unit.statuses.bits & StatusFlags::STUNNED != 0
            || view.tick().saturating_sub(current.started_at) >= ITEM_ACTION_TIMEOUT_TICKS
        {
            return Some(ItemActionEnd::Failed);
        }
        if let Some(BotActionEffect::Item(expected)) = current.action.effect {
            let item = if slot == bota_proto::ItemSlot::TOWN_PORTAL {
                unit.town_portal.as_ref()
            } else {
                unit.items.get(usize::from(slot.0)).and_then(Option::as_ref)
            };
            if item.is_none_or(|item| item.id != expected || item.mute_left > 0) {
                return Some(ItemActionEnd::Failed);
            }
        }
        None
    }
    fn observed_item_ending(
        view: &impl BotActionView,
        source: EntityId,
        current: RunningBotAction,
    ) -> Option<ItemActionEnd> {
        if let Some(fact) = view.item_use_refusal(source)
            && fact.source == source
            && fact.order == current.action.order
            && fact.issued_at >= current.started_at
        {
            return Some(ItemActionEnd::Failed);
        }
        if let Some(fact) = view.item_use_observation(source)
            && fact.source == source
            && fact.tick > current.started_at
            && matches!(current.action.effect,Some(BotActionEffect::Item(item)) if item==fact.item)
        {
            return Some(ItemActionEnd::Applied);
        }
        None
    }
    fn finished(
        &self,
        view: &impl BotActionView,
        source: EntityId,
        current: RunningBotAction,
    ) -> bool {
        if matches!(current.action.order, Order::Use { .. }) {
            return false;
        }
        if self.invalid_target(view, current.action) {
            return true;
        }
        let Some(unit) = view.visible_unit(source) else {
            return true;
        };
        match current.action.kind {
            BotActionType::MoveTo | BotActionType::MoveDirectly | BotActionType::AttackMove => {
                let (Order::Move {
                    target: Target::Pos(point),
                }
                | Order::Attack {
                    target: Target::Pos(point),
                }) = current.action.order
                else {
                    return false;
                };
                unit.pos.within(
                    point,
                    Fixed::from_int(if current.action.kind == BotActionType::MoveDirectly {
                        10
                    } else {
                        25
                    }),
                )
            }
            BotActionType::Attack => {
                current.action.once
                    && view
                        .damage_dealt_at(source)
                        .is_some_and(|tick| tick > current.started_at)
            }
            BotActionType::UseAbility => {
                let Some(effect) = current.action.effect else {
                    return view.tick().saturating_sub(current.started_at)
                        >= UNKNOWN_EFFECT_TIMEOUT_TICKS;
                };
                if view.impacts().any(|impact| {
                    impact.source == source
                        && impact.effect == effect
                        && impact.tick > current.started_at
                }) {
                    return true;
                }
                if matches!(effect, BotActionEffect::Ability(_))
                    && unit.statuses.bits & (StatusFlags::SILENCED | StatusFlags::STUNNED) != 0
                {
                    return true;
                }
                if view.tick() <= current.started_at || !current.cast_observed {
                    return false;
                }
                // A never-started spell may be walking into range; an absent cast phase
                // alone does not prove an ending.
                match effect {
                    BotActionEffect::Ability(ability) => {
                        unit.cast.is_none_or(|cast| cast.ability != Some(ability))
                            && unit
                                .channel
                                .is_none_or(|channel| channel.ability != ability)
                    }
                    BotActionEffect::Item(_) => unit.cast.is_none() && unit.channel.is_none(),
                }
            }
            BotActionType::Idle => false,
        }
    }
}

impl BotAction {
    /// Classify an ordinary client-issued body order using its received equipment sheet.
    pub fn ordinary(order: Order, unit: &UnitView) -> Option<Self> {
        let (kind, effect) = match order {
            Order::Move {
                target: Target::Pos(_),
            } => (BotActionType::MoveTo, None),
            Order::Attack {
                target: Target::Unit(_),
            } => (BotActionType::Attack, None),
            Order::Attack {
                target: Target::Pos(_),
            } => (BotActionType::AttackMove, None),
            Order::Cast { slot, .. } => (
                BotActionType::UseAbility,
                unit.abilities
                    .get(usize::from(slot.0))
                    .map(|ability| BotActionEffect::Ability(ability.id)),
            ),
            Order::Use { slot, .. } => {
                let item = if slot == bota_proto::ItemSlot::TOWN_PORTAL {
                    unit.town_portal.as_ref()
                } else {
                    unit.items.get(usize::from(slot.0)).and_then(Option::as_ref)
                };
                (
                    BotActionType::UseAbility,
                    item.map(|item| BotActionEffect::Item(item.id)),
                )
            }
            _ => return None,
        };
        Some(Self {
            kind,
            order,
            once: false,
            effect,
        })
    }
}
impl BotActions {
    /// The embedder's native policy can adopt an already-published ordinary order.
    pub fn adopt(&mut self, action: BotAction, tick: u32) {
        self.clear(false);
        self.current = Some(RunningBotAction {
            action,
            started_at: tick,
            cast_observed: false,
        });
    }
    pub fn needs_start(&self) -> bool {
        self.current.is_none() && self.len > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn attack(once: bool) -> BotAction {
        BotAction {
            kind: BotActionType::Attack,
            order: Order::Attack {
                target: Target::Unit(EntityId {
                    idx: 7,
                    generation: 1,
                }),
            },
            once,
            effect: None,
        }
    }
    #[test]
    fn effective_current_and_waiting_queries_follow_controller_boundaries() {
        let mut state = BotActions::default();
        assert_eq!(state.current_type(), BotActionType::Idle);
        state
            .request(attack(false), BotActionMode::Replace)
            .unwrap();
        assert_eq!(state.current_type(), BotActionType::Attack);
        assert_eq!(state.queued_len(), 1);
        assert_eq!(state.start(1), Some(attack(false).order));
        assert_eq!(state.queued_len(), 0);
        assert!(!state.request(attack(true), BotActionMode::Replace).unwrap());
        assert_eq!(state.running().unwrap().started_at, 1);
        state.request(attack(true), BotActionMode::Queue).unwrap();
        assert_eq!(state.queued_type(0), Some(BotActionType::Attack));
        assert_eq!(state.start(2), None);
        state.complete();
        assert_eq!(state.start(3), Some(attack(true).order));
        assert!(state.running().unwrap().action.once);
    }
    #[test]
    fn overflow_is_atomic_and_a_replaced_body_cannot_keep_queued_orders() {
        let mut state = BotActions::default();
        state.bind(Some(EntityId {
            idx: 1,
            generation: 1,
        }));
        for _ in 0..BOT_ACTION_CAPACITY {
            state.request(attack(true), BotActionMode::Queue).unwrap();
        }
        let before = state;
        assert_eq!(
            state.request(attack(true), BotActionMode::Push),
            Err(BotActionQueueFull)
        );
        assert_eq!(state, before);
        state.bind(Some(EntityId {
            idx: 1,
            generation: 2,
        }));
        assert_eq!(state.current_type(), BotActionType::Idle);
        assert_eq!(state.queued_len(), 0);
    }
}
