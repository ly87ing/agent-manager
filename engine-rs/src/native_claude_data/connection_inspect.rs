//! Fixed default Claude carriers and lexical references; values never leave memory.
use crate::Context;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub(super) const KEYS: &[&str] = &[
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "ALL_PROXY",
    "all_proxy",
    "ANTHROPIC_CUSTOM_HEADERS",
    "CLAUDE_CONFIG_DIR",
    "PROXY_MANAGED",
];
pub(super) fn is_key(key: &str) -> bool {
    key.len() <= 128
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && (KEYS.contains(&key) || key.starts_with("ANTHROPIC_"))
}
pub(super) fn shared_key(key: &str) -> bool {
    is_key(key) && (key.starts_with("ANTHROPIC_") || key == "CLAUDE_CONFIG_DIR")
}
pub(super) const FILES: &[(&str, &str)] = &[
    ("settings", "settings.json"),
    ("settings-local", "settings.local.json"),
];

pub(super) fn home(ctx: &Context) -> Result<PathBuf> {
    if !ctx.home_dir.is_absolute() {
        bail!("An absolute HOME is required.");
    }
    dunce::canonicalize(&ctx.home_dir).map_err(|_| anyhow::anyhow!("The user HOME is unavailable."))
}

pub(super) fn settings_root(ctx: &Context, selected: Option<&str>) -> Result<PathBuf> {
    let home = home(ctx)?;
    let root = selected
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    if !root.is_absolute()
        || root
            .to_str()
            .is_none_or(|text| text.len() > 4096 || text.chars().any(char::is_control))
        || root
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        bail!("Choose an absolute Claude configuration directory.");
    }
    crate::native_mcp::targets::validate_target_path(&root.join("settings.json"))
        .map_err(|_| anyhow::anyhow!("The selected directory is redirected or unavailable."))?;
    let root = crate::native_profiles::paths::resolved(&root)?;
    let normalize = |path: &Path| {
        path.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    };
    let root_key = normalize(&root);
    let mut protected = vec![
        home.join(".codex"),
        crate::native_profiles::paths::resolved(&home.join(".codex"))?,
        crate::native_profiles::paths::resolved(&ctx.data_dir)?,
    ];
    if let Some(codex) = std::env::var_os("CODEX_HOME") {
        let codex = PathBuf::from(codex);
        if !codex.is_absolute() {
            bail!("The caller's Codex location is uncertain.");
        }
        protected.push(crate::native_profiles::paths::resolved(&codex)?);
    }
    for protected in protected {
        let protected = normalize(&protected);
        if root_key == protected
            || root_key.starts_with(&(protected.clone() + "/"))
            || protected.starts_with(&(root_key.clone() + "/"))
        {
            bail!("The selected directory overlaps Codex or Agent Manager data.");
        }
    }
    for ancestor in root.ancestors().take_while(|path| *path != home) {
        if super::retention::worktrees(ancestor) {
            bail!("Worktree containers are protected.");
        }
        match cap_std::fs::Dir::open_ambient_dir(ancestor, cap_std::ambient_authority()) {
            Ok(dir) if super::retention::project(&dir)? => {
                bail!("Project and worktree directories are protected.")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("Directory ownership could not be checked."),
        }
    }
    Ok(root)
}

pub(super) fn execute(ctx: &Context, request: &crate::Request) -> Result<Value> {
    let mut root = None;
    let mut seen = std::collections::BTreeSet::new();
    let mut args = request.args.iter();
    while let Some(arg) = args.next() {
        if !seen.insert(arg.as_str()) {
            bail!("Duplicate inspection argument.");
        }
        match arg.as_str() {
            "--connection-root" => {
                root = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("Missing selected directory."))?
                        .as_str(),
                )
            }
            "--json" | "--offline" => {}
            _ => bail!("Unsupported inspection argument."),
        }
    }
    inspect_root(ctx, root)
}

pub(super) fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    crate::native_mcp::targets::validate_target_path(path).map_err(|_| {
        anyhow::anyhow!("A source is redirected or is not a supported ordinary file.")
    })?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("A source could not be read."),
    };
    if !file.metadata()?.is_file() || file.metadata()?.len() > 1024 * 1024 {
        bail!("A source is not a bounded ordinary file.");
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("A source grew during inspection.");
    }
    Ok(Some(bytes))
}

fn source(id: &str, path: &Path, settings: bool) -> Value {
    let inspected = (|| -> Result<Option<(Vec<String>, Vec<Value>)>> {
        let Some(bytes) = read(path)? else {
            return Ok(None);
        };
        let items = if settings {
            let mut items:Vec<_>=super::connection_json::environment(&bytes)?.map(|(_,fields)|fields.0.into_iter().filter(|(key,_)|is_key(key)).map(|(key,_)|json!({"id":format!("{id}:{key}"),"key":key,"line":null,"editable":true})).collect()).unwrap_or_default();
            for key in super::connection_json::root_fields(&bytes)? {
                items.push(
                    json!({"id":format!("{id}:root:{key}"),"key":key,"line":null,"editable":true}),
                );
            }
            items
        } else {
            super::connection_shell::items(id, &bytes)?
        };
        if items.len() > 1024 {
            bail!("Too many connection references to inspect safely.");
        }
        let keys: std::collections::BTreeSet<_> = items
            .iter()
            .filter_map(|item| item["key"].as_str().map(str::to_string))
            .collect();
        Ok(Some((keys.into_iter().collect(), items)))
    })();
    match inspected {
        Ok(Some((keys, items))) => {
            json!({"id":id,"path":path,"status":"inspected","keys":keys,"items":items,"kind":if settings {"settings"}else{"references"}})
        }
        Ok(None) => {
            json!({"id":id,"path":path,"status":"not_found","keys":[],"items":[],"kind":if settings {"settings"}else{"references"}})
        }
        Err(_) => {
            json!({"id":id,"path":path,"status":"unavailable","keys":[],"items":[],"kind":if settings {"settings"}else{"references"}})
        }
    }
}

fn inspect_root(ctx: &Context, selected: Option<&str>) -> Result<Value> {
    let home = home(ctx)?;
    let root = settings_root(ctx, selected)?;
    let mut sources: Vec<_> = FILES
        .iter()
        .map(|(id, name)| source(id, &root.join(name), true))
        .collect();
    #[cfg(not(windows))]
    for file in [
        ".zshrc",
        ".zprofile",
        ".zshenv",
        ".bashrc",
        ".bash_profile",
        ".profile",
    ] {
        sources.push(source(file, &home.join(file), false));
    }
    #[cfg(windows)]
    for folder in ["PowerShell", "WindowsPowerShell"] {
        for name in ["profile.ps1", "Microsoft.PowerShell_profile.ps1"] {
            sources.push(source(
                &format!("{folder}/{name}"),
                &home.join("Documents").join(folder).join(name),
                false,
            ));
        }
    }
    let process_keys: Vec<_> = std::env::vars_os()
        .filter_map(|(key, _)| key.into_string().ok())
        .filter(|key| is_key(key))
        .collect();
    sources.push(json!({"id":"caller","path":null,"status":"inspected","keys":process_keys,"items":[],"kind":"environment"}));
    #[cfg(target_os = "macos")]
    {
        let mut keys: std::collections::BTreeSet<String> =
            KEYS.iter().map(|key| key.to_string()).collect();
        for source in &sources {
            for key in source["keys"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                keys.insert(key.to_string());
            }
        }
        let session = (|| -> Result<Value> {
            let scope = super::connection_session::scope()?;
            let mut items = Vec::new();
            let start = std::time::Instant::now();
            if keys.len() > 64 {
                bail!("User environment inspection exceeds its bound.");
            }
            for key in keys {
                if start.elapsed() > std::time::Duration::from_secs(5) {
                    bail!("User environment inspection timed out.");
                }
                if super::connection_session::read(&key)?.is_some() {
                    items.push(json!({"id":format!("user-env:{key}"),"key":key,"line":null,"editable":shared_key(&key)}));
                }
            }
            Ok(
                json!({"id":"user-environment","path":scope,"status":"inspected","keys":items.iter().map(|item|item["key"].clone()).collect::<Vec<_>>(),"items":items,"kind":"environment"}),
            )
        })();
        sources.push(session.unwrap_or_else(|_|json!({"id":"user-environment","path":null,"status":"unavailable","keys":[],"items":[],"kind":"environment"})));
    }
    #[cfg(windows)]
    for (id, hive, path) in [
        (
            "user-environment",
            winreg::enums::HKEY_CURRENT_USER,
            "Environment",
        ),
        (
            "machine-environment",
            winreg::enums::HKEY_LOCAL_MACHINE,
            "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        ),
    ] {
        let scanned = winreg::RegKey::predef(hive)
            .open_subkey_with_flags(path, winreg::enums::KEY_READ)
            .and_then(|registry| {
                let mut present = Vec::new();
                for key in KEYS {
                    match registry.get_raw_value(key) {
                        Ok(_) => present.push(*key),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
                Ok(present)
            });
        sources.push(match scanned {
            Ok(keys) => {
                json!({"id":id,"path":null,"status":"inspected","keys":keys,"items":[],"kind":"environment"})
            }
            Err(_) => {
                json!({"id":id,"path":null,"status":"unavailable","keys":[],"items":[],"kind":"environment"})
            }
        });
    }
    Ok(
        json!({"scope":"claude_connections","root":root,"sources":sources,"cc_switch":super::connection_sources::cc_switch(),"custom_config_present":std::env::var_os("CLAUDE_CONFIG_DIR").is_some(),"clear_supported":cfg!(target_os="macos")}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspection_is_redacted_and_never_executes_profiles_or_reads_codex() {
        let root =
            std::env::temp_dir().join(format!("connection-inspect-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::create_dir(root.join(".codex")).unwrap();
        std::fs::write(
            root.join(".claude/settings.json"),
            br#"{"env":{"ANTHROPIC_API_KEY":"PRIVATE_SENTINEL"},"model":"YOUR_MODEL"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join(".claude/settings.local.json"),
            b"INVALID_PRIVATE_JSON",
        )
        .unwrap();
        std::fs::write(
            root.join(".zshrc"),
            "# ANTHROPIC_AUTH_TOKEN=PRIVATE_SHELL_SENTINEL\ntouch SHOULD_NOT_EXIST\n",
        )
        .unwrap();
        std::fs::write(root.join(".codex/auth.json"), b"PRIVATE_CODEX").unwrap();
        let ctx = Context {
            home_dir: root.clone(),
            data_dir: root.join("missing-data"),
        };
        let value = inspect_root(&ctx, None).unwrap();
        let text = value.to_string();
        assert!(text.contains("ANTHROPIC_API_KEY"));
        assert!(!text.contains("PRIVATE_"));
        assert_eq!(value["sources"][0]["keys"], json!(["ANTHROPIC_API_KEY"]));
        assert_eq!(value["sources"][1]["status"], "unavailable");
        assert!(!ctx.data_dir.exists());
        assert!(!root.join("SHOULD_NOT_EXIST").exists());
        assert_eq!(
            std::fs::read(root.join(".codex/auth.json")).unwrap(),
            b"PRIVATE_CODEX"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
