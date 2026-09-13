//! Rust 原生 MCP 域（native-manager.db 自管域表，无旧 Python 迁移/兼容）。
//!
//! 模块结构：
//! - `model`：绑定模型、版本/写入摘要与定义清理
//! - `targets`：各 Agent 配置文件格式（Claude JSON / Codex TOML / OpenCode JSON）与原子 I/O
//! - `policy_view`：有效策略视图与凭证解析
//! - `persistence`：SQLite `mcp_bindings` 表读写
//! - `status`：状态查询实现（零写）
//! - `plan`：计划生成实现（零写）
//! - `apply`：预检与动作执行（加锁事务写）

pub mod apply;
pub mod codecs;
pub mod discovery;
pub mod model;
pub mod persistence;
pub mod plan;
pub mod policy_view;
pub mod preflight;
pub mod render;
pub mod rollback;
pub mod status;
pub mod targets;

use anyhow::{bail, Result};
use serde_json::Value;

pub use model::{
    clean_definition, compute_applied_digest, compute_definition_version, Binding, Bindings,
};
pub use preflight::preflight_apply;
pub use targets::{AgentTarget, Entry, Format};

use crate::native::{Context, Request};

/// MCP 域主入口：分发 status / plan / apply 命令
pub fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let cmd = req.command.replace('.', ":");
    match cmd.as_str() {
        "mcp:status" => {
            let mut offline = false;
            for arg in &req.args {
                if arg == "--offline" && !offline {
                    offline = true;
                } else {
                    bail!("mcp:status 仅接受可选的单个 --offline 参数，得到 '{}'", arg);
                }
            }
            status::status(ctx)
        }
        "mcp:plan" => plan::plan(ctx, req),
        "mcp:apply" => apply::apply(ctx, req),
        _ => bail!("mcp 域不支持命令 '{}'", req.command),
    }
}
