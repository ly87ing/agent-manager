//! Conservative recognition of automatic configuration backup names.
use anyhow::{bail, Result};
use cap_std::fs::Dir;
use std::path::Path;

fn timestamp(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

pub(super) fn home_name(name: &str) -> bool {
    timestamp(name, ".claude.json.backup.")
}

pub(super) fn nested(path: &Path) -> bool {
    let parts: Vec<_> = path.iter().collect();
    parts.len() == 2
        && parts[0] == "backups"
        && parts[1]
            .to_str()
            .is_some_and(|name| home_name(name) || timestamp(name, ".claude.json.corrupted."))
}

pub(super) struct HomeNames {
    pub selected: Vec<String>,
    pub unclassified: Vec<String>,
}

pub(super) fn home_names(home: &Dir) -> Result<HomeNames> {
    let mut names = Vec::new();
    let mut unclassified = Vec::new();
    for (count, entry) in home.entries()?.enumerate() {
        if count >= 100_000 {
            bail!("The HOME entry count exceeds the maintenance limit.");
        }
        let name = entry?.file_name();
        if name.to_str().is_none()
            && name
                .as_encoded_bytes()
                .get(..13)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b".claude.json."))
        {
            bail!("A Claude configuration copy has an undisplayable name; cleanup is unavailable.");
        }
        if let Some(name) = name
            .to_str()
            .filter(|name| name.to_ascii_lowercase().starts_with(".claude.json."))
        {
            if names.len() + unclassified.len() == 1024 {
                bail!("The configuration backup count exceeds the preview limit.");
            }
            if home_name(name) {
                names.push(name.to_owned());
            } else {
                unclassified.push(name.to_owned());
            }
        }
    }
    names.sort();
    unclassified.sort();
    Ok(HomeNames {
        selected: names,
        unclassified,
    })
}
