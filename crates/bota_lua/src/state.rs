use bota_client_core::{ClientSession, IssuedOrder};
use bota_proto::{EntityId, HeroId, SlotId, Team, UnitView};
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
    pub unit: Option<EntityId>,
    pub gold: Option<i32>,
    pub respawn_left: u32,
}

pub(crate) struct KnownUnit { pub unit: UnitView, pub visible: bool }

pub(crate) struct Snapshot {
    pub team: Team,
    pub tick: u32,
    pub horn: u32,
    pub map: &'static bota_core::MapDef,
    pub players: Vec<Player>,
    pub units: Vec<KnownUnit>,
    pub damage: Vec<(u32, EntityId, EntityId)>,
}

impl Snapshot {
    pub(crate) fn capture(session: &ClientSession) -> Result<Self, BindingError> {
        let view = session.current().ok_or(BindingError::Configuration("a current view is required"))?;
        let bytes = session.entities().iter().try_fold(0usize, |bytes, track| {
            let unit = &track.unit;
            bytes.checked_add(std::mem::size_of::<KnownUnit>())?
                .checked_add(unit.abilities.len().checked_mul(std::mem::size_of::<bota_proto::AbilityView>())?)?
                .checked_add(unit.items.len().checked_mul(std::mem::size_of::<Option<bota_proto::ItemView>>())?)?
                .checked_add(unit.effects.len().checked_mul(std::mem::size_of::<bota_proto::EffectView>())?)?
                .checked_add(unit.queue.len().checked_mul(std::mem::size_of::<bota_proto::Order>())?)
        });
        if session.recent_events().len() > 16384 || session.entities().len() > MAX_HANDLES || view.players.len() > 10 || bytes.is_none_or(|bytes| bytes > MAX_SNAPSHOT_BYTES) {
            return Err(BindingError::Configuration("Lua snapshot storage limit exceeded"));
        }
        Ok(Self {
            team: session.team(), tick: view.tick, horn: session.map().pregame_ticks, map: session.map(),
            players: view.players.iter().map(|player| Player { slot: player.slot, team: player.team, hero: player.hero, unit: player.unit, gold: player.gold, respawn_left: player.respawn_left }).collect(),
            damage: session.recent_events().iter().filter_map(|event| match event.kind { bota_proto::EventKind::Damaged { source: Some(source), target, .. } => Some((event.tick, source, target)), _ => None }).collect(),
            units: session.entities().iter().map(|track| KnownUnit { unit: track.unit.clone(), visible: view.units.binary_search_by_key(&track.unit.id, |unit| unit.id).is_ok() }).collect(),
        })
    }
}

#[derive(Default)]
pub(crate) struct BotState { pub target: Option<EntityId>, pub next_purchase: i32, pub mode: u8, pub desire: f64 }

pub(crate) struct State {
    pub team: Team,
    pub seats: Vec<BotSeat>,
    pub current_slot: SlotId,
    pub snapshot: Option<Snapshot>,
    pub orders: Vec<IssuedOrder>,
    pub units: Vec<(EntityId, TableId)>,
    pub unit_methods: Option<TableId>,
    pub ability_methods: Option<TableId>,
    pub abilities: Vec<(super::api::abilities::Identity, TableId)>,
    pub bots: Vec<BotState>,
}

impl State {
    pub fn new(team: Team, seats: &[BotSeat]) -> Self {
        Self { team, seats: seats.to_vec(), current_slot: seats[0].slot, snapshot: None, orders: Vec::with_capacity(MAX_ORDERS), units: Vec::new(), unit_methods: None, ability_methods: None, abilities: Vec::new(), bots: (0..seats.len()).map(|_| BotState::default()).collect() }
    }

    pub fn bot_index(&self) -> usize { self.seats.iter().position(|seat| seat.slot == self.current_slot).expect("active slot belongs to the team") }

    pub fn own_id(&self) -> Option<EntityId> {
        self.snapshot.as_ref()?.players.iter().find(|player| player.slot == self.current_slot)?.unit
    }

    pub fn unit(&self, id: EntityId) -> Option<&UnitView> {
        self.snapshot.as_ref()?.units.iter().find(|known| known.unit.id == id).map(|known| &known.unit)
    }

    pub fn id(&self, table: Value) -> Option<EntityId> {
        self.units.iter().find(|(_, handle)| table == Value::Table(*handle)).map(|(id, _)| *id)
    }

    pub fn handle(&mut self, vm: &mut Vm, id: Option<EntityId>) -> Result<Value, LuaError> {
        let Some(id) = id else { return Ok(Value::Nil) };
        vm.charge_fuel(1 + self.units.len() as u64 / 8)?;
        if let Some((_, table)) = self.units.iter().find(|(unit, _)| *unit == id) { return Ok(Value::Table(*table)); }
        if self.units.len() >= MAX_HANDLES { return Err(vm.error("unit handle limit exceeded")); }
        let table = vm.new_table()?;
        let meta = vm.new_table()?;
        let key = vm.new_string(b"__index")?;
        vm.raw_set(meta, key, self.unit_methods.map_or(Value::Nil, Value::Table))?;
        let key = vm.new_string(b"__metatable")?;
        vm.raw_set(meta, key, Value::Boolean(false))?;
        vm.set_metatable(table, Some(meta))?;
        vm.pin(Value::Table(table))?;
        self.units.push((id, table));
        Ok(Value::Table(table))
    }

    pub fn order(&mut self, vm: &mut Vm, order: IssuedOrder) -> Result<(), LuaError> {
        if self.orders.len() >= MAX_ORDERS { return Err(vm.error("order limit exceeded")); }
        self.orders.push(order);
        Ok(())
    }
}
