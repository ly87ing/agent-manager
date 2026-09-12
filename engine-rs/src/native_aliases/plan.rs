//! aliases 域纯读 plan：先严格解析参数，再只读调用 status::build_status；
//! 不创建/修改 data_dir、home_dir、DB、表或目标文件。选中的目标若带
//! blocked_reason（外部修改/路径不安全/不可读），整个 plan fail-closed。
//! before/after 均为受管 marker 区块内容的 sha256（无区块/空期望为 null）。

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::{args, model, status};
use crate::native::{Context, Request};

fn action_for(element: &Value) -> Option<Value> {
    let state = element["state"].as_str().unwrap_or("");
    if !matches!(state, "drifted" | "missing" | "not-materialized") {
        return None;
    }
    let observed = &element["observed"];
    let before_digest = if observed["block_present"].as_bool() == Some(true) {
        // 区块存在：before 为当前区块内容 sha256（drifted 才有此组合）。
        observed["actual_block_digest"].clone()
    } else {
        Value::Null
    };
    let id = element["id"].as_str().unwrap_or("");
    let after_digest = if observed["alias_count"].as_u64().unwrap_or(0) == 0 {
        // 期望区块为空：收敛即移除区块，after 为 null。
        Value::Null
    } else {
        observed["expected_digest"].clone()
    };
    Some(json!({
        "op": "write-file",
        "element": id,
        "target": observed["target"],
        "risk": "write-local",
        "before_digest": before_digest,
        "after_digest": after_digest,
        "backup_path": Value::Null,
        "cmd": Value::Null,
        "note": format!("同步 {id} 受管 alias 区块；marker 区块外内容不动"),
    }))
}

pub(crate) fn build_plan(ctx: &Context, req: &Request) -> Result<Value> {
    let parsed = args::parse_plan_args(req)?;
    let status = status::build_status(ctx)?;

    let empty = Vec::new();
    let elements = status["elements"].as_array().unwrap_or(&empty);

    for id in &parsed.only {
        let Some(element) = elements.iter().find(|e| e["id"].as_str() == Some(id)) else {
            continue;
        };
        if !element["observed"]["blocked_reason"].is_null() {
            bail!("plan 拒绝: 选中的 aliases 目标存在阻塞，未生成任何动作");
        }
    }

    let mut actions = Vec::new();
    for spec in model::targets() {
        if !parsed.only.iter().any(|id| id == spec.target_id) {
            continue;
        }
        if let Some(element) = elements
            .iter()
            .find(|e| e["id"].as_str() == Some(spec.target_id))
        {
            if let Some(action) = action_for(element) {
                actions.push(action);
            }
        }
    }

    let write_count = actions.len();
    Ok(json!({
        "domain": "aliases",
        "actions": actions,
        "summary": {
            "read": 0,
            "write-local": write_count,
            "exec-local": 0,
            "network": 0,
            "install-remove": 0,
        },
    }))
}
