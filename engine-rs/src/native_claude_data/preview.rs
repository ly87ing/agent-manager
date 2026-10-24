use super::{credentials, processes, telemetry, tree};
use crate::Context;
use anyhow::{bail, Result};
use cap_std::fs::Dir;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(super) struct Configuration {
    pub in_root: bool,
    pub name: String,
    pub entry: tree::Entry,
    pub retained: telemetry::Retained,
    pub changed: bool,
}

pub(super) struct Snapshot {
    pub home: Dir,
    pub root: Option<Dir>,
    pub root_id: Vec<u8>,
    pub entries: Vec<tree::Entry>,
    pub configurations: Vec<Configuration>,
    pub credentials: Option<credentials::Snapshot>,
    pub files_only: bool,
    pub blockers: Vec<String>,
    pub base: String,
    pub root_path: PathBuf,
}

fn category(path: &Path) -> &'static str {
    match path
        .components()
        .next()
        .and_then(|v| v.as_os_str().to_str())
        .unwrap_or("")
    {
        ".credentials.json" | ".device-keys.json" | ".claude.json" => "account",
        "projects" | "history.jsonl" | "sessions" | "file-history" | "plans" | "session-env"
        | "shell-snapshots" | "tasks" | "todos" => "sessions",
        "settings.json"
        | "settings.local.json"
        | "managed-settings.json"
        | "skills"
        | "rules"
        | "hooks"
        | "commands"
        | "agents"
        | "plugins"
        | "CLAUDE.md" => "configuration",
        "debug" | "cache" | "statsig" | "telemetry" | "logs" | "stats-cache.json"
        | "paste-cache" | "image-cache" | "usage-data" => "cache",
        _ => "other",
    }
}

fn configuration(parent: &Dir, in_root: bool, name: &str, root: &str) -> Result<Configuration> {
    let entry = tree::entry(parent, Path::new(name), PathBuf::from(name))?;
    if entry.kind != tree::Kind::File {
        bail!("A settings file is redirected or is not an ordinary file; no data was changed.");
    }
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        if parent.symlink_metadata(name)?.nlink() != 1 {
            bail!("A settings file has hard-link aliases; no data was changed.");
        }
    }
    #[cfg(windows)]
    super::files::validate_single_link(parent, Path::new(name))?;
    use std::io::Read;
    let mut bytes = Vec::new();
    parent
        .open(name)?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    let retained = telemetry::retain(&bytes, root)?;
    let changed = retained.bytes.as_deref() != Some(bytes.as_slice());
    if changed && retained.bytes.is_some() {
        super::files::validate_replacement(parent, Path::new(name))?;
    }
    if tree::entry(parent, Path::new(name), PathBuf::from(name))?.hash != entry.hash {
        bail!("A settings file changed; request a new preview.");
    }
    Ok(Configuration {
        in_root,
        name: name.into(),
        entry,
        retained,
        changed,
    })
}

impl Snapshot {
    pub fn capture(ctx: &Context, files_only: bool) -> Result<Self> {
        if !ctx.home_dir.is_absolute() {
            bail!("An absolute user HOME is required.");
        }
        let home_path = dunce::canonicalize(&ctx.home_dir)
            .map_err(|_| anyhow::anyhow!("The user HOME could not be inspected."))?;
        if home_path.to_str().is_none() {
            bail!("The user HOME must have a displayable path.");
        }
        let root_path = home_path.join(".claude");
        let caller_evidence = telemetry::caller_evidence(&root_path.to_string_lossy())?;
        let data_path = crate::native_profiles::paths::resolved(&ctx.data_dir)?;
        if root_path.starts_with(&data_path) || data_path.starts_with(&root_path) {
            bail!("Claude and Agent Manager data locations overlap; cleanup is unavailable.");
        }
        let home = Dir::open_ambient_dir(&home_path, cap_std::ambient_authority())?;
        let root = match home.symlink_metadata(".claude") {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                let dir = home.open_dir(".claude")?;
                tree::same_mount(&home, &dir)?;
                if tree::stamp(&metadata)? != tree::stamp(&dir.dir_metadata()?)? {
                    bail!("The Claude directory changed during inspection.");
                }
                Some(dir)
            }
            Ok(_) => {
                bail!("The default Claude directory is redirected or is not an ordinary directory.")
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => bail!("The default Claude directory could not be inspected."),
        };
        let root_id = root
            .as_ref()
            .map(tree::identity)
            .transpose()?
            .unwrap_or_default();
        let entries = root
            .as_ref()
            .map(tree::scan)
            .transpose()?
            .unwrap_or_default();
        let mut configurations = Vec::new();
        if let Some(dir) = &root {
            for name in [
                "settings.json",
                "settings.local.json",
                "managed-settings.json",
                ".claude.json",
                "remote-settings.json",
            ] {
                if entries.iter().any(|entry| entry.path == Path::new(name)) {
                    let config = configuration(dir, true, name, &root_path.to_string_lossy())?;
                    if name == "remote-settings.json" && config.retained.bytes.is_some() {
                        bail!("Cached organization policy contains telemetry settings; cleanup is unavailable without changing that policy.");
                    }
                    configurations.push(config);
                }
            }
        }
        match home.symlink_metadata(".claude.json") {
            Ok(_) => configurations.push(configuration(
                &home,
                false,
                ".claude.json",
                &root_path.to_string_lossy(),
            )?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("The global Claude config could not be inspected."),
        }
        let mut blockers = Vec::new();
        let active = match processes::active() {
            Ok(pids) => {
                if !pids.is_empty() {
                    blockers.push("Close Claude Code, Claude Desktop and their active editor integrations, then request a new preview.".into());
                }
                pids
            }
            Err(error) => {
                let timed_out = error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::TimedOut);
                blockers.push(
                    if timed_out {
                        "Running Claude process inspection timed out; cleanup is unavailable."
                    } else {
                        "Running Claude processes could not be checked; cleanup is unavailable."
                    }
                    .into(),
                );
                vec![]
            }
        };
        let credentials = match credentials::inspect(ctx, files_only) {
            Ok(value) => Some(value),
            Err(e) => {
                blockers.push(e.to_string());
                None
            }
        };
        let mut hash = Sha256::new();
        hash.update(b"claude-default-data-v1");
        hash.update(caller_evidence);
        hash.update(home_path.as_os_str().as_encoded_bytes());
        hash.update(tree::identity(&home)?);
        hash.update(&root_id);
        hash.update(tree::digest(&entries));
        for config in &configurations {
            hash.update([u8::from(config.in_root)]);
            hash.update(config.name.as_bytes());
            hash.update(&config.entry.hash);
        }
        hash.update([u8::from(files_only)]);
        if let Some(credentials) = &credentials {
            hash.update(credentials.fingerprint());
        }
        for pid in active {
            hash.update(pid.to_le_bytes());
        }
        for blocker in &blockers {
            hash.update(blocker.as_bytes());
        }
        let base = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self {
            home,
            root,
            root_id,
            entries,
            configurations,
            credentials,
            files_only,
            blockers,
            base,
            root_path,
        })
    }

    pub fn data(&self) -> Value {
        let mut groups: BTreeMap<&str, (u64, u64, u64)> = BTreeMap::new();
        for entry in &self.entries {
            if self.configurations.iter().any(|c| {
                c.in_root && c.name == entry.path.to_string_lossy() && c.retained.bytes.is_some()
            }) {
                continue;
            }
            let group = groups.entry(category(&entry.path)).or_default();
            if entry.kind == tree::Kind::Directory {
                group.1 += 1;
            } else {
                group.0 += 1;
                group.2 += entry.bytes;
            }
        }
        if let Some(config) = self
            .configurations
            .iter()
            .find(|c| !c.in_root && c.retained.bytes.is_none())
        {
            let group = groups.entry("account").or_default();
            group.0 += 1;
            group.2 += config.entry.bytes;
        }
        let groups: Vec<_> = groups.into_iter().map(|(id,(files,directories,bytes))|json!({"id":id,"files":files,"directories":directories,"bytes":bytes})).collect();
        let preserved: Vec<_> = self.configurations.iter().filter(|c|c.retained.bytes.is_some()).map(|c|json!({"file":if c.in_root {format!(".claude/{}",c.name)} else {c.name.clone()},"fields":c.retained.fields,"changed":c.changed})).collect();
        let changes = self.entries.iter().any(|e| {
            !self
                .configurations
                .iter()
                .any(|c| c.in_root && c.entry.path == e.path && !c.changed)
        }) || self.configurations.iter().any(|c| c.changed)
            || self.credentials.as_ref().is_some_and(|c| c.count() > 0);
        json!({"scope":"default_claude_code","platform":std::env::consts::OS,"root":self.root_path,"config_path":self.root_path.parent().unwrap().join(".claude.json"),"base":self.base,"can_clear":self.blockers.is_empty() && changes,"groups":groups,"preserved":preserved,"credential_items":self.credentials.as_ref().filter(|c|!c.skipped).map(|c|c.count()),"files_only":self.files_only,"blockers":self.blockers,"has_changes":changes})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn settings_with_hard_link_aliases_are_refused_without_writing() {
        let path = std::env::temp_dir().join(format!(
            "am-claude-linked-settings-{}",
            crate::store::uuid_v4()
        ));
        let ctx = Context {
            home_dir: path.join("home"),
            data_dir: path.join("manager"),
        };
        std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
        let file = ctx.home_dir.join(".claude/settings.json");
        let bytes =
            br#"{"env":{"DISABLE_TELEMETRY":"1"},"oauthAccount":{"token":"PRIVATE_ACCOUNT"}}"#;
        std::fs::write(&file, bytes).unwrap();
        std::fs::hard_link(&file, ctx.home_dir.join("retained-settings.json")).unwrap();
        let error = Snapshot::capture(&ctx, true).err().unwrap().to_string();
        assert!(error.contains("link"));
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        assert_eq!(
            std::fs::read(ctx.home_dir.join("retained-settings.json")).unwrap(),
            bytes
        );
        assert!(!ctx.data_dir.exists());
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn preview_is_zero_write_and_never_serializes_account_or_telemetry_secrets() {
        let path =
            std::env::temp_dir().join(format!("am-claude-preview-{}", crate::store::uuid_v4()));
        let ctx = Context {
            home_dir: path.join("home"),
            data_dir: path.join("manager"),
        };
        std::fs::create_dir_all(ctx.home_dir.join(".claude/projects/project")).unwrap();
        let settings = br#"{"env":{"DISABLE_TELEMETRY":"1","OTEL_EXPORTER_OTLP_HEADERS":"PRIVATE_OTEL_SENTINEL","ANTHROPIC_API_KEY":"PRIVATE_ACCOUNT_SENTINEL"},"model":"old"}"#;
        std::fs::write(ctx.home_dir.join(".claude/settings.json"), settings).unwrap();
        std::fs::write(
            ctx.home_dir.join(".claude/projects/project/session.jsonl"),
            b"PRIVATE_SESSION_SENTINEL",
        )
        .unwrap();
        let global = br#"{"oauthAccount":{"id":"PRIVATE_ACCOUNT"}}"#;
        std::fs::write(ctx.home_dir.join(".claude.json"), global).unwrap();
        let preview = Snapshot::capture(&ctx, true).unwrap();
        let data = preview.data();
        assert_eq!(data["preserved"][0]["fields"], 2);
        assert_eq!(data["preserved"][0]["changed"], true);
        let account = data["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["id"] == "account")
            .unwrap();
        assert_eq!(account["files"], 1);
        assert_eq!(account["bytes"], global.len() as u64);
        assert!(!data.to_string().contains("PRIVATE_"));
        assert_eq!(
            std::fs::read(ctx.home_dir.join(".claude/settings.json")).unwrap(),
            settings
        );
        assert!(!ctx.data_dir.exists());
        let original = preview.base.clone();
        drop(preview);
        std::fs::write(
            ctx.home_dir.join(".claude/projects/project/session.jsonl"),
            b"NEW_SESSION",
        )
        .unwrap();
        assert_ne!(original, Snapshot::capture(&ctx, true).unwrap().base);
        std::fs::remove_dir_all(path).unwrap();
    }
}
