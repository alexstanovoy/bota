//! The binding extension contract: checked engine identities, filtered state and ordinary orders.

use bota_client_core::IssuedOrder;
use bota_proto::{EntityId, Order, Target, UnitKind, Vec2};
use luagee::{Args, Lua, LuaError, TableId, Value, Vm};
use super::{Shared, method, global};
use crate::{BindingError, vector::Vectors};

#[derive(Clone)]
pub(crate) struct ApiContext { pub state: Shared, pub vectors: Vectors }
pub(crate) type ApiCallback = fn(&ApiContext, &mut Vm, Args) -> Result<usize, LuaError>;

impl ApiContext {
    pub fn receiver(&self, vm: &mut Vm, args: Args) -> Result<EntityId, LuaError> { self.unit_id(vm, args, 0) }

    /// Resolve a table by its host-maintained identity, never by script fields or a copied metatable.
    pub fn unit_id(&self, vm: &mut Vm, args: Args, index: usize) -> Result<EntityId, LuaError> {
        let state = self.state.borrow();
        vm.charge_fuel(1 + state.units.len() as u64 / 8 + state.snapshot.as_ref().map_or(0, |snapshot| snapshot.units.len()) as u64 / 8)?;
        state.id(vm.arg(args, index)).ok_or_else(|| vm.arg_error(args, index, "unit handle expected"))
    }

    /// Stable per-entity table, pinned and accounted by the host; absent identity becomes Lua nil.
    pub fn unit_value(&self, vm: &mut Vm, id: Option<EntityId>) -> Result<Value, LuaError> {
        self.state.borrow_mut().handle(vm, id)
    }

    /// Movement may also address the active bot's currently observed living courier.
    pub fn issue_move(&self, vm: &mut Vm, receiver: EntityId, position: Vec2) -> Result<(), LuaError> {
        let order = Order::Move { target: Target::Pos(position) };
        if Some(receiver) == self.state.borrow().own_id() { return self.issue_hero(vm, receiver, order); }
        let mut state = self.state.borrow_mut();
        vm.charge_fuel(2 + state.snapshot.as_ref().map_or(0, |snapshot| snapshot.units.len()) as u64 / 8)?;
        let permitted = state.snapshot.as_ref().is_some_and(|snapshot| snapshot.units.iter().any(|known| {
            known.unit.id == receiver && known.visible && known.unit.hp > 0 && known.unit.kind == UnitKind::Courier
                && known.unit.owner == Some(state.current_slot) && known.unit.team == state.team
        }));
        if !permitted { return Err(vm.error("movement requires the active bot or its visible living courier")); }
        state.order(vm, IssuedOrder::courier(receiver, order))
    }

    /// This is a pending intent. The scheduler publishes it only after the bot's complete turn succeeds.
    pub fn issue_hero(&self, vm: &mut Vm, receiver: EntityId, order: Order) -> Result<(), LuaError> {
        vm.charge_fuel(2)?;
        let mut state = self.state.borrow_mut();
        if Some(receiver) != state.own_id() { return Err(vm.error("action requires the active bot")); }
        state.order(vm, IssuedOrder::hero(order))
    }
}

pub(crate) fn register_global(lua: &mut Lua, context: &ApiContext, name: &'static str, callback: ApiCallback) -> Result<(), BindingError> {
    let context = context.clone();
    global(lua, name, std::rc::Rc::new(move |vm, args| callback(&context, vm, args)))
}

pub(crate) fn register_unit(lua: &mut Lua, context: &ApiContext, name: &'static str, callback: ApiCallback) -> Result<(), BindingError> {
    let table = context.state.borrow().unit_methods.ok_or(BindingError::Configuration("unit methods are not initialized"))?;
    register_method(lua, context, table, name, callback)
}

/// Also used for ability/item methods after their shared method table has been installed.
pub(crate) fn register_method(lua: &mut Lua, context: &ApiContext, table: TableId, name: &'static str, callback: ApiCallback) -> Result<(), BindingError> {
    let context = context.clone();
    method(lua, table, name, std::rc::Rc::new(move |vm, args| callback(&context, vm, args)))
}
