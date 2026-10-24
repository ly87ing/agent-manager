//! Explicitly confirmed Claude Code local-data maintenance, separate from account APIs.
mod clear;
mod credentials;
mod dependencies;
mod files;
mod preview;
mod processes;
mod retention;
mod tree;

use crate::{diagnostic, Context, Envelope, Lock, Mode, Request};
use serde_json::json;

pub(crate) fn execute(ctx: &Context, request: &Request) -> Envelope {
    let command = request.command.replace('.', ":");
    let result = match command.as_str() {
        "claude-data:preview"
            if request
                .args
                .iter()
                .all(|v| matches!(v.as_str(), "--files-only" | "--json" | "--offline")) =>
        {
            preview::Snapshot::capture(ctx, request.flag("--files-only"))
                .map(|snapshot| snapshot.data())
        }
        "claude-data:clear" => clear_arguments(request).and_then(|base| {
            clear::execute(ctx, base, request.flag("--files-only"), &request.run_id)
        }),
        _ => Err(anyhow::anyhow!(
            "Unsupported Claude data maintenance command or arguments."
        )),
    };
    let (data, errors) = match result {
        Ok(data) => {
            let errors = if data["status"] == "partial" {
                vec![diagnostic(
                    "CLAUDE_DATA_PARTIAL",
                    None,
                    "Cleanup stopped with a partial or uncertain result; request a new preview."
                        .into(),
                )]
            } else {
                vec![]
            };
            (data, errors)
        }
        Err(error) => (
            json!({}),
            vec![diagnostic(
                "CLAUDE_DATA_UNAVAILABLE",
                None,
                error.to_string(),
            )],
        ),
    };
    Envelope::new(
        request.command.replacen(':', ".", 1),
        Mode {
            offline: request.flag("--offline"),
        },
        Lock {
            held: false,
            pid: None,
        },
        request.run_id.clone(),
        data,
        vec![],
        errors,
    )
}

fn clear_arguments(request: &Request) -> anyhow::Result<&str> {
    let mut seen = std::collections::BTreeSet::new();
    let mut base = None;
    let mut args = request.args.iter();
    while let Some(arg) = args.next() {
        if !seen.insert(arg.as_str()) {
            anyhow::bail!("Duplicate cleanup argument.");
        }
        match arg.as_str() {
            "--base" => base = args.next().map(String::as_str),
            "--confirm" | "--files-only" | "--json" | "--offline" => {}
            _ => anyhow::bail!("Unsupported cleanup argument."),
        }
    }
    let base = base.filter(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    });
    if !request.flag("--confirm") || base.is_none() {
        anyhow::bail!("Cleanup requires --base SHA256 and explicit --confirm.");
    }
    Ok(base.unwrap())
}
