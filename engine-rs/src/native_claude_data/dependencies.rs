//! Preserve files still referenced by the caller's shared runtime environment.
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn file_variable(name: &str) -> bool {
    matches!(
        name,
        "NODE_EXTRA_CA_CERTS" | "SSL_CERT_FILE" | "SSL_CERT_DIR"
    ) || name.starts_with("OTEL_")
        && (name.ends_with("_CERTIFICATE") || name.ends_with("_CLIENT_KEY"))
}

fn normalized(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/").to_lowercase();
    if let Some(value) = value.strip_prefix("//?/unc/") {
        format!("//{value}")
    } else {
        value.strip_prefix("//?/").unwrap_or(&value).to_owned()
    }
}

fn validate(value: &str, targets: &[(PathBuf, bool)], dependency_directory: bool) -> Result<()> {
    if dependency_directory && value.is_empty() && !targets.is_empty() {
        bail!("A shared caller directory dependency has an uncertain empty location; cleanup is unavailable.");
    }
    if value.is_empty() || targets.is_empty() {
        return Ok(());
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        bail!("A shared caller file dependency has an uncertain relative location; cleanup is unavailable.");
    }
    let resolved = match dunce::canonicalize(path) {
        Ok(path) => Some(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            bail!("A shared caller file dependency could not be resolved; cleanup is unavailable.")
        }
    };
    for candidate in std::iter::once(path).chain(resolved.as_deref()) {
        let candidate = normalized(candidate);
        let candidate = candidate.trim_end_matches('/');
        if targets.iter().any(|(target, directory)| {
            let target = normalized(target);
            candidate == target
                || *directory && candidate.starts_with(&format!("{target}/"))
                || dependency_directory && target.starts_with(&format!("{candidate}/"))
        }) {
            bail!("A selected file is referenced by a shared caller environment variable; cleanup is unavailable.");
        }
    }
    Ok(())
}

fn validate_variable(name: &str, value: &str, targets: &[(PathBuf, bool)]) -> Result<()> {
    if name == "SSL_CERT_DIR" {
        if value.is_empty() {
            return validate(value, targets, true);
        }
        for directory in std::env::split_paths(value) {
            let value = directory.to_str().ok_or_else(|| {
                anyhow::anyhow!("A shared caller directory dependency could not be inspected.")
            })?;
            validate(value, targets, true)?;
        }
        Ok(())
    } else {
        validate(value, targets, false)
    }
}

pub(super) fn caller_evidence(targets: &[(PathBuf, bool)]) -> Result<Vec<u8>> {
    let mut evidence = BTreeMap::new();
    for (name, value) in std::env::vars_os() {
        let Some(name) = name.to_str().filter(|name| file_variable(name)) else {
            continue;
        };
        let value = value
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("A shared caller dependency could not be inspected."))?;
        validate_variable(name, value, targets)?;
        evidence.insert(name.to_owned(), value.to_owned());
    }
    Ok(Sha256::digest(serde_json::to_vec(&evidence)?).to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_directory_dependencies_cover_selected_leaves() {
        let work =
            std::env::temp_dir().join(format!("am-claude-ca-dir-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(&work).unwrap();
        let work = dunce::canonicalize(work).unwrap();
        let selected = work.join("hooks/abcd1234.0");
        std::fs::create_dir_all(work.join("hooks/unknown-source")).unwrap();
        std::fs::write(work.join("external.pem"), b"PRIVATE_SHARED_CA").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(work.join("external.pem"), &selected).unwrap();
        #[cfg(windows)]
        std::fs::write(&selected, b"PRIVATE_SHARED_CA").unwrap();
        let targets = vec![(selected, false)];
        assert!(validate_variable(
            "SSL_CERT_DIR",
            work.join("hooks").to_str().unwrap(),
            &targets
        )
        .is_err());
        let paths = std::env::join_paths([work.join("other"), work.join("hooks")]).unwrap();
        assert!(validate_variable("SSL_CERT_DIR", paths.to_str().unwrap(), &targets).is_err());
        let empty = if cfg!(windows) { ";" } else { ":" };
        assert!(validate_variable("SSL_CERT_DIR", empty, &targets).is_err());
        assert!(validate_variable("SSL_CERT_DIR", "", &targets).is_err());
        assert!(validate_variable(
            "SSL_CERT_FILE",
            work.join("hooks").to_str().unwrap(),
            &targets
        )
        .is_ok());
        std::fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn selected_shared_file_dependencies_are_refused_without_disclosing_values() {
        let work =
            std::env::temp_dir().join(format!("am-claude-shared-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(&work).unwrap();
        let work = dunce::canonicalize(work).unwrap();
        let selected = work.join("history.jsonl");
        std::fs::write(&selected, b"PRIVATE_SHARED_FILE").unwrap();
        let targets = vec![(selected.clone(), false)];
        let error = validate(selected.to_str().unwrap(), &targets, false)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("PRIVATE_") && !error.contains(selected.to_str().unwrap()));
        std::fs::write(work.join("retained.pem"), b"PRIVATE_EXTERNAL_CA").unwrap();
        assert!(validate(work.join("retained.pem").to_str().unwrap(), &targets, false).is_ok());
        assert!(validate("relative.pem", &targets, false).is_err());
        #[cfg(unix)]
        {
            let alias = work.join("external.pem");
            std::os::unix::fs::symlink(&selected, &alias).unwrap();
            assert!(validate(alias.to_str().unwrap(), &targets, false).is_err());
            let cycle = work.join("cycle.pem");
            std::os::unix::fs::symlink(&cycle, &cycle).unwrap();
            assert!(validate(cycle.to_str().unwrap(), &targets, false).is_err());
        }
        assert_eq!(std::fs::read(&selected).unwrap(), b"PRIVATE_SHARED_FILE");
        std::fs::remove_dir_all(work).unwrap();
    }
}
