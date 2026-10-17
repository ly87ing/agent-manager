//! Unix uv entries under an explicitly selected tool root need receipt evidence.
use super::uv_metadata_files::Directory;
use crate::platform::Canon;
use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

pub(super) fn locate(path: &Path, real: &Path) -> Option<(String, PathBuf)> {
    let root = PathBuf::from(std::env::var_os("UV_TOOL_DIR")?);
    if !root.is_absolute() {
        return None;
    }
    let root = root.canon().ok()?;
    let relative = real.strip_prefix(&root).ok()?;
    let mut parts = relative.components();
    let (
        Some(Component::Normal(tool)),
        Some(Component::Normal(bin)),
        Some(Component::Normal(name)),
    ) = (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if bin != "bin" || parts.next().is_some() || path.file_name()? != name {
        return None;
    }
    let tool_name = tool.to_str()?;
    super::uv_metadata::normalized_name(tool_name)?;
    let directory = Directory::open(&root).ok()?.child(tool).ok()?;
    let source = directory.child(bin).ok()?.file(name).ok()?;
    if source.metadata().ok()?.permissions().mode() & 0o111 == 0 {
        return None;
    }
    let mut contents = String::new();
    directory
        .file(OsStr::new("uv-receipt.toml"))
        .ok()?
        .take(64 * 1024 + 1)
        .read_to_string(&mut contents)
        .ok()?;
    if contents.len() > 64 * 1024 {
        return None;
    }
    let receipt = contents.parse::<toml::Value>().ok()?;
    let entries = receipt.get("tool")?.get("entrypoints")?.as_array()?;
    if !entries.iter().any(|entry| {
        entry
            .get("name")
            .and_then(toml::Value::as_str)
            .map(OsStr::new)
            == Some(name)
            && entry
                .get("install-path")
                .and_then(toml::Value::as_str)
                .map(Path::new)
                == Some(path)
    }) {
        return None;
    }
    Some((tool_name.to_string(), root.join(tool)))
}
