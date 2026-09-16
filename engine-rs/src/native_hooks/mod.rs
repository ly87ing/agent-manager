//! hooks 域入口：status/apply 的命令规范化与分发。

pub(crate) mod apply;
pub(crate) mod args;
pub(crate) mod model;
pub(crate) mod persistence;
pub(crate) mod preflight;
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
        _ => bail!("hooks 域不支持命令 '{}'", req.command),
    }
}
