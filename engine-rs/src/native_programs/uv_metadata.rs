//! Native installed-distribution evidence; metadata does not prove package authenticity.
use super::uv_metadata_files::Directory;
use anyhow::{anyhow, bail, Context as _, Result};
use pep440_rs::Version;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

const MAX_ENTRIES: usize = 8192;
const MAX_HEADERS: u64 = 64 * 1024;

pub(super) fn normalized_name(name: &str) -> Option<String> {
    let alphanumeric = |byte: u8| byte.is_ascii_alphanumeric();
    if !name.bytes().next().is_some_and(alphanumeric)
        || !name.bytes().last().is_some_and(alphanumeric)
        || !name
            .bytes()
            .all(|byte| alphanumeric(byte) || b"-_.".contains(&byte))
    {
        return None;
    }
    let mut normalized = String::new();
    for byte in name.bytes() {
        if b"-_.".contains(&byte) {
            if !normalized.ends_with('-') {
                normalized.push('-');
            }
        } else {
            normalized.push(byte.to_ascii_lowercase() as char);
        }
    }
    Some(normalized)
}

fn headers(file: std::fs::File) -> Result<(String, Version)> {
    let mut reader = BufReader::new(file.take(MAX_HEADERS + 1));
    let mut fields = [None::<String>, None, None];
    let mut previous: Option<Option<usize>> = None;
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let count = reader.read_until(b'\n', &mut line)?;
        total += count as u64;
        if total > MAX_HEADERS {
            bail!("Package metadata headers exceed the read limit");
        }
        if count == 0 {
            break;
        }
        let line = std::str::from_utf8(&line)?;
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            break;
        }
        if line
            .bytes()
            .any(|byte| byte.is_ascii_control() && byte != b'\t')
        {
            bail!("Malformed metadata header control character");
        }
        if line.starts_with([' ', '\t']) {
            let Some(field) = previous else {
                bail!("Orphan metadata continuation");
            };
            if let Some(index) = field {
                let value: &mut String = fields[index].as_mut().unwrap();
                value.push(' ');
                value.push_str(line.trim_matches([' ', '\t']));
            }
            continue;
        }
        let (name, value) = line.split_once(':').context("Malformed metadata header")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            bail!("Malformed metadata field name");
        }
        let index = ["Metadata-Version", "Name", "Version"]
            .iter()
            .position(|field| name.eq_ignore_ascii_case(field));
        previous = Some(index);
        if let Some(index) = index {
            if fields[index].is_some() {
                bail!("Duplicate package identity header");
            }
            fields[index] = Some(value.trim_matches([' ', '\t']).to_string());
        }
    }
    let format = fields[0]
        .as_deref()
        .context("Missing Metadata-Version")?
        .trim_matches([' ', '\t']);
    let (major, minor) = format.split_once('.').context("Invalid Metadata-Version")?;
    if !matches!(major, "1" | "2")
        || minor.is_empty()
        || !minor.bytes().all(|byte| byte.is_ascii_digit())
    {
        bail!("Unsupported Metadata-Version");
    }
    let name = normalized_name(
        fields[1]
            .as_deref()
            .context("Missing package Name")?
            .trim_matches([' ', '\t']),
    )
    .context("Invalid package Name")?;
    let text = fields[2]
        .as_deref()
        .context("Missing package Version")?
        .trim_matches([' ', '\t']);
    if text.len() > 128 {
        bail!("Package version exceeds the read limit");
    }
    Ok((name, text.parse()?))
}

pub(super) fn version(path: &Path, tool: &str) -> Result<String> {
    Ok(distribution(path, tool)?.0)
}

pub(super) fn origin<T: serde::de::DeserializeOwned>(path: &Path, tool: &str) -> Result<T> {
    let (_, directory) = distribution(path, tool).map_err(|_| {
        anyhow!("Installed distribution identity metadata is unavailable, malformed or ambiguous")
    })?;
    let mut bytes = Vec::new();
    directory
        .file(OsStr::new("direct_url.json"))?
        .take(MAX_HEADERS + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_HEADERS {
        bail!("Installed origin exceeds the read limit");
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("Installed origin metadata is malformed, duplicated or unsupported"))
}

fn distribution(path: &Path, tool: &str) -> Result<(String, Directory)> {
    let expected = normalized_name(tool).context("Invalid tool name")?;
    let environment = Directory::open(path)?;
    #[cfg(windows)]
    let roots = vec![environment
        .child(OsStr::new("Lib"))?
        .child(OsStr::new("site-packages"))?];
    #[cfg(unix)]
    let roots = {
        let lib = environment.child(OsStr::new("lib"))?;
        let mut roots = Vec::new();
        for name in lib.entries(64)? {
            let name = name
                .to_str()
                .context("Non-UTF-8 Python environment directory")?;
            let Some(version) = name
                .strip_prefix("python")
                .or_else(|| name.strip_prefix("pypy"))
            else {
                continue;
            };
            let version = if name.starts_with("python") {
                version.strip_suffix('t').unwrap_or(version)
            } else {
                version
            };
            let Some((major, minor)) = version.split_once('.') else {
                continue;
            };
            if major.is_empty()
                || minor.is_empty()
                || !major
                    .bytes()
                    .chain(minor.bytes())
                    .all(|byte| byte.is_ascii_digit())
            {
                continue;
            }
            if roots.len() == 16 {
                bail!("Too many Python environment roots");
            }
            roots.push(
                lib.child(OsStr::new(name))?
                    .child(OsStr::new("site-packages"))?,
            );
        }
        roots
    };
    let mut found = None;
    for root in roots {
        for entry in root.entries(MAX_ENTRIES)? {
            let name = entry
                .to_str()
                .context("Non-UTF-8 installed distribution entry")?;
            if !name
                .get(name.len().saturating_sub(10)..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".dist-info"))
            {
                continue;
            }
            let Some((package, directory_version)) = name[..name.len() - 10].split_once('-') else {
                continue;
            };
            if normalized_name(package).as_deref() != Some(&expected) {
                continue;
            }
            if found.is_some() {
                bail!("Ambiguous installed distribution metadata");
            }
            let directory_version: Version = directory_version.parse()?;
            let directory = root.child(&entry)?;
            let metadata = directory.file(OsStr::new("METADATA"))?;
            let (actual_name, actual_version) = headers(metadata)?;
            if actual_name != expected || actual_version != directory_version {
                bail!("Installed distribution identity disagrees with its directory");
            }
            found = Some((actual_version.to_string(), directory));
        }
    }
    found.context("Installed distribution metadata is unavailable")
}

#[cfg(test)]
mod tests;
