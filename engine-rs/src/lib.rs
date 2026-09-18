//! agent-manager-engine library entry: the same native core backs the CLI
//! (src/main.rs) and the Tauri bridge (app/src-tauri) — no manage.sh, no
//! runtime Python, no resource-directory copying.
//!
//! Public surface (frozen by design review):
//! - `native::{Context, Request, execute}` — dispatcher entry point.
//! - `store` — SQLite (native-manager.db) connection helpers + policy access.
//! - `envelope` — Contract v1 envelope types.
//!
//! Resource domains not yet implemented return an explicit
//! `DOMAIN_NOT_IMPLEMENTED` error; this core never fabricates success.

pub(crate) mod agent_usage;
pub mod envelope;
pub mod native;
pub mod native_agents;
pub(crate) mod native_aliases;
pub(crate) mod native_config;
pub(crate) mod native_discover;
pub(crate) mod native_hooks;
pub mod native_logs;
pub mod native_mcp;
pub(crate) mod native_profiles;
pub(crate) mod native_rules;
pub(crate) mod native_runs;
pub(crate) mod native_secrets;
pub mod native_skills;
pub mod native_sources;
pub(crate) mod native_system;
pub(crate) mod native_upstream;
pub(crate) mod secret_values;
pub mod store;

pub use envelope::{diagnostic, Diagnostic, Envelope, Lock, Mode, CONTRACT_ID};
pub use native::{execute, Context, Request};
