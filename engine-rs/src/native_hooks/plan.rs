//! hooks 域纯读 plan：先严格解析参数，再只读调用 status::build_status；
//! 不创建/修改 data_dir、home_dir、DB、表或目标文件。选中的 enabled 目标
//! 若 unmanaged-detected 或带 blocked_reason，整个 plan fail-closed；
//! 唯一的例外是 --allow-takeover（仅 --only 单个目标）且目标文件本身可读、
//! 结构合法（actual_digest 存在）：claude 走既有 signature-set 合并，
//! codex whole-file 允许替换并标注 takeover。

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

/// 目标是否处于阻塞态（unmanaged-detected 或带 blocked_reason）。
fn element_blocked(element: &Value) -> bool {
    element["state"].as_str() == Some("unmanaged-detected")
        || !element["observed"]["blocked_reason"].is_null()
}

/// 阻塞目标是否可显式接管：--allow-takeover 已携带（调用方保证单目标），
/// 且 status 探测证明目标文件可读、JSON 与 hooks 子树结构合法
/// （actual_digest 存在）。结构损坏/路径不安全的目标依然 fail-closed。
fn takeover_eligible(element: &Value, allow_takeover: bool) -> bool {
    allow_takeover && element["observed"]["actual_digest"].is_string()
}

/// 显式接管动作：codex whole-file 替换标注 takeover 并在 note 明示整体替换
/// 风险；claude 走既有 signature-set 合并（保留用户字段与非本工具 hook），
/// 动作形态与常规同步一致。
fn takeover_action_for(element: &Value) -> Value {
    let observed = &element["observed"];
    let id = element["id"].as_str().unwrap_or("");
    let whole_file = element["spec"]["merge"].as_str() == Some("whole-file");
    let note = if whole_file {
        format!(
            "同步 {id} 钩子文件（whole-file 替换：目标文件现有内容将被整体替换，请先把需要保留的 hook 并入 hooks.codex 来源）"
        )
    } else {
        format!("同步 {id} 钩子文件")
    };
    let mut action = json!({
        "op": "write-file",
        "element": id,
        "target": observed["target"],
        "risk": "write-local",
        "before_digest": observed["actual_digest"],
        "after_digest": observed["expected_digest"],
        "backup_path": Value::Null,
        "cmd": Value::Null,
        "note": note,
    });
    if whole_file {
        action
            .as_object_mut()
            .expect("object")
            .insert("takeover".into(), Value::Bool(true));
    }
    action
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
        if enabled && element_blocked(element) && !takeover_eligible(element, parsed.allow_takeover)
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
            } else {
                let enabled = element["policy"]["enabled"].as_bool().unwrap_or(false);
                if enabled
                    && element_blocked(element)
                    && takeover_eligible(element, parsed.allow_takeover)
                {
                    actions.push(takeover_action_for(element));
                }
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
