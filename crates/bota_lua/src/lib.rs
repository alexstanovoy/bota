//! Lua bot bindings over filtered client sessions. The interpreter is provided by luagee.

#![forbid(unsafe_code)]

#[cfg(test)]
extern crate self as bota_lua;

mod api;
mod default_bot;
mod handles;
mod modules;
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub mod native;
mod numeric;
mod state;
mod team;
mod vector;

pub use luagee::{GcStress, Keys, Limits, Loader, ModuleName};
pub use modules::ModuleBank;
pub use team::*;

/// A setup or session contract error; script failures are reported per bot in `TeamTurn`.
#[derive(Debug)]
pub enum BindingError {
    Configuration(&'static str),
    Source { path: String, reason: String },
    Lua(luagee::Error),
    Callback(luagee::LuaError),
}

impl From<luagee::Error> for BindingError {
    fn from(error: luagee::Error) -> Self {
        Self::Lua(error)
    }
}

impl From<luagee::LuaError> for BindingError {
    fn from(error: luagee::LuaError) -> Self {
        Self::Callback(error)
    }
}

impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Configuration(message) => f.write_str(message),
            Self::Source { path, reason } => write!(f, "module source {path:?}: {reason}"),
            Self::Lua(error) => error.fmt(f),
            Self::Callback(error) => write!(f, "callback setup failed: {error:?}"),
        }
    }
}

impl std::error::Error for BindingError {}
