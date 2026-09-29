//! Rust 原生 profiles 域：同一个助手的多套独立配置（实例）。
//!
//! 一个实例 = 一个独立配置目录 + 一条启动命令。启动时用 `CLAUDE_CONFIG_DIR` /
//! `CODEX_HOME` 指向该目录，助手的设置、会话、凭据、技能与 MCP 全在那里，
//! 因此多个实例可以同时运行互不干扰，全局配置一个字节都不动。
//!
//! 实例定义就存在 `agents.definitions.<id>`（带 profile 段），所以技能与 MCP 的
//! 逐助手分配天然适用于实例，不另造一套分配模型。

pub(crate) mod apply;
pub(crate) mod model;
pub(crate) mod persistence;
pub(crate) mod preview_seed;
pub(crate) mod seed;
pub(crate) mod shared;
pub(crate) mod status;

pub(crate) use apply::preflight_apply;
pub(crate) use seed::preflight as preflight_seed;
pub(crate) use shared::preflight as preflight_shared;

use anyhow::{bail, Result};
use serde_json::Value;

use crate::native::{Context, Request};

pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let cmd = req.command.replace('.', ":");
    match cmd.as_str() {
        "profiles:status" => {
            for arg in &req.args {
                if arg != "--offline" {
                    bail!("profiles:status 仅接受可选的 --offline，得到 '{}'", arg);
                }
            }
            status::build_status(ctx)
        }
        "profiles:preview" => apply::preview(ctx, req),
        // apply：用实例定义写实例文件（取锁）；adopt：用实际文件更新实例定义（只写库）
        "profiles:apply" | "profiles:apply-definition" | "profiles:adopt" => apply::apply(ctx, req),
        // seed：把默认环境的权限、状态栏、hooks、规则与 MCP 一次性带进实例（取锁）
        "profiles:seed" => seed::seed(ctx, req),
        "profiles:shared-status" => shared::status(ctx, req),
        "profiles:shared-apply" => shared::apply(ctx, req),
        _ => bail!("profiles 域不支持命令 '{}'", req.command),
    }
}
