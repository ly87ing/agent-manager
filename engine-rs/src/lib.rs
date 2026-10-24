//! agent-manager-engine library entry: the same native core backs the CLI
//! (src/main.rs) and the Tauri bridge (app/src-tauri) — no manage.sh, no
//! runtime Python, no resource-directory copying.
//!
//! Public surface (frozen by design review):
//! - `native::{Context, Request, execute}` — dispatcher entry point.
//! - `store` — SQLite (native-manager.db) connection helpers + policy access.
//! - `envelope` — Contract v1 envelope types.
//! - `shutdown_temporary_commands` — desktop exit cleanup for owned commands.
//!
//! Resource domains not yet implemented return an explicit
//! `DOMAIN_NOT_IMPLEMENTED` error; this core never fabricates success.

pub mod data_dir;
pub mod desktop_links;
pub mod envelope;
pub mod native;
pub mod native_account_context;
mod native_account_locks;
pub(crate) mod native_agent_adapters;
pub mod native_agents;
pub(crate) mod native_claude_data;
pub(crate) mod native_config;
pub(crate) mod native_discover;
pub(crate) mod native_environment;
pub(crate) mod native_launch;
pub mod native_mcp;
pub(crate) mod native_profiles;
pub(crate) mod native_programs;
pub(crate) mod native_rules;
pub(crate) mod native_runs;
pub(crate) mod native_secrets;
pub mod native_skills;
pub(crate) mod native_system;
pub(crate) mod native_upstream;
pub mod platform;
pub(crate) mod process;
pub(crate) mod progress;
pub mod sanitize;
pub(crate) mod secret_values;
pub mod store;

pub use envelope::{diagnostic, Diagnostic, Envelope, Lock, Mode, CONTRACT_ID};
pub use native::{execute, Context, Request};
pub use process::shutdown_groups as shutdown_temporary_commands;

/// Restricted startup dispatch for the explicitly confirmed native clock helper.
pub fn environment_clock_helper_cli(args: &[String]) -> Option<i32> {
    native_environment::clock_helper_cli(args)
}

/// Use the packaged native sidecar for confirmed clock correction.
pub fn configure_clock_helper(path: std::path::PathBuf) {
    native_environment::configure_clock_helper(path);
}

/// Read-only source inspection in its own, non-elevated process.
pub fn claude_data_inspection_helper_cli(args: &[std::ffi::OsString]) -> i32 {
    native_claude_data::inspection_helper_cli(args)
}

/// Use the bundled inspector; None preserves a refused resource location.
pub fn configure_claude_data_inspection_helper(path: Option<std::path::PathBuf>) {
    native_claude_data::configure_inspection_helper(path);
}
