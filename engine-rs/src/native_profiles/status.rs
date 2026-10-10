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

/// 定义的写入基准：只输出摘要，候选更新在锁内重新比对原定义。
pub(crate) fn definition_base(raw: &Value) -> Result<String> {
    Ok(crate::native_mcp::model::sha256_hex_bytes(
        &serde_json::to_vec(raw)?,
    ))
}

/// 编辑预填复用实际配置的采纳逻辑，但不写登记。存在无法采纳的字段时不以旧值代填。
fn editable_profile(def: &ProfileDef, raw: &Value, bytes: Option<&[u8]>) -> Result<Value> {
    let original = raw["profile"].as_object().expect("validated profile");
    let profile = if bytes.is_some() {
        let (profile, notes) = model::adopt_profile(def, original, bytes, &|name| {
            crate::secret_values::lookup_account(name).ok()
        })?;
        if !notes.is_empty() {
            anyhow::bail!("{}", notes.join("；"));
        }
        profile
    } else {
        original.clone()
    };
    let mut candidate = raw.clone();
    candidate["profile"] = Value::Object(profile.clone());
    model::parse_definition(&def.id, &candidate, def.enabled)?;
    if profile
        .get("env")
        .and_then(Value::as_object)
        .is_some_and(|env| {
            env.iter().any(|(key, value)| {
                key.to_ascii_uppercase().contains("HEADER")
                    && value
                        .as_str()
                        .is_some_and(|text| !(text.starts_with("${") && text.ends_with('}')))
            })
        })
    {
        anyhow::bail!("实际配置包含请求头取值，不能自动预填接入参数");
    }
    let keys: &[&str] = match def.kind {
        model::ProfileKind::Claude => &["env", "model"],
        model::ProfileKind::Codex => &["base_url", "env_key", "wire_api", "model"],
    };
    let editable: Map<String, Value> = keys
        .iter()
        .filter_map(|key| {
            profile
                .get(*key)
                .map(|value| ((*key).to_string(), value.clone()))
        })
        .collect();
    // 实际文件中的普通字段也可能被误填成密钥；不把这样的内容带进表单或登记。
    if crate::native::reject_secret_text(&serde_json::to_string(&editable)?).is_some() {
        anyhow::bail!("实际配置包含疑似凭据的普通字段，不能自动预填接入参数");
    }
    Ok(Value::Object(editable))
}

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

fn element(ctx: &Context, def: &ProfileDef, raw: &Value) -> Value {
    let dir_check = super::paths::config_dir(ctx, def);
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
        match super::paths::target(ctx, def, &dir.join(file)) {
            Ok(real) => match model::read_account_file(&real) {
                Ok(read) => bytes = read,
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
    if file_blocked.is_none() {
        let auth = (|| -> Result<()> {
            model::validate_secret_ownership(ctx, def)?;
            if let Some(dir) = config_dir.as_deref() {
                model::validate_local_auth(
                    &ctx.home_dir,
                    def.kind,
                    Some(dir),
                    model::account_auth_mode(def),
                )?;
            }
            if let Some(bytes) = bytes.as_deref() {
                model::validate_auth_shape(def, bytes)?;
            }
            Ok(())
        })();
        if let Err(err) = auth {
            file_blocked = Some(format!("{err:#}"));
        }
    }
    let fields = if file_blocked.is_none() && config_dir.is_some() {
        model::compare_fields(def, bytes.as_deref(), &|name| {
            crate::secret_values::lookup_account(name).ok()
        })
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
    if base.is_some() {
        if let Ok(connection) = model::live_connection(def.kind, bytes.as_deref()) {
            observed.insert("current_connection".into(), connection);
        }
    }
    observed.insert("definition_base".into(), json!(definition_base(raw).ok()));
    let editing = if let Some(reason) = blocked.as_ref().or(file_blocked.as_ref()) {
        Err(anyhow::anyhow!("{reason}"))
    } else {
        editable_profile(def, raw, bytes.as_deref())
    };
    match editing {
        Ok(profile) => {
            observed.insert("editable_profile".into(), profile);
        }
        Err(err) => {
            observed.insert("edit_blocked_reason".into(), json!(format!("{err:#}")));
        }
    }
    observed.insert("fields".into(), Value::Array(fields));
    observed.insert("env_refs".into(), Value::Array(env_refs));
    match crate::native_account_context::launch_value(
        ctx,
        &format!("profiles:{}", def.id),
        "launch",
    ) {
        Ok(value) => {
            observed.insert("launch_value".into(), json!(value));
        }
        Err(err) => {
            observed.insert("launch_value".into(), Value::Null);
            observed.insert("launch_blocked_reason".into(), json!(format!("{err:#}")));
        }
    }
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
            // Claude 实例的地址在 env 里；两种都给出，地址里内嵌的账号密码打码
            "base_url": def
                .base_url
                .clone()
                .or_else(|| {
                    def.env
                        .get("ANTHROPIC_BASE_URL")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .map(|url| crate::native::redact_url_userinfo(&url)),
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
            Ok(def) => elements.push(element(ctx, &def, &raw)),
            Err(err) => elements.push(invalid_element(&id, &raw, &err)),
        }
    }
    Ok(json!({"domain": "profiles", "elements": elements}))
}
