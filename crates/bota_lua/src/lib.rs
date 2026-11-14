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
pub mod statistics;
mod team;
mod vector;

pub use luagee::{GcStress, Keys, Limits, Loader, ModuleName};

/// The outer locals one script function may capture in the host: the reference game's script runtime accepts up to 120
/// upvalues per function (game build 6975), and scripts written for it rely on that.
pub const SCRIPT_UPVALUES: u8 = 120;
/// The limits of a script host: the interpreter's defaults with the upvalue limit above.
pub const SCRIPT_LIMITS: Limits = Limits::DEFAULT.with_function_upvalues(SCRIPT_UPVALUES);
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
