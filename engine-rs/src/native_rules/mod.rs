//! rules 域入口：生效规则以各助手的规则文件为准。
//!
//! - `rules:status`：逐目标（主 Claude/Codex + 各实例）输出实际全文、base 摘要与匹配模板，
//!   另附已保存模板与来源规则（技能来源里的 rules/*.md）；只读零写。
//! - `rules:apply`：写一个目标的实际文件（直接编辑 / 应用模板 / 应用来源规则 / 复制另一目标），
//!   base CAS、先回填再写，取 apply 锁。
//! - `rules:save-template` / `rules:delete-template`：只写库。

pub(crate) mod apply;
pub(crate) mod args;
pub(crate) mod model;
pub(crate) mod sources;
pub(crate) mod status;
pub(crate) mod template_ops;
pub(crate) mod templates;

pub(crate) use apply::preflight_apply;

use anyhow::{bail, Result};
use serde_json::Value;

use crate::native::{Context, Request};

/// rules 域主入口：先规范化 ./: 再分发；status 先严格解析参数。
pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let cmd = req.command.replace('.', ":");
    match cmd.as_str() {
        "rules:status" => {
            args::parse_status_args(req)?;
            status::build_status(ctx)
        }
        "rules:apply" => apply::apply(ctx, req),
        "rules:save-template" => template_ops::save(ctx, req),
        "rules:delete-template" => template_ops::delete(ctx, req),
        _ => bail!("rules 域不支持命令 '{}'", req.command),
    }
}
