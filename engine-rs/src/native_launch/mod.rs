//! General launch recipes are independent of assistant accounts and MCP secrets.
#[cfg(any(target_os = "linux", windows, test))]
mod catalog;
mod render;
mod runtime;
pub(crate) mod saved;

use crate::{diagnostic, Context, Envelope, Lock, Mode, Request};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EnvironmentVariable {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Variant {
    pub name: String,
    pub timezone: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recipe {
    pub platform: String,
    pub shell: String,
    pub kind: String,
    pub target: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<EnvironmentVariable>,
    pub variants: Vec<Variant>,
}

impl Recipe {
    fn validate(&self) -> Result<()> {
        let compatible = match self.platform.as_str() {
            "macos" | "linux" => matches!(self.shell.as_str(), "zsh" | "bash"),
            "windows" => self.shell == "powershell",
            _ => false,
        };
        if !compatible || !matches!(self.kind.as_str(), "app" | "command") {
            bail!("Unsupported platform, shell or target type");
        }
        text(&self.target, 4096)?;
        if self.target.starts_with('-') || self.args.len() > 128 {
            bail!("Invalid executable or too many arguments");
        }
        for arg in &self.args {
            if arg.len() > 16384 || arg.chars().any(char::is_control) {
                bail!("Arguments cannot contain control characters or exceed 16384 bytes");
            }
        }
        if self.env.len() > 64 {
            bail!("Choose no more than 64 environment variables");
        }
        let mut environment_names = BTreeSet::new();
        for variable in &self.env {
            let mut bytes = variable.name.bytes();
            if variable.name.len() > 128
                || !bytes
                    .next()
                    .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || variable.value.len() > 16384
                || variable.value.chars().any(char::is_control)
            {
                bail!("Environment variables need a portable name and a value without control characters");
            }
            let normalized = if self.platform == "windows" {
                variable.name.to_ascii_lowercase()
            } else {
                variable.name.clone()
            };
            if !environment_names.insert(normalized) {
                bail!("Environment variable names must be unique");
            }
        }
        if self.variants.is_empty() || self.variants.len() > 32 {
            bail!("Choose between 1 and 32 aliases");
        }
        let mut names = BTreeSet::new();
        for variant in &self.variants {
            let normalized = if self.platform == "windows" {
                variant.name.to_ascii_lowercase()
            } else {
                variant.name.clone()
            };
            let reserved = matches!(normalized.as_str(), "con" | "prn" | "aux" | "nul")
                || (normalized.len() == 4
                    && (normalized.starts_with("com") || normalized.starts_with("lpt"))
                    && matches!(normalized.as_bytes()[3], b'1'..=b'9'));
            if !valid_alias_name(&variant.name)
                || variant.name.len() > 80
                || !names.insert(normalized)
                || (self.platform == "windows" && reserved)
            {
                bail!("Alias names must be unique and use letters, digits, underscores or hyphens");
            }
            if !variant.timezone.is_empty() {
                text(&variant.timezone, 128)?;
            }
        }
        Ok(())
    }

    fn environment<'a>(&'a self, variant: &'a Variant) -> Vec<(&'a str, &'a str)> {
        let mut values = self
            .env
            .iter()
            .map(|variable| (variable.name.as_str(), variable.value.as_str()))
            .collect::<Vec<_>>();
        if variant.timezone.is_empty() {
            return values;
        }
        let timezone = values.iter_mut().find(|(name, _)| {
            if self.platform == "windows" {
                name.eq_ignore_ascii_case("TZ")
            } else {
                *name == "TZ"
            }
        });
        if let Some((_, value)) = timezone {
            *value = &variant.timezone;
        } else {
            values.insert(0, ("TZ", &variant.timezone));
        }
        values
    }
}

fn valid_alias_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn text(value: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        bail!("A required value is empty, too long or contains control characters");
    }
    Ok(())
}

pub(super) fn recipe(req: &Request) -> Result<Recipe> {
    let spec: Recipe = serde_json::from_str(
        req.value("--spec")
            .ok_or_else(|| anyhow::anyhow!("Missing --spec"))?,
    )
    .map_err(|_| anyhow::anyhow!("Invalid launch recipe JSON"))?;
    spec.validate()?;
    Ok(spec)
}

/// The dependency page uses the same literal Windows execution renderer.
pub(crate) fn windows_program_command(
    program: &std::path::Path,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&std::path::Path>,
) -> Result<String> {
    let variant = Variant {
        name: "manual".into(),
        timezone: String::new(),
    };
    let spec = Recipe {
        platform: "windows".into(),
        shell: "powershell".into(),
        kind: "command".into(),
        target: program.to_string_lossy().into(),
        args: args.to_vec(),
        env: env
            .iter()
            .map(|(name, value)| EnvironmentVariable {
                name: name.clone(),
                value: value.clone(),
            })
            .collect(),
        variants: vec![variant.clone()],
    };
    spec.validate()?;
    let cwd = cwd.map(|path| path.to_string_lossy());
    if let Some(cwd) = &cwd {
        text(cwd, 4096)?;
    }
    Ok(render::build_with_cwd(&spec, &variant, cwd.as_deref()).command)
}

fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let command = req.command.replace('.', ":");
    if matches!(command.as_str(), "launch:catalog" | "launch:run") {
        return runtime::handle(ctx, req);
    }
    if matches!(
        command.as_str(),
        "launch:list" | "launch:save" | "launch:delete"
    ) {
        return saved::handle(ctx, req);
    }
    if command != "launch:build" {
        bail!("Unsupported launch operation");
    }
    if req.args.len() != 2 || req.args[0] != "--spec" {
        bail!("build accepts exactly one --spec argument");
    }
    let spec = recipe(req)?;
    let entries = spec
        .variants
        .iter()
        .map(|variant| render::build(&spec, variant))
        .collect::<Vec<_>>();
    Ok(
        json!({"entries": entries, "platform": spec.platform, "shell": spec.shell,
        "timezone_verified": false}),
    )
}

pub(crate) fn execute(ctx: &Context, req: &Request) -> Envelope {
    let result = handle(ctx, req);
    let (data, errors) = match result {
        Ok(value) => (value, vec![]),
        Err(error) => (
            json!({}),
            vec![diagnostic(
                "LAUNCH_ERROR",
                None,
                crate::sanitize::sanitize_diagnostic_message(&error.to_string()),
            )],
        ),
    };
    let warnings = if data["audit_recorded"] == false {
        vec![diagnostic("LAUNCH_AUDIT_FAILED", None,
            "The launch request was accepted, but its result could not be recorded. Check the application before trying again.".into())]
    } else {
        vec![]
    };
    Envelope::new(
        req.command.replacen(':', ".", 1),
        Mode { offline: true },
        Lock {
            held: false,
            pid: None,
        },
        req.run_id.clone(),
        data,
        warnings,
        errors,
    )
}
