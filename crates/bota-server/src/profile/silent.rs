//! The phase profile compiled out: regions cost nothing and record nothing.

use std::marker::PhantomData;
use std::rc::Rc;

use crate::profile::Phase;

/// One region that records nothing.
#[must_use]
pub struct ScopeGuard {
    /// Keeps a guard on the thread it was made on, as the recording one is.
    _thread: PhantomData<Rc<()>>,
}

impl ScopeGuard {
    /// Starts a region that records nothing.
    pub fn new(_phase: Phase, _tick: u32, _entities: usize) -> Self {
        Self {
            _thread: PhantomData,
        }
    }
}
