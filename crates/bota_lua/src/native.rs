//! Native execution and source loading. The core binding uses neither threads nor filesystem access.

use std::path::Path;

use crate::{BindingError, Limits, ModuleBank};

#[cfg(target_os = "linux")]
mod files;

/// Preloads regular Lua files into an immutable-ready bank. Only setup performs filesystem I/O.
/// Linux requires openat2 and trusted procfs; descendants may contain neither symlinks nor submounts.
pub fn load_modules(pack: &Path, runtime: Option<&Path>, limits: Limits) -> Result<ModuleBank, BindingError> {
    #[cfg(target_os = "linux")]
    { files::load(pack, runtime, limits) }
    #[cfg(not(target_os = "linux"))]
    { let _ = (pack, runtime, limits); Err(BindingError::Configuration("secure directory loading is unavailable on this platform")) }
}
