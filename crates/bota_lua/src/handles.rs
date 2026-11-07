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
