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
}

pub(crate) struct Snapshot {
    pub team: Team,
    pub tick: u32,
    pub game_tick: u32,
    pub horn: u32,
    pub map: &'static bota_core::MapDef,
    pub players: Vec<Player>,
    pub units: Vec<KnownUnit>,
    pub damage: Vec<(u32, EntityId, EntityId)>,
    pub talents: Vec<TalentView>,
    pub stock: Vec<StockView>,
    pub catalogue: bota_core::Catalogue,
    pub attacks: Vec<AttackFact>,
}

impl Snapshot {
    pub(crate) fn capture(session: &ClientSession) -> Result<Self, BindingError> {
        let view = session
            .current()
            .ok_or(BindingError::Configuration("a current view is required"))?;
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
            team: session.team(),
            tick: view.tick,
            game_tick: view.game_tick(),
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
                    }
                })
                .collect(),
            damage: session
                .recent_events()
                .iter()
                .filter_map(|event| match event.kind {
                    bota_proto::EventKind::Damaged {
                        source: Some(source),
                        target,
                        ..
                    } => Some((event.tick, source, target)),
                    _ => None,
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
            attacks: attack_facts(session, view),
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
    pub target: Option<EntityId>,
    pub next_purchase: i32,
    pub mode: u8,
    pub desire: f64,
    pub pending_once: Option<PendingOnce>,
    pub native_courier: Option<NativeCourierCommand>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeCourierCommand {
    pub unit: EntityId,
    pub ability: bota_proto::AbilityId,
    pub issued_at: u32,
}

/// Host action continuations rewind when their outbox was not accepted; Lua-visible state does not.
#[derive(Clone, Copy)]
pub(crate) struct HostCheckpoint {
    pending_once: Option<PendingOnce>,
    native_courier: Option<NativeCourierCommand>,
}
impl BotState {
    pub fn checkpoint(&self) -> HostCheckpoint {
        HostCheckpoint {
            pending_once: self.pending_once,
            native_courier: self.native_courier,
        }
    }
    pub fn restore(&mut self, checkpoint: HostCheckpoint) {
        self.pending_once = checkpoint.pending_once;
        self.native_courier = checkpoint.native_courier;
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
    pub team: Team,
    pub seats: Vec<BotSeat>,
    pub declared_bots: [bool; 10],
    pub team_desires: TeamDesires,
    pub current_slot: SlotId,
    pub snapshot: Option<Snapshot>,
    pub orders: Vec<IssuedOrder>,
    pub units: Vec<(EntityId, TableId)>,
    unit_identities: crate::handles::Identities<EntityId>,
    pub unit_methods: Option<TableId>,
    pub ability_methods: Option<TableId>,
    pub abilities: Vec<(super::api::abilities::Identity, TableId)>,
    pub ability_identities: crate::handles::Identities<super::api::abilities::Identity>,
    pub bots: Vec<BotState>,
    pub selecting: bool,
    pub picks: Vec<Option<HeroId>>,
    pub purchases: Option<super::api::shop::Projection>,
}

impl State {
    pub fn new(team: Team, seats: &[BotSeat]) -> Self {
        Self {
            team,
            seats: seats.to_vec(),
            declared_bots: [false; 10],
            team_desires: TeamDesires::default(),
            current_slot: seats[0].slot,
            snapshot: None,
            orders: Vec::with_capacity(MAX_ORDERS),
            units: Vec::new(),
            unit_identities: crate::handles::Identities::new(),
            unit_methods: None,
            ability_methods: None,
            abilities: Vec::new(),
            ability_identities: crate::handles::Identities::new(),
            bots: (0..seats.len()).map(|_| BotState::default()).collect(),
            selecting: false,
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
        if self.orders.len() >= MAX_ORDERS {
            return Err(vm.error("order limit exceeded"));
        }
        if order.unit.is_none() && !order.queue && self.breaks_once(order.order) {
            let index = self.bot_index();
            self.bots[index].pending_once = None;
        }
        let index = self.bot_index();
        if order.unit.is_some_and(|unit| {
            self.bots[index]
                .native_courier
                .is_some_and(|command| command.unit == unit)
        }) {
            self.bots[index].native_courier = None;
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
