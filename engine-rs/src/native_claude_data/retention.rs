//! Positive artifact ownership; project markers and unknown content always win.
use anyhow::{bail, Result};
use cap_std::fs::Dir;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Retention {
    Project,
    Unknown,
    Ancestor,
}

impl Retention {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Unknown => "unclassified",
            Self::Ancestor => "retained_parent",
        }
    }
}

pub(super) fn project(dir: &Dir) -> Result<bool> {
    for name in [
        ".git",
        ".hg",
        ".svn",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "CMakeLists.txt",
        "pom.xml",
        "build.gradle",
        "deno.json",
    ] {
        match dir.symlink_metadata(name) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("Project ownership could not be checked; cleanup is unavailable."),
        }
    }
    Ok(false)
}

pub(super) fn worktrees(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("worktrees") || name.eq_ignore_ascii_case(".worktrees")
        })
}

pub(super) fn unknown_file(dir: &Dir, prefix: &Path) -> Result<bool> {
    for (count, item) in dir.entries()?.enumerate() {
        if count >= 100_000 {
            bail!("File count exceeds the maintenance limit.");
        }
        let name = item?.file_name();
        let metadata = dir.symlink_metadata(&name)?;
        if !metadata.is_dir()
            && !owned(&prefix.join(name), false, metadata.file_type().is_symlink())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn owned(path: &Path, directory: bool, link: bool) -> bool {
    let parts: Vec<_> = path
        .iter()
        .map(|part| part.to_str().unwrap_or(""))
        .collect();
    let Some(root) = parts.first().copied() else {
        return false;
    };
    let name = parts.last().copied().unwrap_or("");
    if parts.len() == 1 && !directory {
        if link {
            return false;
        }
        return matches!(
            root,
            ".credentials.json"
                | ".device-keys.json"
                | ".claude.json"
                | "settings.json"
                | "settings.local.json"
                | "managed-settings.json"
                | "remote-settings.json"
                | "history.jsonl"
                | "stats-cache.json"
                | "usage.jsonl"
                | "usage.with-fix.jsonl"
                | "keybindings.json"
                | "CLAUDE.md"
        );
    }
    let json = name.ends_with(".json") || name.ends_with(".jsonl");
    match root {
        "projects" => {
            if parts.len() <= 2 {
                return directory;
            }
            if parts[2] == "memory" {
                return directory || name.ends_with(".md");
            }
            if parts.len() == 3 && !directory {
                return transcript(name);
            }
            session_id(parts[2])
                && (parts.len() == 3 && directory
                    || parts.len() == 4 && directory && parts[3] == "subagents"
                    || parts.len() == 5
                        && !directory
                        && parts[3] == "subagents"
                        && name.starts_with("agent-")
                        && name.ends_with(".jsonl"))
        }
        "file-history" => {
            parts.len() == 1 && directory
                || parts.get(1).is_some_and(|id| session_id(id))
                    && (parts.len() == 2 && directory
                        || parts.len() == 3
                            && !directory
                            && name.rsplit_once("@v").is_some_and(|(hash, version)| {
                                !hash.is_empty()
                                    && hash.bytes().all(|c| c.is_ascii_hexdigit())
                                    && !version.is_empty()
                                    && version.bytes().all(|c| c.is_ascii_digit())
                            }))
        }
        "image-cache" | "uploads" => {
            parts.len() == 1 && directory
                || parts.get(1).is_some_and(|id| session_id(id))
                    && (parts.len() == 2 && directory
                        || parts.len() == 3
                            && !directory
                            && [".png", ".jpg", ".jpeg", ".webp", ".gif"]
                                .iter()
                                .any(|extension| name.ends_with(extension)))
        }
        "paste-cache" | "feedback-bundles" | "session-env" | "shell-snapshots" => {
            parts.len() == 1 && directory
                || !directory
                    && parts.len() == 2
                    && (json
                        || name.ends_with(".sh")
                        || name.ends_with(".txt")
                        || name.ends_with(".zip"))
        }
        "plans" => {
            parts.len() == 1 && directory || parts.len() == 2 && !directory && name.ends_with(".md")
        }
        "agent-memory" => {
            parts.len() <= 2 && directory || parts.len() == 3 && !directory && name == "MEMORY.md"
        }
        "sessions" => {
            parts.len() == 1 && directory || parts.len() == 2 && !directory && transcript(name)
        }
        "tasks" | "todos" => {
            parts.len() == 1 && directory
                || parts.get(1).is_some_and(|id| session_id(id))
                    && (parts.len() == 2 && directory
                        || parts.len() == 3
                            && !directory
                            && (matches!(name, ".lock" | ".highwatermark")
                                || name.strip_suffix(".json").is_some_and(|id| {
                                    !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit())
                                })))
        }
        "backups" => parts.len() == 1 && directory,
        "skills" | "hooks" | "workflows" => parts.len() == 1 && directory || link,
        "rules" | "commands" | "agents" | "output-styles" => {
            parts.len() == 1 && directory || link || parts.len() == 2 && name.ends_with(".md")
        }
        "plugins" => {
            parts.len() == 1 && directory
                || link
                || parts.len() == 2
                    && matches!(
                        name,
                        "installed_plugins.json" | "known_marketplaces.json" | "config.json"
                    )
        }
        "themes" => parts.len() == 1 && directory || parts.len() == 2 && json,
        "debug" | "logs" => {
            parts.len() == 1 && directory
                || parts.len() == 2
                    && !directory
                    && [".log", ".txt"]
                        .iter()
                        .any(|extension| name.strip_suffix(extension).is_some_and(session_id))
        }
        _ => false,
    }
}

fn transcript(name: &str) -> bool {
    name.strip_suffix(".jsonl").is_some_and(|stem| {
        session_id(stem)
            || stem
                .split_once(".orphaned-")
                .is_some_and(|(id, suffix)| session_id(id) && !suffix.is_empty())
    }) || name
        .split_once(".jsonl.superseded-")
        .is_some_and(|(id, suffix)| session_id(id) && !suffix.is_empty())
}

fn session_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, c)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
