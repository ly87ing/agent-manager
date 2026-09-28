//! hooks 域入口：生效 hooks 以各助手的实际配置为准。
//!
//! - `hooks:status`：逐目标（主 Claude/Codex + 各实例）输出实际 hooks、base 摘要，
//!   以及清理建议与旧版本保存的 hooks（未生效）建议；只读零写。
//! - `hooks:apply`：写一个目标的实际文件（直接编辑 / 清理一条建议 / 加入一条旧版 hook），
//!   base CAS，取 apply 锁。
//! - `hooks:preview`：复用写入计划，只读展示目标文件的完整候选原文。
//! - `hooks:discard-legacy`：丢弃一条旧版本保存的 hook，只写库。

pub(crate) mod apply;
pub(crate) mod args;
pub(crate) mod legacy;
pub(crate) mod model;
pub(crate) mod status;

pub(crate) use apply::preflight_apply;

use anyhow::{bail, Result};
use serde_json::Value;

use crate::native::{Context, Request};

/// hooks 域主入口：先规范化 ./: 再分发；status 先严格解析参数。
pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let cmd = req.command.replace('.', ":");
    match cmd.as_str() {
        "hooks:status" => {
            args::parse_status_args(req)?;
            status::build_status(ctx)
        }
        "hooks:apply" => apply::apply(ctx, req),
        "hooks:preview" => apply::preview(ctx, req),
        "hooks:discard-legacy" => legacy::discard(ctx, req),
        _ => bail!("hooks 域不支持命令 '{}'", req.command),
    }
}
