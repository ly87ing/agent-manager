//! Assistant identities are immutable metadata, never extension dependencies.

use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) struct Scope {
    commands: BTreeSet<String>,
    npm_packages: BTreeSet<String>,
}

fn command_name(value: &str) -> String {
    let value = value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".ps1"] {
        if let Some(name) = value.strip_suffix(suffix) {
            return name.to_string();
        }
    }
    value
}

impl Scope {
    pub(super) fn new(policy: &Value) -> Self {
        let mut scope = Self {
            commands: BTreeSet::new(),
            npm_packages: BTreeSet::new(),
        };
        for policy in [&crate::store::builtin_policy(), policy] {
            if let Some(definitions) = policy
                .pointer("/agents/definitions")
                .and_then(Value::as_object)
            {
                for definition in definitions.values() {
                    if let Some(bin) = definition.get("bin").and_then(Value::as_str) {
                        scope.commands.insert(command_name(bin));
                    }
                    if let Some(package) = definition.get("upstream_npm").and_then(Value::as_str) {
                        scope.npm_packages.insert(package.to_ascii_lowercase());
                    }
                }
            }
        }
        scope
    }

    pub(super) fn assistant_bin(&self, bin: &str) -> bool {
        self.commands.contains(&command_name(bin))
    }

    pub(super) fn assistant_program(&self, program: &str) -> bool {
        let Some((scheme, name)) = program.split_once(':') else {
            return false;
        };
        match scheme.to_ascii_lowercase().as_str() {
            "bin" | "path" => self.assistant_bin(name),
            "npm" => {
                let name = name.to_ascii_lowercase();
                self.npm_packages.iter().any(|package| {
                    name == *package
                        || name
                            .strip_prefix(package)
                            .is_some_and(|rest| rest.starts_with('@'))
                })
            }
            _ => false,
        }
    }

    pub(super) fn ensure_extension<'a>(
        &self,
        program: &str,
        bins: impl IntoIterator<Item = &'a str>,
    ) -> Result<()> {
        if self.assistant_program(program) || bins.into_iter().any(|bin| self.assistant_bin(bin)) {
            bail!("助手程序不属于扩展依赖，本工具不能管理其安装、命令或版本");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn assistant_scope_uses_names_and_immutable_metadata_not_install_locations() {
        let scope = Scope::new(
            &json!({"agents":{"definitions":{"custom":{"bin":"C:\\tools\\custom-assistant.EXE", "upstream_npm":"@example/assistant", "_delete":true}}}}),
        );
        for bin in [
            "claude",
            "CODEX.CMD",
            "/unusual/bin/claude.exe",
            "custom-assistant",
            "C:\\other\\custom-assistant.ps1",
        ] {
            assert!(scope.assistant_bin(bin), "{bin}");
        }
        for id in [
            "npm:@openai/codex",
            "NPM:@openai/codex",
            "npm:@anthropic-ai/claude-code@1.0.0",
            "npm:@example/assistant",
            "path:C:\\other\\codex.exe",
            "bin:claude",
        ] {
            assert!(scope.assistant_program(id), "{id}");
        }
        for id in [
            "npm:@openai/codex-extension",
            "npm:extension-fixture",
            "bin:node",
        ] {
            assert!(!scope.assistant_program(id), "{id}");
        }
    }
}
