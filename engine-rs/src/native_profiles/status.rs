//! profiles 域只读状态：实例定义 ∪ 已写入记录的全集视图。
//!
//! 零写：不创建配置目录、数据目录、数据库或目标文件；只做路径校验与字节读取。
//! 输出只含变量名与是否可解析，绝不含变量取值或目标文件正文。

use anyhow::Result;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

use super::model::{self, ProfileDef};
use super::persistence::{load_bindings_from, Binding};
use crate::native::Context;
use crate::native_mcp::model::sha256_hex_bytes;
use crate::store;

/// 实例执行的命令：取基础助手定义里的 bin，取不到时退回实例类型的默认命令名。
fn base_bin(ctx: &Context, def: &ProfileDef) -> String {
    let fallback = match def.kind {
        model::ProfileKind::Claude => "claude",
        model::ProfileKind::Codex => "codex",
    };
    store::effective_policy(ctx)
        .ok()
        .and_then(|policy| {
            policy
                .get("agents")?
                .get("definitions")?
                .get(&def.base_agent)?
                .get("bin")?
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| fallback.to_string())
}

fn bindings(ctx: &Context) -> Result<BTreeMap<String, Binding>> {
    match store::open_readonly(ctx)? {
        Some(conn) => load_bindings_from(&conn),
        None => Ok(BTreeMap::new()),
    }
}

/// 定义解析失败的固定语义块：只给类别与可读原因，不回显定义正文。
fn invalid_element(id: &str, err: &anyhow::Error) -> Value {
    json!({
        "id": format!("profiles:{id}"),
        "domain": "profiles",
        "spec": {},
        "policy": {"enabled": false},
        "observed": {"blocked_reason": format!("{err:#}")},
        "state": "unsupported",
        "freshness": {"state": "not-applicable", "current": null, "upstream": null, "checked_at": null, "probe": null, "cache_age_sec": null},
        "actions": [],
    })
}

fn element(ctx: &Context, def: &ProfileDef, binding: Option<&Binding>) -> Value {
    let dir_check = model::validate_config_dir(def, &ctx.home_dir, &ctx.data_dir);
    let (config_dir, blocked) = match dir_check {
        Ok(path) => (Some(path), None),
        Err(err) => (None, Some(format!("{err:#}"))),
    };
    let target_path = config_dir
        .as_ref()
        .map(|dir| dir.join(def.kind.target_file()));
    let dir_exists = config_dir.as_ref().map(|dir| dir.is_dir()).unwrap_or(false);
    let target_bytes = target_path
        .as_ref()
        .filter(|path| path.is_file())
        .and_then(|path| std::fs::read(path).ok());
    let actual_digest = target_bytes.as_ref().map(|bytes| sha256_hex_bytes(bytes));

    // 环境变量只报变量名与当前环境是否给得出取值，绝不输出取值本身。
    let mut env_refs = Vec::new();
    for (name, value) in &def.env {
        let text = value.as_str().unwrap_or("");
        let var = text.strip_prefix("${").and_then(|v| v.strip_suffix('}'));
        env_refs.push(json!({
            "name": name,
            "kind": if var.is_some() { "reference" } else { "literal" },
            "variable": var,
            "resolved": var.map(|v| std::env::var(v).is_ok()),
        }));
    }
    if let Some(key) = &def.env_key {
        env_refs.push(json!({
            "name": key,
            "kind": "reference",
            "variable": key,
            "resolved": std::env::var(key).is_ok(),
        }));
    }

    let state = if blocked.is_some() {
        "unsupported"
    } else if !def.enabled {
        "disabled"
    } else {
        match (&actual_digest, binding) {
            (None, _) => "not-materialized",
            (Some(actual), Some(b)) if *actual == b.applied_digest => "managed-ok",
            // 目标已存在但不是本工具最后写入的那份：外部改过或本工具从未写过
            (Some(_), _) => "drifted",
        }
    };

    let mut actions = Vec::new();
    if blocked.is_none() && def.enabled {
        actions.push(
            json!({"id": "apply", "risk": "write-local", "needs_lock": true, "offline_ok": true}),
        );
    }

    let mut observed = Map::new();
    observed.insert("base_agent".into(), json!(def.base_agent));
    observed.insert("kind".into(), json!(def.kind.as_str()));
    observed.insert("config_env".into(), json!(def.kind.config_env()));
    observed.insert("config_dir".into(), json!(def.config_dir_raw));
    observed.insert("config_dir_exists".into(), json!(dir_exists));
    observed.insert(
        "target".into(),
        json!(format!("{}/{}", def.config_dir_raw, def.kind.target_file())),
    );
    observed.insert("target_exists".into(), json!(actual_digest.is_some()));
    observed.insert("env_refs".into(), Value::Array(env_refs));
    // 建议的启动命令取值：前端可直接存成同名别名，与别名域的取值格式一致
    observed.insert(
        "launch_value".into(),
        json!(model::launch_value(def, base_bin(ctx, def).as_str(), "")),
    );
    observed.insert(
        "blocked_reason".into(),
        blocked.clone().map(Value::String).unwrap_or(Value::Null),
    );

    json!({
        "id": format!("profiles:{}", def.id),
        "domain": "profiles",
        "spec": {
            "display_name": def.display_name,
            "base_agent": def.base_agent,
            "kind": def.kind.as_str(),
            "config_dir": def.config_dir_raw,
            "base_url": def.base_url,
            "model": def.model,
        },
        "policy": {"enabled": def.enabled},
        "observed": Value::Object(observed),
        "state": state,
        "freshness": {"state": "not-applicable", "current": null, "upstream": null, "checked_at": null, "probe": null, "cache_age_sec": null},
        "actions": actions,
    })
}

pub(crate) fn build_status(ctx: &Context) -> Result<Value> {
    let policy = store::effective_policy(ctx)?;
    let bindings = bindings(ctx)?;
    let mut elements = Vec::new();
    for (index, parsed) in model::load_definitions(&policy).into_iter().enumerate() {
        match parsed {
            Ok(def) => {
                let binding = bindings.get(&def.id);
                elements.push(element(ctx, &def, binding));
            }
            Err(err) => elements.push(invalid_element(&format!("invalid-{index}"), &err)),
        }
    }
    Ok(json!({"domain": "profiles", "elements": elements}))
}
