use bota_client_core::{ClientSession, IssuedOrder, VelocityEstimate};
use bota_proto::{
    AbilityView, EntityId, HeroId, ItemSlot, ItemView, Kit, SlotId, StockView, TalentView, Team,
    UnitView,
};
use luagee::{LuaError, TableId, Value, Vm};

use crate::{BindingError, BotSeat};

pub(crate) const MAX_ORDERS: usize = 64;
const MAX_HANDLES: usize = 8192;
const MAX_SNAPSHOT_BYTES: usize = 4 << 20;
// Covers two snapshots during replacement, handle storage, callback captures and bounded seat bookkeeping.
pub(crate) const HOST_STORAGE_BYTES: usize = 16 << 20;

pub(crate) struct Player {
    pub slot: SlotId,
    pub team: Team,
    pub hero: HeroId,
    pub level: u8,
    pub deaths: u16,
    pub last_seen: Option<(u32, bota_proto::Vec2)>,
    pub statistics: Option<UnitStatistics>,
    pub unit: Option<EntityId>,
    pub gold: Option<i32>,
    pub kit: Option<Kit>,
    pub stash: Option<Vec<Option<ItemView>>>,
    pub respawn_left: u32,
    /// The seat's buyback, as its own side is told it.
    pub buyback: Option<bota_proto::BuybackView>,
}

/// Scalar statistics received for a body; no inventory, effects or new hidden state.
#[derive(Clone, Copy)]
pub(crate) struct UnitStatistics {
    pub mana: i32,
    pub max_mana: i32,
    pub hp_regen: bota_proto::Fixed,
    pub mana_regen: bota_proto::Fixed,
    pub armor: bota_proto::Fixed,
    pub attack_damage: i32,
    pub attack_range: bota_proto::Fixed,
    pub attack_time: u32,
    /// The attack speed the server times with, its fraction included.
    pub attack_speed: bota_proto::Fixed,
    pub move_speed: bota_proto::Fixed,
    pub bound: bota_proto::Fixed,
    pub evasion: bota_proto::Fixed,
    pub facing: bota_proto::Angle,
    pub attributes: bota_proto::Attributes,
    pub primary: Option<bota_proto::Attribute>,
}

impl From<&UnitView> for UnitStatistics {
    fn from(unit: &UnitView) -> Self {
        Self {
            mana: unit.mana,
            max_mana: unit.max_mana,
            hp_regen: unit.hp_regen,
            mana_regen: unit.mana_regen,
            armor: unit.armor,
            attack_damage: unit.attack_damage,
            attack_range: unit.attack_range,
            attack_time: unit.attack_time,
            attack_speed: bota_core::exact_attack_speed(unit),
            move_speed: unit.move_speed,
            bound: unit.bound,
            evasion: unit.evasion,
            facing: unit.facing,
            attributes: unit.attributes,
            primary: unit.primary,
        }
    }
}

pub(crate) struct KnownUnit {
    pub unit: UnitView,
    pub visible: bool,
    pub last_visible: (u32, bota_proto::Vec2),
    pub velocity: Option<VelocityEstimate>,
    pub activity: bota_client_core::ActivityClock,
    pub recent_damage: bota_client_core::RecentDamage,
    pub attack_target: Option<EntityId>,
    pub damage_dealt_at: Option<u32>,
    pub direction_history: bota_client_core::DirectionHistory,
}

pub(crate) struct AttackFact {
    pub tick: u32,
    pub source: EntityId,
    pub target: Option<EntityId>,
}

#[derive(Clone, Copy)]
pub(crate) struct PendingOnce {
    pub source: EntityId,
    pub target: EntityId,
    pub tick: u32,
    pub launched: bool,
    /// Native policy requests expire; explicit script requests have no host lease.
    pub expires_at: Option<u32>,
}

pub(crate) struct Snapshot {
    pub ended: Option<bota_client_core::SessionEnd>,
    pub statics: std::sync::Arc<bota_client_core::MatchStatic>,
    pub felled_trees: Vec<u32>,
    pub planted_trees: Vec<bota_proto::Vec2>,
    pub team: Team,
    pub tick: u32,
    pub game_tick: u32,
    pub day_tick: u32,
    pub horn: u32,
    pub map: &'static bota_core::MapDef,
    pub players: Vec<Player>,
    pub units: Vec<KnownUnit>,
    pub talents: Vec<TalentView>,
    pub stock: Vec<StockView>,
    pub catalogue: bota_core::Catalogue,
    pub glyph_cooldown: Option<u32>,
    pub attacks: Vec<AttackFact>,
    pub action_impacts: Vec<bota_client_core::bot_actions::BotActionImpact>,
    pub item_uses: [Option<bota_client_core::ItemUseObservation>;2],
    pub item_refusals: [Option<bota_client_core::ItemUseRefusal>;2],
    pub projectiles: Vec<bota_proto::ProjectileView>,
}

impl Snapshot {
    pub(crate) fn capture(session: &ClientSession) -> Result<Self, BindingError> {
        let view = session
            .current()
            .ok_or(BindingError::Configuration("a current view is required"))?;
        for side in [Team::Radiant, Team::Dire] {
            crate::api::players::validate_team_size(
                view.players
                    .iter()
                    .filter(|player| player.team == side)
                    .count(),
            )?;
        }
        let players = view.players.iter().try_fold(0usize, |bytes, player| {
            bytes
                .checked_add(std::mem::size_of::<Player>())?
                .checked_add(
                    player
                        .stash
                        .as_ref()
                        .map_or(0, |slots| slots.len())
                        .checked_mul(std::mem::size_of::<Option<ItemView>>())?,
                )?
                .checked_add(
                    player
                        .kit
                        .as_ref()
                        .map_or(0, |kit| kit.abilities.len())
                        .checked_mul(std::mem::size_of::<AbilityView>())?,
                )?
                .checked_add(
                    player
                        .kit
                        .as_ref()
                        .map_or(0, |kit| kit.items.len())
                        .checked_mul(std::mem::size_of::<Option<ItemView>>())?,
                )
        });
        let players = players.and_then(|bytes| {
            bytes
                .checked_add(
                    view.talents
                        .len()
                        .checked_mul(std::mem::size_of::<TalentView>())?,
                )?
                .checked_add(
                    view.stock
                        .len()
                        .checked_mul(std::mem::size_of::<StockView>())?,
                )?
                .checked_add(
                    session
                        .recent_events()
                        .len()
                        .checked_mul(std::mem::size_of::<(u32, EntityId, EntityId)>())?,
                )?
                .checked_add(
                    session
                        .entities()
                        .len()
                        .checked_mul(std::mem::size_of::<AttackFact>())?,
                )
        });
        let players =
            players.and_then(|bytes| {
                bytes.checked_add(session.recent_events().len().checked_mul(
                    std::mem::size_of::<bota_client_core::bot_actions::BotActionImpact>(),
                )?)
            });
        let players = players.and_then(|bytes| bytes.checked_add(std::mem::size_of::<([Option<bota_client_core::ItemUseObservation>;2],[Option<bota_client_core::ItemUseRefusal>;2])>()));
        let players = players.and_then(|bytes| {
            bytes.checked_add(
                view.projectiles
                    .len()
                    .checked_mul(std::mem::size_of::<bota_proto::ProjectileView>())?,
            )
        });
        let bytes =
            session
                .entities()
                .iter()
                .try_fold(players.unwrap_or(usize::MAX), |bytes, track| {
                    let unit = &track.unit;
                    bytes
                        .checked_add(std::mem::size_of::<KnownUnit>())?
                        .checked_add(
                            unit.abilities
                                .len()
                                .checked_mul(std::mem::size_of::<bota_proto::AbilityView>())?,
                        )?
                        .checked_add(
                            unit.items
                                .len()
                                .checked_mul(std::mem::size_of::<Option<bota_proto::ItemView>>())?,
                        )?
                        .checked_add(
                            unit.effects
                                .len()
                                .checked_mul(std::mem::size_of::<bota_proto::EffectView>())?,
                        )?
                        .checked_add(
                            unit.queue
                                .len()
                                .checked_mul(std::mem::size_of::<bota_proto::Order>())?,
                        )
                });
        let bytes = bytes.and_then(|bytes| {
            bytes
                .checked_add(
                    view.felled_trees
                        .len()
                        .checked_mul(std::mem::size_of::<u32>())?,
                )?
                .checked_add(
                    view.planted_trees
                        .len()
                        .checked_mul(std::mem::size_of::<bota_proto::Vec2>())?,
                )
        });
        if session.recent_events().len() > 16384
            || session.entities().len() > MAX_HANDLES
            || view.players.len() > 10
            || bytes.is_none_or(|bytes| bytes > MAX_SNAPSHOT_BYTES)
        {
            return Err(BindingError::Configuration(
                "Lua snapshot storage limit exceeded",
            ));
        }
        Ok(Self {
            ended: session.ended(),
            statics: session.statics().clone(),
            felled_trees: view.felled_trees.clone(),
            planted_trees: view.planted_trees.clone(),
            team: session.team(),
            tick: view.tick,
            game_tick: view.game_tick(),
            day_tick: view.day_tick(),
            horn: session.map().pregame_ticks,
            map: session.map(),
            players: view
                .players
                .iter()
                .map(|player| {
                    let own = player.team == session.team();
                    let known = player.unit.and_then(|id| session.known(id));
                    Player {
                        slot: player.slot,
                        team: player.team,
                        hero: player.hero,
                        level: player.level,
                        deaths: player.deaths,
                        last_seen: known
                            .map(|(unit, age)| (view.tick.saturating_sub(age), unit.pos)),
                        statistics: if own {
                            known.map(|(unit, _)| UnitStatistics::from(unit))
                        } else {
                            None
                        },
                        unit: player.unit,
                        gold: own.then_some(player.gold).flatten(),
                        kit: own.then(|| player.kit.clone()).flatten(),
                        stash: own.then(|| player.stash.clone()).flatten(),
                        respawn_left: player.respawn_left,
                        buyback: own.then_some(player.buyback).flatten(),
                    }
                })
                .collect(),
            talents: view
                .talents
                .iter()
                .filter(|talents| {
                    view.players
                        .iter()
                        .any(|player| player.slot == talents.slot && player.team == session.team())
                })
                .copied()
                .collect(),
            stock: view.stock.clone(),
            catalogue: session.statics().catalogue(),
            glyph_cooldown: view
                .glyphs
                .iter()
                .find(|glyph| glyph.team == session.team())
                .and_then(|glyph| glyph.ready_after),
            attacks: attack_facts(session, view),
            item_uses: session.item_use_observations(),
            item_refusals: session.item_use_refusals(),
            action_impacts: session
                .recent_events()
                .iter()
                .filter_map(|event| {
                    bota_client_core::bot_actions::BotActionImpact::observed(
                        event.tick,
                        &event.kind,
                    )
                })
                .collect(),
            projectiles: view.projectiles.clone(),
            units: session
                .entities()
                .iter()
                .map(|track| KnownUnit {
                    unit: track.unit.clone(),
                    visible: view
                        .units
                        .binary_search_by_key(&track.unit.id, |unit| unit.id)
                        .is_ok(),
                    last_visible: (track.last_seen_tick, track.unit.pos),
                    velocity: track.velocity,
                    activity: track.activity,
                    recent_damage: track.recent_damage,
                    attack_target: track.attack_target(),
                    damage_dealt_at: track.last_damage_dealt.map(|damage| damage.tick),
                    direction_history: track.direction_history,
                })
                .collect(),
        })
    }
}

fn attack_facts(session: &ClientSession, view: &bota_proto::WorldView) -> Vec<AttackFact> {
    let owned = |source| {
        view.unit(source)
            .is_some_and(|unit| unit.team == session.team())
            || view
                .players
                .iter()
                .any(|player| player.team == session.team() && player.unit == Some(source))
    };
    let mut facts = Vec::with_capacity(session.entities().len());
    for track in session.entities() {
        if owned(track.id)
            && let Some(launch) = track.last_attack_launched
        {
            facts.push(AttackFact {
                tick: launch.tick,
                source: track.id,
                target: launch.target,
            });
        }
    }
    facts
}

#[derive(Default)]
pub(crate) struct BotState {
    pub last_chat: Option<u32>,
    pub target: Option<EntityId>,
    pub next_purchase: i32,
    pub mode: u8,
    pub desire: f64,
    pub pending_once: Option<PendingOnce>,
    pub native_courier: Option<NativeCourierCommand>,
    pub native_laning: Option<crate::default_bot::LaningMovement>,
    pub actions: [bota_client_core::bot_actions::BotActions; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeCourierCommand {
    pub unit: EntityId,
    pub ability: bota_proto::AbilityId,
    pub issued_at: u32,
}

/// Only a discarded native reply rewinds host continuations; Lua-visible state does not.
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
#[derive(Clone, Copy)]
pub(crate) struct HostCheckpoint {
    last_chat: Option<u32>,
    pending_once: Option<PendingOnce>,
    native_courier: Option<NativeCourierCommand>,
    native_laning: Option<crate::default_bot::LaningMovement>,
    actions: [bota_client_core::bot_actions::BotActions; 2],
}
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
impl BotState {
    pub fn checkpoint(&self) -> HostCheckpoint {
        HostCheckpoint {
            last_chat: self.last_chat,
            pending_once: self.pending_once,
            native_courier: self.native_courier,
            native_laning: self.native_laning,
            actions: self.actions,
        }
    }
    pub fn restore(&mut self, checkpoint: HostCheckpoint) {
        self.last_chat = checkpoint.last_chat;
        self.pending_once = checkpoint.pending_once;
        self.native_courier = checkpoint.native_courier;
        self.native_laning = checkpoint.native_laning;
        self.actions = checkpoint.actions;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    Top,
    Mid,
    Bot,
}
impl Lane {
    pub fn from_core(lane: u8) -> Option<Self> {
        match lane {
            bota_core::rules::LANE_MID => Some(Self::Mid),
            bota_core::rules::LANE_TOP => Some(Self::Top),
            bota_core::rules::LANE_BOT => Some(Self::Bot),
            _ => None,
        }
    }
    pub fn from_engine(lane: i32) -> Option<Self> {
        Self::from_core(match lane {
            1 => bota_core::rules::LANE_TOP,
            2 => bota_core::rules::LANE_MID,
            3 => bota_core::rules::LANE_BOT,
            _ => return None,
        })
    }
}

/// The decoder supplies engine TOP, MID, BOT order; consumers use a physical lane, never an index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LaneDesires([f64; 3]);
impl LaneDesires {
    pub fn from_engine_order(values: [f64; 3]) -> Self {
        Self(values)
    }
    pub fn get(self, lane: Lane) -> f64 {
        match lane {
            Lane::Top => self.0[0],
            Lane::Mid => self.0[1],
            Lane::Bot => self.0[2],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TeamDesires {
    pub push: Option<LaneDesires>,
    pub defend: Option<LaneDesires>,
}

pub(crate) struct State {
    pub activity_profiles:
        std::sync::Arc<std::collections::BTreeMap<String, bota_client_core::ActivityProfile>>,
    pub planted_tree_ids: Vec<bota_proto::Vec2>,

    pub statistics: Option<crate::statistics::SharedStatistics>,

    pub team: Team,
    pub seats: Vec<BotSeat>,
    pub declared_bots: [bool; 10],
    pub team_desires: TeamDesires,
    pub current_slot: SlotId,
    pub snapshot: Option<Snapshot>,
    pub sight: Option<bota_client_core::SightGround>,
    pub orders: Vec<IssuedOrder>,
    pub chat: Vec<bota_proto::Line>,
    pub units: Vec<(EntityId, TableId)>,
    unit_identities: crate::handles::Identities<EntityId>,
    pub unit_methods: Option<TableId>,
    pub host_purchase: Option<Value>,
    pub ability_methods: Option<TableId>,
    pub abilities: Vec<(super::api::abilities::Identity, TableId)>,
    pub ability_identities: crate::handles::Identities<super::api::abilities::Identity>,
    pub bots: Vec<BotState>,
    pub selecting: bool,
    pub selection_finished: bool,
    pub picks: Vec<Option<HeroId>>,
    pub purchases: Option<super::api::shop::Projection>,
}

impl State {
    pub fn new(team: Team, seats: &[BotSeat]) -> Self {
        Self {
            activity_profiles: std::sync::Arc::new(std::collections::BTreeMap::new()),
            team,
            statistics: None,
            planted_tree_ids: Vec::new(),
            seats: seats.to_vec(),
            declared_bots: [false; 10],
            team_desires: TeamDesires::default(),
            current_slot: seats[0].slot,
            snapshot: None,
            sight: None,
            orders: Vec::with_capacity(MAX_ORDERS),
            chat: Vec::new(),
            units: Vec::new(),
            unit_identities: crate::handles::Identities::new(),
            unit_methods: None,
            host_purchase: None,
            ability_methods: None,
            abilities: Vec::new(),
            ability_identities: crate::handles::Identities::new(),
            bots: (0..seats.len()).map(|_| BotState::default()).collect(),
            selecting: false,
            selection_finished: false,
            picks: vec![None; seats.len()],
            purchases: None,
        }
    }

    pub fn bot_index(&self) -> usize {
        self.seats
            .iter()
            .position(|seat| seat.slot == self.current_slot)
            .expect("active slot belongs to the team")
    }

    pub(crate) fn prepare_sight(&mut self, snapshot: &Snapshot) {
        bota_client_core::SightGround::refresh_forest(
            &mut self.sight,
            &snapshot.statics,
            &snapshot.felled_trees,
            &snapshot.planted_trees,
        );
    }

    pub fn own_id(&self) -> Option<EntityId> {
        self.snapshot
            .as_ref()?
            .players
            .iter()
            .find(|player| player.slot == self.current_slot)?
            .unit
    }

    pub fn unit(&self, id: EntityId) -> Option<&UnitView> {
        self.snapshot
            .as_ref()?
            .units
            .iter()
            .find(|known| known.unit.id == id)
            .map(|known| &known.unit)
    }

    pub fn player(&self, id: EntityId) -> Option<&Player> {
        self.snapshot
            .as_ref()?
            .players
            .iter()
            .find(|player| player.unit == Some(id))
    }

    /// A receiver this seat may ask about: a body or player of its own team, or any body in view.
    /// The reference refuses the rest ("cannot be seen"). O(units + players): callers have paid.
    pub fn askable(&self, id: EntityId) -> bool {
        let Some(snapshot) = &self.snapshot else {
            return false;
        };
        snapshot
            .units
            .iter()
            .any(|known| known.unit.id == id && (known.visible || known.unit.team == self.team))
            || snapshot
                .players
                .iter()
                .any(|player| player.unit == Some(id) && player.team == self.team)
    }

    /// Counts a call the reference rejects with a logged error, or one the seat cannot answer.
    /// `name` is `<API name>_<reason>`; unmeasured runs keep no statistics.
    pub fn decline(&self, name: &'static str) {
        if let Some(statistics) = &self.statistics {
            statistics
                .borrow_mut()
                .declined_api_calls
                .record(name.as_bytes());
        }
    }

    /// Last received own-hero body position survives respawn waits; foreign memory does not.
    pub fn position(&self, id: EntityId) -> Option<bota_proto::Vec2> {
        self.unit(id).map(|unit| unit.pos).or_else(|| {
            self.player(id)
                .filter(|player| player.team == self.team)
                .and_then(|player| player.last_seen.map(|(_, position)| position))
        })
    }

    pub fn hero(&self, id: EntityId) -> Option<HeroId> {
        self.player(id)
            .map(|player| player.hero)
            .or_else(|| self.unit(id)?.hero)
    }

    pub fn ability_book(&self, id: EntityId) -> Option<&[AbilityView]> {
        self.player(id)
            .and_then(|player| player.kit.as_ref())
            .map(|kit| kit.abilities.as_slice())
            .or_else(|| self.unit(id).map(|unit| unit.abilities.as_slice()))
    }

    pub fn item(&self, id: EntityId, slot: ItemSlot) -> Option<&ItemView> {
        let player = self.player(id);
        let kit = player.and_then(|player| player.kit.as_ref());
        let unit = self.unit(id);
        match usize::from(slot.0) {
            index @ 0..=8 => kit
                .map(|kit| kit.items.as_slice())
                .or_else(|| unit.map(|unit| unit.items.as_slice()))?
                .get(index)?
                .as_ref(),
            index @ 9..=14 => player?.stash.as_ref()?.get(index - 9)?.as_ref(),
            15 => kit
                .map(|kit| &kit.town_portal)
                .or_else(|| unit.map(|unit| &unit.town_portal))?
                .as_ref(),
            _ => None,
        }
    }

    pub fn id(&self, table: Value) -> Option<EntityId> {
        self.units
            .iter()
            .find(|(_, handle)| table == Value::Table(*handle))
            .map(|(id, _)| *id)
            .or_else(|| self.unit_identities.value(table))
    }

    pub fn handle(&mut self, vm: &mut Vm, id: Option<EntityId>) -> Result<Value, LuaError> {
        let _timer = crate::statistics::HandleTimer::new(&self.statistics);
        let Some(id) = id else { return Ok(Value::Nil) };
        vm.charge_fuel(1 + self.units.len() as u64 / 8)?;
        if let Some((_, table)) = self.units.iter().find(|(unit, _)| *unit == id) {
            return Ok(Value::Table(*table));
        }
        let tracked = self.tracked(id);
        if let Some(table) = self.unit_identities.table(vm, id)? {
            if tracked {
                if self.units.len() >= MAX_HANDLES {
                    return Err(vm.error("unit handle limit exceeded"));
                }
                vm.pin(Value::Table(table))?;
                self.units.push((id, table));
            }
            return Ok(Value::Table(table));
        }
        if !tracked {
            return Ok(Value::Nil);
        }
        if self.units.len() >= MAX_HANDLES {
            return Err(vm.error("unit handle limit exceeded"));
        }
        let table = vm.new_table()?;
        let meta = vm.new_table()?;
        let key = vm.new_string(b"__index")?;
        vm.raw_set(
            meta,
            key,
            self.unit_methods.map_or(Value::Nil, Value::Table),
        )?;
        let key = vm.new_string(b"__metatable")?;
        vm.raw_set(meta, key, Value::Boolean(false))?;
        vm.set_metatable(table, Some(meta))?;
        self.unit_identities.remember(vm, table, meta, id)?;
        vm.pin(Value::Table(table))?;
        self.units.push((id, table));
        Ok(Value::Table(table))
    }

    fn tracked(&self, id: EntityId) -> bool {
        self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .units
                .binary_search_by_key(&id, |known| known.unit.id)
                .is_ok()
                || snapshot
                    .players
                    .iter()
                    .any(|player| player.unit == Some(id))
        })
    }

    pub fn retire_handles(&mut self) -> Vec<TableId> {
        let mut retired = Vec::new();
        let mut index = 0;
        while index < self.units.len() {
            if self.tracked(self.units[index].0) {
                index += 1;
            } else {
                retired.push(self.units.swap_remove(index).1);
            }
        }
        let mut index = 0;
        while index < self.abilities.len() {
            if self.tracked(self.abilities[index].0.owner())
                && super::api::abilities::valid(self, self.abilities[index].0)
            {
                index += 1;
            } else {
                retired.push(self.abilities.swap_remove(index).1);
            }
        }
        retired
    }

    pub fn order(&mut self, vm: &mut Vm, order: IssuedOrder) -> Result<(), LuaError> {
        if self.orders.len() + self.pending_action_orders() >= MAX_ORDERS {
            return Err(vm.error("order limit exceeded"));
        }
        if order.unit.is_none() && !order.queue && self.breaks_once(order.order) {
            let index = self.bot_index();
            self.bots[index].pending_once = None;
            self.bots[index].native_laning = None;
        }
        let index = self.bot_index();
        if order.unit.is_some_and(|unit| {
            self.bots[index]
                .native_courier
                .is_some_and(|command| command.unit == unit)
        }) {
            self.bots[index].native_courier = None;
        }
        if !order.queue && self.breaks_once(order.order) {
            let body = usize::from(order.unit.is_some());
            let source = order.unit.or_else(|| self.own_id());
            let action = source
                .and_then(|source| self.unit(source))
                .and_then(|unit| {
                    bota_client_core::bot_actions::BotAction::ordinary(order.order, unit)
                });
            let tick = self.snapshot.as_ref().map_or(0, |snapshot| snapshot.tick);
            let controller = &mut self.bots[index].actions[body];
            controller.bind(source);
            if let Some(action) = action {
                controller.adopt(action, tick);
            } else {
                controller.clear(false);
            }
        }
        self.orders.push(order);
        Ok(())
    }

    fn breaks_once(&self, order: bota_proto::Order) -> bool {
        use bota_proto::Order;
        match order {
            Order::Move { .. } | Order::MoveDirection { .. } | Order::Attack { .. } => true,
            Order::Cast { slot, .. } => self
                .own_id()
                .and_then(|id| self.ability_book(id)?.get(usize::from(slot.0)))
                .and_then(|ability| bota_core::ability_def(ability.id))
                .is_none_or(|ability| !ability.immediate),
            Order::Use { slot, .. } => self
                .own_id()
                .and_then(|id| self.item(id, slot))
                .and_then(|item| bota_core::item_def(item.id))
                .is_none_or(|item| !item.immediate),
            _ => false,
        }
    }
}

impl bota_client_core::bot_actions::BotActionView for Snapshot {
    fn item_use_refusal(&self,id:EntityId)->Option<bota_client_core::ItemUseRefusal> {
        self.item_refusals.iter().flatten().copied().find(|fact|fact.source==id)
    }
    fn item_use_observation(&self, id: EntityId) -> Option<bota_client_core::ItemUseObservation> {
        self.item_uses.iter().flatten().copied().find(|fact|fact.source==id)
    }
    fn deaths(&self, id: EntityId) -> Option<u16> {
        self.players
            .iter()
            .find(|player| player.unit == Some(id))
            .map(|player| player.deaths)
    }
    fn tick(&self) -> u32 {
        self.tick
    }
    fn visible_unit(&self, id: EntityId) -> Option<&UnitView> {
        self.units
            .iter()
            .find(|known| known.visible && known.unit.id == id)
            .map(|known| &known.unit)
    }
    fn damage_dealt_at(&self, id: EntityId) -> Option<u32> {
        self.units
            .iter()
            .find(|known| known.unit.id == id)
            .and_then(|known| known.damage_dealt_at)
    }
    fn impacts(&self) -> impl Iterator<Item = bota_client_core::bot_actions::BotActionImpact> {
        self.action_impacts.iter().copied()
    }
}
impl State {
    pub(crate) fn controller_body(&self, id: EntityId) -> Option<usize> {
        if Some(id) == self.own_id() {
            return Some(0);
        }
        self.snapshot
            .as_ref()?
            .units
            .iter()
            .any(|known| {
                known.visible
                    && known.unit.id == id
                    && known.unit.kind == bota_proto::UnitKind::Courier
                    && known.unit.owner == Some(self.current_slot)
                    && known.unit.team == self.team
                    && known.unit.hp > 0
            })
            .then_some(1)
    }
    pub(crate) fn request_action(
        &mut self,
        vm: &mut Vm,
        id: EntityId,
        action: bota_client_core::bot_actions::BotAction,
        mode: bota_client_core::bot_actions::BotActionMode,
    ) -> Result<(), LuaError> {
        self.request_actions(vm, id, &[action], mode)
    }
    pub(crate) fn request_actions(
        &mut self,
        vm: &mut Vm,
        id: EntityId,
        actions: &[bota_client_core::bot_actions::BotAction],
        mode: bota_client_core::bot_actions::BotActionMode,
    ) -> Result<(), LuaError> {
        use bota_client_core::bot_actions::{BOT_ACTION_CAPACITY, BotActionMode};
        let units = self
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.units.len());
        // One roster lookup, a controller copy plus its possible shift/clear, and
        // one constant-time insertion per requested entry. The roster is not rescanned per slot.
        vm.charge_fuel(16 + units as u64 + 2 * BOT_ACTION_CAPACITY as u64 + actions.len() as u64)?;
        let body = self.controller_body(id).ok_or_else(|| {
            vm.error("action requires the active bot or its visible living courier")
        })?;
        if actions.len() > BOT_ACTION_CAPACITY {
            return Err(vm.error("bot action queue limit exceeded"));
        }
        let index = self.bot_index();
        let mut next = self.bots[index].actions[body];
        next.bind(Some(id));
        let mut changed = false;
        for (at, &action) in actions.iter().enumerate() {
            changed |= next
                .request(action, if at == 0 { mode } else { BotActionMode::Queue })
                .map_err(|_| vm.error("bot action queue limit exceeded"))?;
        }
        if !changed {
            return Ok(());
        }
        let pending = usize::from(next.needs_start())
            + usize::from(self.bots[index].actions[1 - body].needs_start());
        if self.orders.len() + pending > MAX_ORDERS {
            return Err(vm.error("order limit exceeded"));
        }
        self.bots[index].actions[body] = next;
        if body == 0 && mode != BotActionMode::Queue {
            self.bots[index].pending_once = None;
            self.bots[index].native_laning = None;
        }
        if body == 1 {
            self.bots[index].native_courier = None;
        }
        Ok(())
    }
    pub(crate) fn clear_actions(
        &mut self,
        vm: &mut Vm,
        id: EntityId,
        stop: bool,
    ) -> Result<(), LuaError> {
        vm.charge_fuel(
            16 + bota_client_core::bot_actions::BOT_ACTION_CAPACITY as u64
                + self
                    .snapshot
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.units.len()) as u64,
        )?;
        let body = self.controller_body(id).ok_or_else(|| {
            vm.error("action requires the active bot or its visible living courier")
        })?;
        let index = self.bot_index();
        let mut next = self.bots[index].actions[body];
        next.bind(Some(id));
        let order = next.clear(stop);
        if self.orders.len()
            + usize::from(order.is_some())
            + usize::from(self.bots[index].actions[1 - body].needs_start())
            > MAX_ORDERS
        {
            return Err(vm.error("order limit exceeded"));
        }
        self.bots[index].actions[body] = next;
        if body == 0 {
            self.bots[index].pending_once = None;
            self.bots[index].native_laning = None;
        }
        if body == 1 {
            self.bots[index].native_courier = None;
        }
        if let Some(order) = order {
            self.orders.push(IssuedOrder {
                unit: if body == 0 { None } else { Some(id) },
                order,
                queue: false,
            });
        }
        Ok(())
    }
    fn pending_action_orders(&self) -> usize {
        self.bots[self.bot_index()]
            .actions
            .iter()
            .filter(|controller| controller.needs_start())
            .count()
    }
    pub(crate) fn flush_actions(&mut self, index: usize, observe: bool) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let slot = self.seats[index].slot;
        let hero = snapshot
            .players
            .iter()
            .find(|player| player.slot == slot)
            .and_then(|player| player.unit);
        let courier = snapshot
            .units
            .iter()
            .find(|known| {
                known.visible
                    && known.unit.kind == bota_proto::UnitKind::Courier
                    && known.unit.owner == Some(slot)
            })
            .map(|known| known.unit.id);
        for (body, source) in [hero, courier].into_iter().enumerate() {
            let controller = &mut self.bots[index].actions[body];
            controller.bind(source);
            let order = if observe {
                controller.update(snapshot)
            } else {
                controller.start_ready(snapshot)
            };
            if let Some(order) = order {
                assert!(
                    self.orders.len() < MAX_ORDERS,
                    "controller publication reserved at request time"
                );
                self.orders.push(IssuedOrder {
                    unit: if body == 0 { None } else { source },
                    order,
                    queue: false,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod support {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs"));
    }

    #[test]
    fn the_opposing_snapshot_never_captures_private_inventory_or_gold() {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let item = bota_core::item_by_key("item_branches").expect("item");
        let buy = sessions[0]
            .issue(IssuedOrder::hero(bota_proto::Order::Buy { item }))
            .expect("buy");
        let streams = local.step(&[&[buy], &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
        let own = Snapshot::capture(&sessions[0]).expect("own snapshot");
        let enemy = Snapshot::capture(&sessions[1]).expect("opposing snapshot");
        let own_player = own
            .players
            .iter()
            .find(|player| player.slot == SlotId(0))
            .expect("own player");
        let hidden_player = enemy
            .players
            .iter()
            .find(|player| player.slot == SlotId(0))
            .expect("opposing player");
        assert!(own_player.gold.is_some());
        assert!(own_player.stash.is_some());
        assert_eq!(hidden_player.gold, None);
        assert!(hidden_player.stash.is_none());
        assert!(hidden_player.kit.is_none());
        assert!(
            !enemy
                .units
                .iter()
                .any(|known| Some(known.unit.id) == own_player.unit)
        );
    }

    #[test]
    fn last_visible_motion_freezes_when_an_enemy_walks_into_fog() {
        use bota_proto::{Order, Target, Vec2};
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let id = sessions[0]
            .own_player()
            .expect("player")
            .unit
            .expect("hero");
        let home = bota_core::fountain_pos(sessions[0].map(), Team::Radiant);
        let middle = Vec2::from_ints(9216, 9216);
        let mut last_seen = None;
        let mut hidden_ticks = 0;
        for _ in 0..2000 {
            let radiant = sessions[0]
                .issue(IssuedOrder::hero(Order::Move {
                    target: Target::Pos(if last_seen.is_some() { home } else { middle }),
                }))
                .expect("radiant order");
            let dire = sessions[1]
                .issue(IssuedOrder::hero(Order::Move {
                    target: Target::Pos(middle),
                }))
                .expect("dire order");
            let streams = local.step(&[&[radiant], &[dire]]).expect("step");
            for (session, stream) in sessions.iter_mut().zip(streams) {
                support::fold(session, stream);
            }
            let snapshot = Snapshot::capture(&sessions[1]).expect("opposing snapshot");
            let Some(known) = snapshot.units.iter().find(|known| known.unit.id == id) else {
                continue;
            };
            if known.visible {
                assert_eq!(known.last_visible, (snapshot.tick, known.unit.pos));
                last_seen = Some((known.last_visible, known.velocity));
            } else {
                assert_eq!(Some((known.last_visible, known.velocity)), last_seen);
                hidden_ticks += 1;
                if hidden_ticks == 10 {
                    break;
                }
            }
        }
        assert_eq!(
            hidden_ticks, 10,
            "the enemy was seen moving, then lost in fog"
        );
    }

    #[test]
    fn talent_capture_keeps_only_the_receiving_teams_talents() {
        let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
        for (index, slot) in [(0, SlotId(0)), (1, SlotId(1))] {
            let snapshot = Snapshot::capture(&sessions[index]).expect("snapshot");
            assert!(!snapshot.talents.is_empty());
            assert!(snapshot.talents.iter().all(|talents| talents.slot == slot));
        }
    }

    #[test]
    fn shop_capture_uses_only_the_announced_catalogue_and_received_stock() {
        let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
        for session in &sessions {
            let snapshot = Snapshot::capture(session).expect("snapshot");
            assert_eq!(snapshot.catalogue, session.statics().catalogue());
            assert_eq!(snapshot.stock, session.current().expect("view").stock);
        }
    }
}
