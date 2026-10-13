//! Read-only extension registrations and shared executable search locations.

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::native::Context;
use crate::store;

pub(crate) struct AgentDef {
    pub id: String,
    pub bin: String,
    pub enabled: bool,
    spec: Value,
}

impl AgentDef {
    pub(crate) fn spec_ref(&self) -> &Value {
        &self.spec
    }
}

pub(crate) fn strip_internal_markers(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "_replace" && key.as_str() != "_delete")
                .map(|(key, value)| (key.clone(), strip_internal_markers(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_internal_markers).collect()),
        _ => value.clone(),
    }
}

pub(crate) fn load_definitions(ctx: &Context) -> Result<Vec<AgentDef>> {
    let policy = store::effective_policy(ctx)?;
    let definitions = policy
        .pointer("/agents/definitions")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("agents.definitions must be an object"))?;
    let mut targets = Vec::new();
    for (id, raw) in definitions {
        let raw = raw
            .as_object()
            .ok_or_else(|| anyhow!("Invalid extension target metadata"))?;
        if raw.get("_delete") == Some(&Value::Bool(true)) {
            continue;
        }
        targets.push(AgentDef {
            id: id.clone(),
            bin: raw
                .get("bin")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_string(),
            enabled: policy
                .get("agents")
                .and_then(|agents| agents.get(id))
                .and_then(|target| target.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(true),
            spec: strip_internal_markers(&Value::Object(raw.clone())),
        });
    }
    Ok(targets)
}

pub(crate) fn fallback_bin_dirs(home_dir: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        // Windows：官方安装器（~\.local\bin\claude.exe）、npm 全局（%APPDATA%\npm）、
        // Volta、Bun 与 Scoop 的命令目录
        let appdata = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir.join("AppData").join("Roaming"));
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir.join("AppData").join("Local"));
        return vec![
            home_dir.join(".local").join("bin"),
            // Codex 官方 install.ps1 的入口
            local
                .join("Programs")
                .join("OpenAI")
                .join("Codex")
                .join("bin"),
            appdata.join("npm"),
            local.join("Volta").join("bin"),
            home_dir.join(".bun").join("bin"),
            home_dir.join("scoop").join("shims"),
        ];
    }
    vec![
        home_dir.join(".local").join("bin"),
        home_dir.join(".npm-global").join("bin"),
        home_dir.join(".volta").join("bin"),
        home_dir.join(".bun").join("bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        // Linux 上的 Homebrew
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
    ]
}
