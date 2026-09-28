//! Rust 原生 MCP 域：各助手配置文件里的实际条目即当前配置。
//!
//! 模块结构：
//! - `targets`：各助手的 MCP 目标文件、格式与路径安全校验
//! - `codecs`：配置文件解析与 Codex TOML 文本渲染
//! - `entry`：条目的已建模视图、脱敏展示与写入变换
//! - `inventory`：实际条目 + 已保存项的只读清单
//! - `saved`：「已保存未生效」项（`mcp_saved` 表：停用快照、未分配定义）
//! - `snapshot`：Claude 停用快照的生成与写回（疑似凭据转存外部密钥文件）
//! - `legacy`：旧服务库定义 + `mcp_bindings` 的迁移（读路径推导、写路径落库）
//! - `write`：单条目写入的新字节计算与写前自检
//! - `status` / `preflight` / `apply`：只读状态、写入计划、执行
//! - `policy_view`：定义形态校验（policy:set / config:import / 已保存定义）

pub mod apply;
pub mod codecs;
pub mod entry;
pub mod inventory;
pub mod legacy;
pub mod model;
pub mod policy_view;
pub mod preflight;
pub mod rollback;
pub mod saved;
pub mod snapshot;
pub mod status;
pub mod targets;
pub mod write;

use anyhow::{bail, Result};
use serde_json::{json, Value};

pub use preflight::preflight_apply;
pub use targets::{AgentTarget, Entry, Format};

use crate::native::{Context, Request};

/// MCP 域主入口：返回 (data, warnings)。
pub fn dispatch(ctx: &Context, req: &Request) -> Result<(Value, Vec<String>)> {
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
            Ok((status::status(ctx)?, Vec::new()))
        }
        "mcp:preview" => {
            let parsed = preflight::parse_action_args(req)?;
            if !matches!(
                parsed.action,
                preflight::Action::Add | preflight::Action::Edit
            ) {
                bail!("mcp:preview 仅支持 add/edit");
            }
            let inv = inventory::load(ctx)?;
            let plan = preflight::plan(&parsed, &inv)?;
            let files = plan
                .file_writes
                .iter()
                .map(|write| {
                    if write.new_bytes.len() > 4 * 1024 * 1024 {
                        bail!("目标文件超过 4 MiB，不能在界面预览");
                    }
                    Ok(json!({
                        "path": write.path,
                        "exists": write.old_bytes.is_some(),
                        "content": std::str::from_utf8(&write.new_bytes)?,
                    }))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((json!({ "files": files }), plan.warnings))
        }
        "mcp:apply" => apply::apply(ctx, req),
        _ => bail!("mcp 域不支持命令 '{}'", req.command),
    }
}

/// 只要 data 的调用方（system:audit）。
pub fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    dispatch(ctx, req).map(|(data, _)| data)
}
