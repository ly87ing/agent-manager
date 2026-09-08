//! agents domain kernels — port of build_agents_status / build_agents_plan
//! from src/engine_v1.py. State mapping (convergence axis) mirrors the Python
//! element-state function exactly; diagnostics codes/messages are byte-identical.

use crate::envelope::{diagnostic, Diagnostic};
use crate::ids::canonicalize_agent_mapping;
use crate::mcp::ConfigInvalid;
use crate::providers;
use crate::providers::Inventory;
use crate::upstream;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

const AGENT_INSTALL_ACTION: &str =
    "{\"id\": \"install\", \"risk\": \"install-remove\", \"needs_lock\": true, \"offline_ok\": false}";
const AGENT_UPGRADE_ACTION: &str =
    "{\"id\": \"upgrade\", \"risk\": \"network\", \"needs_lock\": true, \"offline_ok\": false}";
const AGENT_UNINSTALL_ACTION: &str =
    "{\"id\": \"uninstall\", \"risk\": \"install-remove\", \"needs_lock\": true, \"offline_ok\": false}";

fn action_json(template: &str) -> Value {
    serde_json::from_str(template).unwrap()
}

fn actions_for_state(state: &str) -> Vec<Value> {
    match state {
        "missing" => vec![action_json(AGENT_INSTALL_ACTION)],
        "managed-ok" | "drifted" => vec![action_json(AGENT_UPGRADE_ACTION), action_json(AGENT_UNINSTALL_ACTION)],
        "unmanaged-detected" => vec![action_json(AGENT_INSTALL_ACTION), action_json(AGENT_UNINSTALL_ACTION)],
        _ => vec![],
    }
}

struct Diagnostics<'a> {
    warnings: &'a mut Vec<Diagnostic>,
    errors: &'a mut Vec<Diagnostic>,
}

/// _agent_element_state port.
fn element_state(
    agent_id: &str,
    definition: &Map<String, Value>,
    policy: Option<&Map<String, Value>>,
    declared: bool,
    enabled: bool,
    providers_: &[String],
    classified: &providers::Classified,
    installed: bool,
    diagnostics: &mut Diagnostics,
) -> &'static str {
    let preferred = policy
        .and_then(|p| p.get("preferred_provider"))
        .and_then(Value::as_str)
        .or_else(|| definition.get("preferred_provider").and_then(Value::as_str))
        .unwrap_or("npm");
    if enabled {
        if !classified.unsupported.is_empty() {
            diagnostics.errors.push(diagnostic(
                "AGENT_PROVIDER_UNSUPPORTED",
                Some(format!("agents:{}", agent_id)),
                format!("检测到未声明的安装来源: {}", classified.unsupported.join(", ")),
            ));
            return "unsupported";
        }
        if !classified.deprecated.is_empty() {
            return "drifted";
        }
        if !classified.system_only.is_empty()
            && definition
                .get("require_managed_provider")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            return "unmanaged-detected";
        }
        if !providers_.is_empty() && !installed {
            // doctor: provider 已激活但运行入口缺失 = 漂移。
            return "drifted";
        }
        if !classified.managed.is_empty() && installed {
            return "managed-ok";
        }
        if !providers_.is_empty() {
            return "unmanaged-detected";
        }
        return "missing";
    }
    if !declared {
        // 定义层存在但策略未声明（如 detect-only 的 kimi-code）：探测到才上报。
        return if !providers_.is_empty() || installed {
            "unmanaged-detected"
        } else {
            "disabled"
        };
    }
    if !providers_.is_empty() || installed {
        // doctor: 禁用但仍安装 = 漂移；官方无 uninstall 动作时降为提示。
        let preferred_cfg = definition
            .get("providers")
            .and_then(Value::as_object)
            .and_then(|providers| providers.get(providers::canonical_provider_name(preferred)));
        let unsupported_actions = match providers::get_unsupported_actions(preferred_cfg) {
            Ok(actions) => actions,
            Err(exc) => {
                diagnostics.errors.push(diagnostic(
                    "AGENT_PROVIDER_CONFIG_INVALID",
                    Some(format!("agents:{}", agent_id)),
                    exc,
                ));
                Default::default()
            }
        };
        if unsupported_actions.contains("uninstall") {
            diagnostics.warnings.push(diagnostic(
                "AGENT_OFFICIAL_GAP",
                Some(format!("agents:{}", agent_id)),
                "官方未提供受管 uninstall 动作，引擎不会伪造删除命令".to_string(),
            ));
        }
        return "drifted";
    }
    "disabled"
}

/// _classify_agent_probe port：preferred provider 的 install 是 npm 包 → 包名。
fn classify_agent_probe(definition: &Map<String, Value>, policy: Option<&Map<String, Value>>) -> Option<String> {
    let preferred = policy
        .and_then(|p| p.get("preferred_provider"))
        .and_then(Value::as_str)
        .or_else(|| definition.get("preferred_provider").and_then(Value::as_str))
        .unwrap_or("npm");
    let install_cmd = definition
        .get("providers")
        .and_then(Value::as_object)
        .and_then(|providers| providers.get(preferred))
        .and_then(|cfg| cfg.get("install"))
        .and_then(providers::resolve_provider_command);
    install_cmd.and_then(|cmd| providers::extract_npm_package_name(&cmd))
}

/// _agent_local_version port：bin 存在才探测 --version（本地只读）。
fn agent_local_version(definition: &Map<String, Value>) -> Option<String> {
    let bin_name = definition.get("bin").and_then(Value::as_str)?;
    if bin_name.is_empty() || providers::which(bin_name).is_none() {
        return None;
    }
    providers::extract_version_token(&providers::command_version(bin_name))
}

/// build_agents_status port.
pub fn build_agents_status(
    _root_dir: &Path,
    config_file: &Path,
    def_files: &[PathBuf],
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    let config = crate::mcp::load_json_object(config_file)?;
    let agents_policy = crate::mcp::typed_mapping(&config, "agents", &config_file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default(), false)?;
    let agents_policy = canonicalize_agent_mapping(&Value::Object(agents_policy)).map_err(ConfigInvalid)?;
    let agent_defs = canonicalize_agent_mapping(&Value::Object(
        crate::mcp::load_json_object(&def_files[0])?,
    ))
    .map_err(ConfigInvalid)?;

    let mut inventory = Inventory::new();
    if config.get("agent_definitions").is_some_and(|value| !value.is_object()) {
        return Err(ConfigInvalid("agent_definitions 必须是对象".into()));
    }
    let (agent_defs, overlay_warnings) = crate::overlay::apply_definition_overlays(
        &agent_defs, &crate::overlay::overlay_definitions(&config, "agents"), "agents",
    ).map_err(ConfigInvalid)?;
    for warning in overlay_warnings {
        warnings.push(diagnostic("DEFINITION_OVERLAY_UNKNOWN_KEYS", None, warning));
    }
    let cache = upstream::load_cache();
    let mut elements = Vec::new();
    let mut def_names: Vec<&String> = agent_defs.keys().collect();
    def_names.sort();
    for agent_id in def_names {
        let definition = match agent_defs[agent_id].as_object() {
            Some(def) => def.clone(),
            None => continue,
        };
        let policy = agents_policy.get(agent_id).and_then(Value::as_object).cloned();
        let declared = policy.is_some();
        let enabled = declared
            && policy
                .as_ref()
                .and_then(|p| p.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
        let preferred = policy
            .as_ref()
            .and_then(|p| p.get("preferred_provider"))
            .and_then(Value::as_str)
            .or_else(|| definition.get("preferred_provider").and_then(Value::as_str))
            .unwrap_or("npm")
            .to_string();
        let bin = definition.get("bin").and_then(Value::as_str).unwrap_or("");
        let detected = inventory.detect_all_providers(bin, &definition);
        let classified = providers::classify_provider_state(&detected, &definition);
        let installed = providers::is_agent_installed(&definition);
        let preferred_canonical = providers::canonical_provider_name(&preferred);
        let preferred_active = classified
            .managed
            .iter()
            .any(|name| providers::canonical_provider_name(name) == preferred_canonical);

        let state = {
            let mut diagnostics = Diagnostics {
                warnings: &mut warnings,
                errors: &mut errors,
            };
            element_state(
                agent_id,
                &definition,
                policy.as_ref(),
                declared,
                enabled,
                &detected,
                &classified,
                installed,
                &mut diagnostics,
            )
        };

        let probe = if enabled {
            classify_agent_probe(&definition, policy.as_ref())
        } else {
            None
        };
        let element_id = format!("agents:{}", agent_id);
        let freshness = crate::mcp::freshness_block(
            &element_id,
            probe.is_some(),
            &cache,
            agent_local_version(&definition),
            &mut warnings,
        );

        let mut policy_block = Map::new();
        policy_block.insert("enabled".to_string(), Value::Bool(enabled));
        policy_block.insert("declared_in_policy".to_string(), Value::Bool(declared));
        policy_block.insert(
            "auto_install".to_string(),
            Value::Bool(
                policy
                    .as_ref()
                    .and_then(|p| p.get("auto_install"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            ),
        );
        policy_block.insert(
            "auto_update".to_string(),
            Value::Bool(
                policy
                    .as_ref()
                    .and_then(|p| p.get("auto_update"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            ),
        );
        policy_block.insert(
            "preferred_provider".to_string(),
            Value::String(preferred.clone()),
        );

        let mut observed = Map::new();
        observed.insert(
            "providers".to_string(),
            Value::Array(detected.iter().cloned().map(Value::String).collect()),
        );
        observed.insert(
            "classified".to_string(),
            json_classified(&classified),
        );
        observed.insert("installed".to_string(), Value::Bool(installed));
        observed.insert("preferred_active".to_string(), Value::Bool(preferred_active));

        let mut element = Map::new();
        element.insert("id".to_string(), Value::String(element_id));
        element.insert("domain".to_string(), Value::String("agents".to_string()));
        element.insert("spec".to_string(), Value::Object(definition));
        element.insert("policy".to_string(), Value::Object(policy_block));
        element.insert("observed".to_string(), Value::Object(observed));
        element.insert("state".to_string(), Value::String(state.to_string()));
        element.insert("freshness".to_string(), freshness);
        element.insert(
            "actions".to_string(),
            Value::Array(actions_for_state(state)),
        );
        elements.push(Value::Object(element));
    }

    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("agents".to_string()));
    data.insert("elements".to_string(), Value::Array(elements));
    Ok((Value::Object(data), warnings, errors))
}

fn json_classified(classified: &providers::Classified) -> Value {
    let mut map = Map::new();
    map.insert(
        "managed".to_string(),
        Value::Array(classified.managed.iter().cloned().map(Value::String).collect()),
    );
    map.insert(
        "deprecated".to_string(),
        Value::Array(classified.deprecated.iter().cloned().map(Value::String).collect()),
    );
    map.insert(
        "system_only".to_string(),
        Value::Array(classified.system_only.iter().cloned().map(Value::String).collect()),
    );
    map.insert(
        "unsupported".to_string(),
        Value::Array(classified.unsupported.iter().cloned().map(Value::String).collect()),
    );
    Value::Object(map)
}

/// 升级动作的 freshness 说明（_upgrade_note port）。
fn upgrade_note(kind_label: &str, provider_key: Option<&str>, freshness: &Value) -> String {
    let provider_part = provider_key
        .map(|key| format!("（provider: {}）", key))
        .unwrap_or_default();
    match freshness.get("state").and_then(Value::as_str) {
        Some("behind") => format!(
            "升级{}{}：当前 {} → 最新 {}",
            kind_label,
            provider_part,
            freshness.get("current").and_then(Value::as_str).unwrap_or("?"),
            freshness.get("upstream").and_then(Value::as_str).unwrap_or("?"),
        ),
        Some("unknown") => format!("升级{}{}：上游未检查或探测失败（unknown）", kind_label, provider_part),
        _ => format!("升级{}{}：无上游探测源，跟随同步器升级语义", kind_label, provider_part),
    }
}

/// build_agents_plan port：missing + auto_install → install 动作；其余 warning 跳过。
pub fn build_agents_plan(
    root_dir: &Path,
    config_file: &Path,
    def_files: &[PathBuf],
    only: &[String],
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let (status, mut warnings, errors) = build_agents_status(root_dir, config_file, def_files)?;
    let agent_defs: Map<String, Value> = status["elements"].as_array().unwrap().iter()
        .map(|element| (element["id"].as_str().unwrap().trim_start_matches("agents:").to_string(), element["spec"].clone()))
        .collect();

    let mut actions = Vec::new();
    let elements = status
        .get("elements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !only.is_empty() {
        for target in only {
            if !elements.iter().any(|e| e.get("id").and_then(Value::as_str) == Some(target.as_str())) {
                return Err(ConfigInvalid(format!("元素 '{}' 在域 'agents' 中不存在", target)));
            }
        }
    }
    for element in elements {
        let element_id = element.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        if !only.is_empty() && !only.contains(&element_id) {
            continue;
        }
        let agent_id = element_id.splitn(2, ':').nth(1).unwrap_or("").to_string();
        let state = element.get("state").and_then(Value::as_str).unwrap_or("");
        if state != "missing" {
            // 升级动作（同步器语义：enabled+installed+受管 provider 激活+auto_update
            // → 无条件跑激活 provider 的 upgrade 命令；current 不产出）。
            if state != "managed-ok" {
                continue;
            }
            let observed = element.get("observed").cloned().unwrap_or(Value::Null);
            let installed = observed.get("installed").and_then(Value::as_bool).unwrap_or(false);
            let managed_active: Vec<String> = observed
                .get("classified")
                .and_then(|c| c.get("managed"))
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            if !installed || managed_active.is_empty() {
                continue;
            }
            let auto_update = element
                .get("policy")
                .and_then(|p| p.get("auto_update"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
            if !auto_update {
                warnings.push(diagnostic(
                    "AGENT_UPDATE_DISABLED_BY_POLICY",
                    Some(element_id.clone()),
                    "策略 auto_update=false，plan 不生成升级动作".to_string(),
                ));
                continue;
            }
            let preferred = element
                .get("policy")
                .and_then(|p| p.get("preferred_provider"))
                .and_then(Value::as_str)
                .unwrap_or("npm")
                .to_string();
            let canonical_managed: Vec<&str> = managed_active
                .iter()
                .map(|name| providers::canonical_provider_name(name))
                .collect();
            let preferred_canonical = providers::canonical_provider_name(&preferred);
            let active_key = if canonical_managed.contains(&preferred_canonical) {
                preferred_canonical.to_string()
            } else {
                canonical_managed[0].to_string()
            };
            let provider_cfg = agent_defs
                .get(&agent_id)
                .and_then(Value::as_object)
                .and_then(|def| def.get("providers"))
                .and_then(Value::as_object)
                .and_then(|providers| providers.get(&active_key))
                .cloned();
            let upgrade_cmd = provider_cfg
                .as_ref()
                .and_then(|cfg| cfg.get("upgrade"))
                .and_then(providers::resolve_provider_command);
            if let Some(ref cfg) = provider_cfg {
                if cfg.get("upgrade").is_some() && upgrade_cmd.is_none() {
                    warnings.push(diagnostic(
                        "AGENT_UPGRADE_UNAVAILABLE",
                        Some(element_id.clone()),
                        format!("provider '{}' 未提供当前平台的 upgrade 命令", active_key),
                    ));
                    continue;
                }
            }
            let upgrade_cmd = match upgrade_cmd {
                Some(cmd) => cmd,
                None => continue,
            };
            let freshness = element.get("freshness").cloned().unwrap_or(Value::Null);
            if freshness.get("state").and_then(Value::as_str) == Some("current") {
                continue;
            }
            actions.push(crate::canon::plan_action(
                "run-cmd",
                &element_id,
                Value::Null,
                "network",
                None,
                None,
                Some(upgrade_cmd),
                Some(upgrade_note("Agent", Some(&active_key), &freshness)),
            ));
            continue;
        }
        let auto_install = element
            .get("policy")
            .and_then(|p| p.get("auto_install"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if !auto_install {
            warnings.push(diagnostic(
                "AGENT_INSTALL_DISABLED_BY_POLICY",
                Some(element_id.clone()),
                "策略 auto_install=false，plan 不生成安装动作".to_string(),
            ));
            continue;
        }
        let preferred = element
            .get("policy")
            .and_then(|p| p.get("preferred_provider"))
            .and_then(Value::as_str)
            .unwrap_or("npm")
            .to_string();
        let install_cmd = agent_defs
            .get(&agent_id)
            .and_then(Value::as_object)
            .and_then(|def| def.get("providers"))
            .and_then(Value::as_object)
            .and_then(|providers| providers.get(&preferred))
            .and_then(|cfg| cfg.get("install"))
            .and_then(providers::resolve_provider_command);
        let install_cmd = match install_cmd {
            Some(cmd) => cmd,
            None => {
                warnings.push(diagnostic(
                    "AGENT_INSTALL_UNAVAILABLE",
                    Some(element_id.clone()),
                    format!("provider '{}' 未提供当前平台的 install 命令", preferred),
                ));
                continue;
            }
        };
        let mut action = Map::new();
        action.insert("op".to_string(), Value::String("install".to_string()));
        action.insert("element".to_string(), Value::String(element_id));
        action.insert("target".to_string(), Value::Null);
        action.insert("risk".to_string(), Value::String("install-remove".to_string()));
        action.insert("before_digest".to_string(), Value::Null);
        action.insert("after_digest".to_string(), Value::Null);
        action.insert("backup_path".to_string(), Value::Null);
        action.insert("cmd".to_string(), Value::String(install_cmd));
        action.insert(
            "note".to_string(),
            Value::String(format!("安装缺失 Agent（provider: {}）", preferred)),
        );
        actions.push(Value::Object(action));
    }

    let mut summary = Map::new();
    for key in ["read", "write-local", "exec-local", "network", "install-remove"] {
        summary.insert(key.to_string(), Value::Number(0.into()));
    }
    for action in &actions {
        if let Some(risk) = action.get("risk").and_then(Value::as_str) {
            if let Some(Value::Number(count)) = summary.get(risk).cloned() {
                if let Some(current) = count.as_i64() {
                    summary.insert(risk.to_string(), Value::Number((current + 1).into()));
                }
            }
        }
    }
    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("agents".to_string()));
    data.insert("actions".to_string(), Value::Array(actions));
    data.insert("summary".to_string(), Value::Object(summary));
    Ok((Value::Object(data), warnings, errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn element_state_mapping() {
        let definition = json!({"bin": "x", "providers": {"npm": {"managed": true}}});
        let def = definition.as_object().unwrap();
        let mut warnings = Vec::new();
        let mut errors = Vec::new();
        let mut diag = Diagnostics { warnings: &mut warnings, errors: &mut errors };
        let managed = providers::Classified {
            managed: vec!["npm".to_string()],
            ..Default::default()
        };
        assert_eq!(
            element_state("a", def, None, true, true, &["npm".to_string()], &managed, true, &mut diag),
            "managed-ok"
        );
        assert_eq!(
            element_state("a", def, None, true, true, &[], &Default::default(), false, &mut diag),
            "missing"
        );
        // 未声明策略 + 探测到 → unmanaged-detected；未探测 → disabled。
        assert_eq!(
            element_state("k", def, None, false, false, &["npm".to_string()], &managed, true, &mut diag),
            "unmanaged-detected"
        );
        assert_eq!(
            element_state("k", def, None, false, false, &[], &Default::default(), false, &mut diag),
            "disabled"
        );
        // 禁用但仍安装 → drifted。
        assert_eq!(
            element_state("a", def, Some(def), true, false, &["npm".to_string()], &managed, true, &mut diag),
            "drifted"
        );
    }

    #[test]
    fn unsupported_source_is_explicit_error() {
        let definition = json!({"bin": "x", "providers": {"npm": {"managed": true}}});
        let def = definition.as_object().unwrap();
        let mut warnings = Vec::new();
        let mut errors = Vec::new();
        let classified = providers::Classified {
            unsupported: vec!["mystery".to_string()],
            ..Default::default()
        };
        let state = {
            let mut diag = Diagnostics { warnings: &mut warnings, errors: &mut errors };
            element_state("a", def, Some(def), true, true, &["mystery".to_string()], &classified, true, &mut diag)
        };
        assert_eq!(state, "unsupported");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "AGENT_PROVIDER_UNSUPPORTED");
    }
}
