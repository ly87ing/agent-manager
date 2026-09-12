//! hooks 域纯读 plan：先严格解析参数，再只读调用 status::build_status；
//! 不创建/修改 data_dir、home_dir、DB、表或目标文件。选中的 enabled 目标
//! 若 unmanaged-detected 或带 blocked_reason，整个 plan fail-closed。

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::{args, model, status};
use crate::native::{Context, Request};

fn action_for(element: &Value) -> Option<Value> {
    let state = element["state"].as_str().unwrap_or("");
    if state != "missing" && state != "drifted" {
        return None;
    }
    let observed = &element["observed"];
    let before_digest = if state == "missing" {
        Value::Null
    } else if let Some(applied) = observed["applied_digest"].as_str() {
        json!(applied)
    } else if let Some(actual) = observed["actual_digest"].as_str() {
        // 无 binding 的 drifted（含安全合并）：before 为当前实际字节摘要。
        json!(actual)
    } else {
        // 无 binding 的 registration-needed 场景：当前字节即期望字节。
        observed["expected_digest"].clone()
    };
    let id = element["id"].as_str().unwrap_or("");
    Some(json!({
        "op": "write-file",
        "element": id,
        "target": observed["target"],
        "risk": "write-local",
        "before_digest": before_digest,
        "after_digest": observed["expected_digest"],
        "backup_path": Value::Null,
        "cmd": Value::Null,
        "note": format!("同步 {id} 钩子文件"),
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
        let enabled = element["policy"]["enabled"].as_bool().unwrap_or(false);
        if enabled
            && (element["state"].as_str() == Some("unmanaged-detected")
                || !element["observed"]["blocked_reason"].is_null())
        {
            bail!("plan 拒绝: 选中的 hooks 目标存在阻塞，未生成任何动作");
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
        "domain": "hooks",
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
