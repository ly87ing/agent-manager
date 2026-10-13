//! General launch recipes are independent of assistant accounts and MCP secrets.
mod render;
mod runtime;
mod shortcuts;

use crate::{diagnostic, Context, Envelope, Lock, Mode, Request};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

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
            if !crate::native_aliases::model::valid_alias_name(&variant.name)
                || variant.name.len() > 80
                || !names.insert(normalized)
                || (self.platform == "windows" && reserved)
            {
                bail!("Alias names must be unique and use letters, digits, underscores or hyphens");
            }
            text(&variant.timezone, 128)?;
        }
        Ok(())
    }
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

fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let command = req.command.replace('.', ":");
    if matches!(command.as_str(), "launch:catalog" | "launch:run") {
        return runtime::handle(ctx, req);
    }
    if command != "launch:build" {
        return shortcuts::handle(ctx, req);
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
    Envelope::new(
        req.command.replacen(':', ".", 1),
        Mode { offline: true },
        Lock {
            held: false,
            pid: None,
        },
        req.run_id.clone(),
        data,
        vec![],
        errors,
    )
}
