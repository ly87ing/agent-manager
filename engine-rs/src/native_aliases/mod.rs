//! aliases 域入口：生效的终端快捷命令以 ~/.zshrc 与 ~/.zprofile 受管区块里的实际行为准。
//!
//! - `aliases:status`：逐文件输出区块内的实际别名、区块外用户自己的别名（只读）与 base，
//!   以及按名合并的当前别名、内置推荐与旧版本保存的别名（未生效）；只读零写。
//! - `aliases:apply`：按名编辑区块里的一条（新增/改写/删除/添加推荐/加入旧版别名），
//!   每个文件独立 base CAS，取 apply 锁。
//! - `aliases:discard-legacy`：丢弃一条旧版本保存的别名，只写库。

pub(crate) mod apply;
pub(crate) mod args;
pub(crate) mod legacy;
pub(crate) mod model;
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
        "aliases:apply" => apply::apply(ctx, req),
        "aliases:discard-legacy" => legacy::discard(ctx, req),
        _ => bail!("aliases 域不支持命令 '{}'", req.command),
    }
}
