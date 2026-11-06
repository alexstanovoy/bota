//! Native execution and source loading. The core binding uses neither threads nor filesystem access.

use std::path::Path;
mod worker;
pub use worker::{NativeConfig, NativeError, NativeTeam};

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

/// Independent entropy for hash keys and the script random stream.
pub fn random_keys() -> Result<crate::Keys, NativeError> {
    let mut entropy = [0u8; 24];
    getrandom::fill(&mut entropy).map_err(|_| NativeError::Setup("interpreter entropy is unavailable".to_owned()))?;
    let word = |index| u64::from_le_bytes(entropy[index..index + 8].try_into().expect("eight entropy bytes"));
    Ok(crate::Keys { hash: [word(0), word(8)], random: word(16) })
}
