//! Bounded source storage. Scripts see logical module names; loading performs no I/O.

use std::collections::BTreeMap;
use std::sync::Arc;

use luagee::{Limits, Loader, ModuleName};

use crate::BindingError;

const BANK_OVERHEAD: usize = 1024;
const ENTRY_OVERHEAD: usize = 2048;

/// Builds a module bank before installing it in a team. Installation makes the bank immutable.
/// `bots/` addresses pack modules, `game/` runtime modules, and bare names address the pack.
#[derive(Debug)]
pub struct ModuleBank {
    sources: BTreeMap<String, Vec<u8>>,
    activity_profiles: BTreeMap<String, bota_client_core::ActivityProfile>,
    limits: Limits,
    source_bytes: usize,
    max_source: usize,
    retained_bytes: usize,
}

impl ModuleBank {
    pub fn new(limits: Limits) -> Self {
        Self {
            sources: BTreeMap::new(),
            activity_profiles: BTreeMap::new(),
            limits,
            source_bytes: 0,
            max_source: 0,
            retained_bytes: BANK_OVERHEAD,
        }
    }

    /// Supplies trusted local model timing before scripts start. No script can change it.
    pub fn set_activity_profile(
        &mut self,
        unit: &str,
        profile: bota_client_core::ActivityProfile,
    ) -> Result<(), BindingError> {
        if unit.is_empty()
            || unit.len() > 256
            || !unit.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(BindingError::Configuration("invalid activity profile key"));
        }
        if self.activity_profiles.contains_key(unit) || self.activity_profiles.len() >= 1024 {
            return Err(BindingError::Configuration(
                "activity profile count limit exceeded",
            ));
        }
        let periods = [
            profile.idle_ms,
            profile.rare_idle_ms,
            profile.run_ms,
            profile.flail_ms,
            profile.stunned_ms,
            profile.die_ms,
            profile.spawn_ms,
        ];
        if periods
            .into_iter()
            .chain(profile.channel_ms)
            .flatten()
            .any(|ms| ms > 600_000)
        {
            return Err(BindingError::Configuration(
                "activity profile duration is out of range",
            ));
        }
        let bytes = self
            .retained_bytes
            .checked_add(ENTRY_OVERHEAD + unit.len())
            .ok_or(BindingError::Configuration(
                "activity profile storage limit exceeded",
            ))?;
        if bytes > self.limits.memory {
            return Err(BindingError::Configuration(
                "activity profile storage limit exceeded",
            ));
        }
        self.activity_profiles.insert(unit.to_owned(), profile);
        self.retained_bytes = bytes;
        Ok(())
    }

    /// Copies source only after checking its canonical name and every storage bound.
    pub fn insert(&mut self, name: &str, source: &[u8]) -> Result<(), BindingError> {
        let name: ModuleName = name
            .parse()
            .map_err(|_| BindingError::Configuration("invalid module name"))?;
        self.insert_key(key(&name), source)
    }

    #[cfg(feature = "native")]
    pub(crate) fn insert_file(
        &mut self,
        root: &str,
        relative: &str,
        source: &[u8],
    ) -> Result<(), BindingError> {
        let name: ModuleName = relative
            .parse()
            .map_err(|_| BindingError::Configuration("invalid module name"))?;
        self.insert_key(format!("{root}/{name}"), source)
    }

    fn insert_key(&mut self, key: String, source: &[u8]) -> Result<(), BindingError> {
        if self.sources.contains_key(&key) {
            return Err(BindingError::Configuration("duplicate module name"));
        }
        if self.sources.len() >= self.limits.module_count as usize {
            return Err(BindingError::Configuration("module count limit exceeded"));
        }
        if source.len() > self.limits.module_size {
            return Err(BindingError::Configuration("module source limit exceeded"));
        }
        let source_bytes =
            self.source_bytes
                .checked_add(source.len())
                .ok_or(BindingError::Configuration(
                    "total module source limit exceeded",
                ))?;
        if source_bytes > self.limits.scripts_size {
            return Err(BindingError::Configuration(
                "total module source limit exceeded",
            ));
        }
        let retained_bytes = self
            .retained_bytes
            .checked_add(ENTRY_OVERHEAD)
            .and_then(|bytes| bytes.checked_add(source.len()))
            .ok_or(BindingError::Configuration("module storage limit exceeded"))?;
        if retained_bytes > self.limits.memory {
            return Err(BindingError::Configuration("module storage limit exceeded"));
        }
        self.sources.insert(key, source.to_vec());
        self.source_bytes = source_bytes;
        self.retained_bytes = retained_bytes;
        self.max_source = self.max_source.max(source.len());
        Ok(())
    }

    pub(crate) fn freeze(self, limits: Limits) -> Result<Arc<Sources>, BindingError> {
        if self.sources.len() > limits.module_count as usize
            || self.source_bytes > limits.scripts_size
            || self.max_source > limits.module_size
        {
            return Err(BindingError::Configuration(
                "module bank exceeds team limits",
            ));
        }
        // A loader's returned Vec may stay live across nested module execution. Fund all possible simultaneous
        // copies, including the last load refused by a source/count limit, before scripts can invoke the loader.
        let by_bytes = limits.scripts_size.checked_add(self.max_source);
        let by_count = (limits.module_count as usize)
            .checked_add(1)
            .and_then(|count| count.checked_mul(self.max_source));
        let copies = by_bytes
            .zip(by_count)
            .map(|(bytes, count)| bytes.min(count))
            .ok_or(BindingError::Configuration("module copy budget overflow"))?;
        let retained_bytes = self
            .retained_bytes
            .checked_add(copies)
            .ok_or(BindingError::Configuration("module copy budget overflow"))?;
        if retained_bytes > limits.memory {
            return Err(BindingError::Configuration("module storage limit exceeded"));
        }
        Ok(Arc::new(Sources {
            sources: self.sources,
            activity_profiles: Arc::new(self.activity_profiles),
            retained_bytes,
        }))
    }
}

#[derive(Debug)]
pub(crate) struct Sources {
    sources: BTreeMap<String, Vec<u8>>,
    pub activity_profiles: Arc<BTreeMap<String, bota_client_core::ActivityProfile>>,
    pub retained_bytes: usize,
}

impl Sources {
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.sources.get(name).map(Vec::as_slice)
    }
}

pub(crate) struct MemoryLoader(pub Arc<Sources>);

impl Loader for MemoryLoader {
    fn load(&mut self, module: &ModuleName) -> Option<Vec<u8>> {
        self.0.get(&key(module)).map(<[u8]>::to_vec)
    }
}

fn key(module: &ModuleName) -> String {
    let name = module.to_string();
    if name.starts_with("bots/") || name.starts_with("game/") {
        name
    } else {
        format!("bots/{name}")
    }
}
