//! Lua owns historical identities; host indexes never keep their tables alive.
use std::{cell::RefCell, collections::HashMap, hash::Hash, rc::{Rc, Weak}};
use luagee::{LuaError, TableId, Value, Vm};

// Covers both indexes at their growth/shrink high-water marks and the Rc allocation.
// The small empty-map capacity is covered by the fixed host storage reserve.
const IDENTITY_STORAGE_BYTES: usize = 1024;

type Witness<I> = Weak<Identity<I>>;
struct Registry<I: Copy + Eq + Hash> {
    by_value: HashMap<I, Witness<I>>,
    by_table: HashMap<TableId, Witness<I>>,
}
struct Identity<I: Copy + Eq + Hash> {
    table: TableId,
    value: I,
    registry: Weak<RefCell<Registry<I>>>,
}
impl<I: Copy + Eq + Hash> Drop for Identity<I> {
    fn drop(&mut self) {
        let Some(registry) = self.registry.upgrade() else { return };
        // Host allocation never collects or runs Lua. A witness is destroyed only at a
        // collector safe point, where no binding callback can hold an index borrow.
        let mut registry = registry.borrow_mut();
        if registry.by_value.get(&self.value).is_some_and(|known| std::ptr::eq(known.as_ptr(), self)) {
            registry.by_value.remove(&self.value);
        }
        registry.by_table.remove(&self.table);
        compact(&mut registry.by_value);
        compact(&mut registry.by_table);
    }
}
fn compact<K: Eq + Hash, V>(index: &mut HashMap<K, V>) {
    if index.capacity() > 16 && index.len().saturating_mul(4) < index.capacity() {
        index.shrink_to(index.len().saturating_mul(2).max(4));
    }
}

pub(crate) struct Identities<I: Copy + Eq + Hash> { registry: Rc<RefCell<Registry<I>>> }
impl<I: Copy + Eq + Hash + 'static> Identities<I> {
    pub fn new() -> Self { Self { registry: Rc::new(RefCell::new(Registry { by_value: HashMap::new(), by_table: HashMap::new() })) } }

    pub fn value(&self, value: Value) -> Option<I> {
        let Value::Table(table) = value else { return None };
        self.registry.borrow().by_table.get(&table)?.upgrade().filter(|identity| identity.table == table).map(|identity| identity.value)
    }

    pub fn table(&self, vm: &mut Vm, value: I) -> Result<Option<TableId>, LuaError> {
        let identity = self.registry.borrow().by_value.get(&value).and_then(Weak::upgrade);
        let Some(identity) = identity else { return Ok(None) };
        // A table ID in host data is not a traced Lua reference. Validate it before reuse.
        match vm.raw_get(identity.table, Value::Nil) {
            Ok(_) => Ok(Some(identity.table)),
            Err(error @ LuaError::Limit(_)) => Err(error),
            Err(_) => Ok(None),
        }
    }

    pub fn remember(&mut self, vm: &mut Vm, table: TableId, meta: TableId, value: I) -> Result<(), LuaError> {
        let identity = Rc::new(Identity { table, value, registry: Rc::downgrade(&self.registry) });
        let witness = vm.userdata(identity.clone(), None, IDENTITY_STORAGE_BYTES)?;
        let key = vm.new_string(b"__identity")?;
        vm.raw_set(meta, key, witness)?;
        let mut registry = self.registry.borrow_mut();
        registry.by_value.insert(value, Rc::downgrade(&identity));
        registry.by_table.insert(table, Rc::downgrade(&identity));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luagee::{Error, Keys, Limit, Limits, Lua};

    mod support { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs")); }

    #[test]
    fn fuel_failure_cannot_publish_a_handle_without_its_metatable() {
        use crate::{api, state::{Snapshot, State}, vector};
        use bota_proto::{HeroId, SlotId, Team};
        for ability in [false, true] {
            for budget in 8..160 {
                let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
                let state = Rc::new(RefCell::new(State::new(Team::Radiant, &[crate::BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }])));
                state.borrow_mut().snapshot = Some(Snapshot::capture(&sessions[0]).expect("snapshot"));
                let mut lua = Lua::new(Limits::DEFAULT, Keys { hash: [17,19], random:23 }).expect("vm");
                let vectors = vector::install(&mut lua).expect("vectors");
                api::install(&mut lua, state, vectors).expect("API");
                let drain = lua.function("drain", Rc::new(|vm,args| {
                    let amount = vm.check_integer(args,0)?;
                    vm.charge_fuel(amount as u64)?;
                    Ok(0)
                })).expect("drain callback");
                lua.set_global("drain",drain).expect("global");
                if ability { lua.exec("prime", b"GetBot()").expect("prime unit handle"); }
                lua.set_gc_stress(luagee::GcStress::Off);
                let expression = if ability { "GetBot():GetAbilityByName('nevermore_shadowraze1'):GetName()" } else { "GetBot():GetHealth()" };
                let source = format!("drain({});return {}", Limits::DEFAULT.fuel_per_call-budget, expression);
                let first = lua.exec("limited",source.as_bytes());
                if let Err(error) = first { assert!(matches!(error, Error::Limit(Limit::FuelPerCall)), "budget={budget}, ability={ability}: {error}"); }
                let recovered = lua.exec("recover",format!("return {expression}").as_bytes())
                    .unwrap_or_else(|error| panic!("budget={budget}, ability={ability}: {error}"));
                if ability { assert_eq!(lua.bytes(recovered[0]), Some(b"nevermore_shadowraze1".as_slice())); }
                else { assert!(matches!(recovered[0], Value::Number(value) if value>0.0)); }
            }
        }
    }

    #[test]
    fn witness_allocation_failure_drops_host_data_without_borrowing_an_index() {
        let mut lua = Lua::new(Limits { memory: 1 << 20, ..Limits::DEFAULT }, Keys { hash: [17,19], random:23 }).expect("vm");
        let table = lua.vm().new_table().expect("table");
        let meta = lua.vm().new_table().expect("metatable");
        lua.pin(Value::Table(table)).expect("root table");
        lua.pin(Value::Table(meta)).expect("root metatable");
        // Trusted setup allocations do not collect, so consume their bounded headroom.
        for _ in 0..2048 {
            if lua.vm().userdata((), None, 1024).is_err() { break; }
        }
        let mut exhausted = false;
        for _ in 0..2048 {
            match lua.vm().userdata((), None, 0) {
                Ok(_) => {},
                Err(LuaError::Limit(Limit::Memory)) => { exhausted = true; break; },
                Err(error) => panic!("unexpected setup error: {error:?}"),
            }
        }
        assert!(exhausted, "the setup allocation headroom was consumed");
        let mut identities = Identities::new();
        let error = identities.remember(lua.vm(), table, meta, 7u32).expect_err("witness cannot fit");
        assert!(matches!(error, LuaError::Limit(Limit::Memory)));
        assert_eq!(identities.value(Value::Table(table)), None);
        assert_eq!(identities.table(lua.vm(), 7).expect("empty identity index"), None);
    }
}
