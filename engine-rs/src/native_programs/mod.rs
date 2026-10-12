//! 程序域：技能与 MCP 依赖的命令行程序的生命周期（docs/PROGRAM-LIFECYCLE.md）。
//! 智能体本身（Claude Code、Codex）不在范围内。

pub(crate) mod apply;
pub(crate) mod declare;
pub(crate) mod locate;
mod scope;
pub(crate) mod status;

use anyhow::{bail, Result};
use serde_json::Value;

use crate::native::{Context, Request};

pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let verb = req.command.split([':', '.']).nth(1).unwrap_or_default();
    match verb {
        "status" => {
            for arg in &req.args {
                if arg != "--rescan" && arg != "--json" {
                    bail!("programs:status 不支持参数 '{arg}'");
                }
            }
            status::status(ctx, req.args.iter().any(|a| a == "--rescan"))
        }
        "preview" => Ok(apply::plan(ctx, req)?.to_json(ctx)),
        "apply" => apply::execute(ctx, req),
        "declare" => apply::declare(ctx, req),
        other => bail!("programs 域没有动词 '{other}'"),
    }
}
