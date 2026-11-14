//! Tree identities refer only to the forest the receiving client has been told about.
use super::{context::{ApiContext,register_unit},globals::bounded_integer};
use crate::BindingError;
use bota_proto::Fixed;
use luagee::{Args,Lua,LuaError,Value,Vm};
const MAX_PLANTED_IDENTITIES:usize=8192;
pub(super) fn install(lua:&mut Lua,context:&ApiContext)->Result<(),BindingError> {register_unit(lua,context,"GetNearbyTrees",nearby)}
fn nearby(context:&ApiContext,vm:&mut Vm,args:Args)->Result<usize,LuaError> {
    let id=context.receiver(vm,args)?;
    let radius=bounded_integer(vm,args,1,0,32767)?;
    let mut state=context.state.borrow_mut();
    let snapshot=state.snapshot.as_ref().ok_or_else(||vm.error("a current view is required"))?;
    let originals=snapshot.statics.trees();
    let lookup=1+snapshot.felled_trees.len().max(1).ilog2() as u64;
    vm.charge_fuel(8+snapshot.units.len() as u64+originals.len() as u64*(1+lookup)+snapshot.planted_trees.len() as u64)?;
    let owned=state.player(id).is_some_and(|player|player.team==state.team);
    let observed=snapshot.units.iter().any(|known|known.unit.id==id && known.visible);
    if !owned && !observed {return vm.results(&[Value::Nil]);}
    let position=state.position(id).ok_or_else(||vm.error("unit is no longer known"))?;
    let radius=Fixed::from_int(radius);
    let mut ids=Vec::new();
    for (index,&tree) in originals.iter().enumerate() {
        if snapshot.felled_trees.binary_search(&(index as u32)).is_err() && position.within(tree,radius) {ids.push(index as u32);}
    }
    let base=originals.len() as u32;
    let planted=snapshot.planted_trees.iter().copied().filter(|&tree|position.within(tree,radius)).collect::<Vec<_>>();
    for tree in planted {
        ids.push(base+planted_id(vm,&mut state,tree)?);

    }
    vm.charge_fuel(ids.len() as u64)?;
    let table=vm.new_table()?;
    for (at,id) in ids.into_iter().enumerate() {vm.raw_set(table,Value::Number((at+1) as f64),Value::Number(f64::from(id)))?;}
    vm.results(&[Value::Table(table)])
}

fn planted_id(vm:&mut Vm,state:&mut crate::state::State,tree:bota_proto::Vec2)->Result<u32,LuaError> {
    vm.charge_fuel(1+state.planted_tree_ids.len() as u64)?;
    if let Some(index)=state.planted_tree_ids.iter().position(|&known|known==tree) {return Ok(index as u32);}
    if state.planted_tree_ids.len()>=MAX_PLANTED_IDENTITIES {return Err(vm.error("planted tree identity limit exceeded"));}
    state.planted_tree_ids.push(tree);Ok((state.planted_tree_ids.len()-1) as u32)
}
