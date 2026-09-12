//! aliases 域入口：status/plan/apply 的命令规范化与分发。

pub(crate) mod apply;
pub(crate) mod args;
pub(crate) mod model;
pub(crate) mod persistence;
pub(crate) mod plan;
pub(crate) mod status;

pub(crate) use apply::preflight_apply;

use anyhow::{bail, Result};
use serde_json::Value;

use crate::native::{Context, Request};

/// 域错误码：参数解析失败（UsageError）映射 USAGE_ERROR，其余 ALIAS_ERROR。
pub(crate) fn error_code(err: &anyhow::Error) -> &'static str {
    if err.downcast_ref::<args::UsageError>().is_some() {
        "USAGE_ERROR"
    } else {
        "ALIAS_ERROR"
    }
}

/// aliases 域主入口：先规范化 ./: 再分发；status 先严格解析参数。
pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let cmd = req.command.replace('.', ":");
    match cmd.as_str() {
        "aliases:status" => {
            args::parse_status_args(req)?;
            status::build_status(ctx)
        }
        "aliases:plan" => plan::build_plan(ctx, req),
        "aliases:apply" => apply::apply(ctx, req),
        _ => bail!("aliases 域不支持命令 '{}'", req.command),
    }
}
