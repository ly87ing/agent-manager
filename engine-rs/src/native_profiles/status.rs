//! profiles 域只读状态：每个实例定义一个元素。
//!
//! 零写：不创建配置目录、数据目录、数据库或目标文件；只做路径校验与字节读取。
//! 以实际文件为准：输出实例目标文件的写入基准与逐字段对比（定义 vs 实际）；凭据只给
//! 是否一致，绝不含取值或目标文件正文。

use anyhow::Result;
use serde_json::{json, Map, Value};

use super::model::{self, ProfileDef};
use crate::native::Context;
use crate::store;

/// 实例执行的命令：取基础助手定义里的 bin，取不到时退回实例类型的默认命令名。
pub(crate) fn base_bin(ctx: &Context, def: &ProfileDef) -> String {
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

/// 定义解析失败的固定语义块：只给类别与可读原因，不回显定义正文。带真实 id 与名称、所属助手等
/// 标识，界面才能列出它并提供编辑、移除（例如按新规则不再合法的旧定义）。
fn invalid_element(id: &str, raw: &Value, err: &anyhow::Error) -> Value {
    let profile = raw.get("profile");
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_string);
    json!({
        "id": format!("profiles:{id}"),
        "domain": "profiles",
        "spec": {
            "display_name": text(raw.get("display_name")).unwrap_or_else(|| id.to_string()),
            "base_agent": text(profile.and_then(|p| p.get("base_agent"))),
            "kind": text(profile.and_then(|p| p.get("kind"))),
            "config_dir": text(profile.and_then(|p| p.get("config_dir"))),
        },
        "policy": {"enabled": false},
        "observed": {"invalid": true, "blocked_reason": format!("{err:#}")},
        "state": "unsupported",
        "freshness": {"state": "not-applicable", "current": null, "upstream": null, "checked_at": null, "probe": null, "cache_age_sec": null},
        "actions": [],
    })
}

fn element(ctx: &Context, def: &ProfileDef) -> Value {
    let dir_check = model::validate_config_dir(def, &ctx.home_dir, &ctx.data_dir);
    let (config_dir, blocked) = match dir_check {
        Ok(path) => (Some(path), None),
        Err(err) => (None, Some(format!("{err:#}"))),
    };
    let dir_exists = config_dir.as_ref().map(|dir| dir.is_dir()).unwrap_or(false);

    // 实际文件为准：读实例配置目录里的目标文件（符号链接解析到最终文件），给出写入基准与
    // 逐字段对比；不再用「上次写入指纹」判定能不能写。
    let file = def.kind.target_file();
    let mut file_blocked: Option<String> = None;
    let mut bytes: Option<Vec<u8>> = None;
    if let Some(dir) = config_dir.as_ref() {
        match crate::native_agent_settings::resolve_write_target(&dir.join(file)) {
            Ok(real) => match std::fs::symlink_metadata(&real) {
                Ok(meta) if meta.is_file() => match std::fs::read(&real) {
                    Ok(read) => bytes = Some(read),
                    Err(err) => file_blocked = Some(format!("无法读取 {file}: {err}")),
                },
                Ok(_) => file_blocked = Some(format!("{file} 不是普通文件")),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => file_blocked = Some(format!("无法读取 {file}: {err}")),
            },
            Err(err) => file_blocked = Some(format!("{err:#}")),
        }
    }
    let base = if file_blocked.is_none() && config_dir.is_some() {
        match model::live_base(def.kind, bytes.as_deref()) {
            Ok(base) => Some(base),
            Err(err) => {
                file_blocked = Some(format!("{err:#}"));
                None
            }
        }
    } else {
        None
    };
    let fields = if file_blocked.is_none() && config_dir.is_some() {
        model::compare_fields(def, bytes.as_deref(), &crate::secret_values::lookup)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let any_present = bytes.is_some();
    let all_same = fields.iter().all(|f| f["state"] == "same");
    let file_state = if file_blocked.is_some() {
        "unsupported"
    } else if !any_present {
        "not-materialized"
    } else if all_same {
        "managed-ok"
    } else {
        "drifted"
    };
    let files = vec![json!({"file": file, "state": file_state})];

    // 环境变量只报变量名与当前环境是否给得出取值，绝不输出取值本身。
    let mut env_refs = Vec::new();
    for (name, value) in &def.env {
        let text = value.as_str().unwrap_or("");
        let var = text.strip_prefix("${").and_then(|v| v.strip_suffix('}'));
        env_refs.push(json!({
            "name": name,
            "kind": if var.is_some() { "reference" } else { "literal" },
            "variable": var,
            "resolved": var.map(crate::secret_values::is_set),
        }));
    }
    if let Some(key) = &def.env_key {
        env_refs.push(json!({
            "name": key,
            "kind": "reference",
            "variable": key,
            "resolved": crate::secret_values::is_set(key),
        }));
    }

    let state = if blocked.is_some() || file_blocked.is_some() {
        "unsupported"
    } else if !def.enabled {
        "disabled"
    } else {
        file_state
    };

    // 写入实例文件（用定义覆盖实际）与采纳实际配置（用实际更新定义）都要带 base；
    // 两个方向都由用户明确选择，不自动改写任一方。
    let mut actions = Vec::new();
    if blocked.is_none() && file_blocked.is_none() && def.enabled {
        actions.push(
            json!({"id": "apply", "risk": "write-local", "needs_lock": true, "offline_ok": true}),
        );
        // 采纳只搬定义声明的项；只剩隔离项不同时采纳改变不了什么，不提供
        let adoptable = fields
            .iter()
            .any(|f| f["state"] != "same" && f.get("purpose").is_none());
        if any_present && adoptable {
            actions.push(
                json!({"id": "adopt", "risk": "write-local", "needs_lock": false, "offline_ok": true}),
            );
        }
    }

    let mut observed = Map::new();
    observed.insert("base_agent".into(), json!(def.base_agent));
    observed.insert("kind".into(), json!(def.kind.as_str()));
    observed.insert("config_env".into(), json!(def.kind.config_env()));
    // 用第三方接口或凭据的实例：目录里不能有官方账号登录
    let third_party = model::is_third_party(def);
    observed.insert("third_party".into(), json!(third_party));
    // 实例目录挡不住的系统级设置（托管设置、Codex 的 /etc/codex 等）：只有来源与键名，只读
    let auth_kind = match def.kind {
        model::ProfileKind::Claude => crate::native_auth::AuthKind::Claude,
        model::ProfileKind::Codex => crate::native_auth::AuthKind::Codex,
    };
    observed.insert(
        "system_overrides".into(),
        Value::Array(crate::native_auth::instance_system_overrides(
            &ctx.home_dir,
            auth_kind,
            !third_party,
            bytes.as_deref(),
        )),
    );
    observed.insert("config_dir".into(), json!(def.config_dir_raw));
    observed.insert("config_dir_exists".into(), json!(dir_exists));
    observed.insert(
        "target".into(),
        json!(format!("{}/{}", def.config_dir_raw, def.kind.target_file())),
    );
    observed.insert("target_exists".into(), json!(any_present));
    // 实例多半是另一个账号，额度得从它自己的配置目录读，不能用默认那份
    observed.insert(
        "usage".into(),
        config_dir
            .as_deref()
            .and_then(|dir| crate::agent_usage::read_profile(def.kind.as_str(), dir, &ctx.data_dir))
            .unwrap_or(Value::Null),
    );
    observed.insert("files".into(), Value::Array(files));
    observed.insert("base".into(), json!(base));
    observed.insert("fields".into(), Value::Array(fields));
    observed.insert("env_refs".into(), Value::Array(env_refs));
    observed.insert(
        "launch_value".into(),
        json!(model::launch_value(def, base_bin(ctx, def).as_str(), "")),
    );
    observed.insert(
        "blocked_reason".into(),
        blocked
            .clone()
            .or(file_blocked.clone())
            .map(Value::String)
            .unwrap_or(Value::Null),
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
    let mut elements = Vec::new();
    for (id, raw, parsed) in model::load_definition_entries(&policy) {
        match parsed {
            Ok(def) => elements.push(element(ctx, &def)),
            Err(err) => elements.push(invalid_element(&id, &raw, &err)),
        }
    }
    Ok(json!({"domain": "profiles", "elements": elements}))
}
