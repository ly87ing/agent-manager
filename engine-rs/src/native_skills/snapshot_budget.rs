//! Resource limits apply only to the directory-materialized snapshot format.
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use super::snapshot_materialized::{join_key, ordinary_file};
use super::snapshot_v2::{Entries, Entry};

const MAX_NODES: usize = 65_536;
const MAX_PATH_BYTES: usize = 4 * 1024 * 1024;
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const HEADROOM: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Budget {
    nodes: usize,
    paths: usize,
    bytes: u64,
}

impl Budget {
    pub(super) fn entry(&mut self, output: &str, input: &str) -> Result<()> {
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Snapshot node budget overflow"))?;
        self.paths = self
            .paths
            .checked_add(output.len())
            .and_then(|n| n.checked_add(input.len()))
            .ok_or_else(|| anyhow::anyhow!("Snapshot path budget overflow"))?;
        if self.nodes > MAX_NODES || self.paths > MAX_PATH_BYTES {
            bail!("Directory materialization exceeds its node or path budget");
        }
        Ok(())
    }

    pub(super) fn bytes(&mut self, count: u64) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(count)
            .ok_or_else(|| anyhow::anyhow!("Snapshot byte budget overflow"))?;
        if self.bytes > MAX_BYTES {
            bail!("Directory materialization exceeds its 512 MiB content budget");
        }
        Ok(())
    }
}

pub(super) fn preflight(
    source: &Path,
    destination: &Path,
    paths: &BTreeMap<String, String>,
    entries: &Entries,
) -> Result<()> {
    let mut budget = Budget::default();
    for (output, input) in paths {
        budget.entry(output, input)?;
        let path = join_key(source, input);
        match entries.get(input) {
            Some(Entry::File(_)) => budget.bytes(ordinary_file(&path)?.metadata()?.len())?,
            Some(Entry::Directory) => {
                super::snapshot_storage::open_directory(&path)?;
            }
            _ => bail!("Directory materialization requires ordinary source entries"),
        }
    }
    headroom(destination, budget.bytes)
}

fn headroom(destination: &Path, bytes: u64) -> Result<()> {
    let mut existing = destination;
    while !existing.try_exists()? {
        existing = existing
            .parent()
            .ok_or_else(|| anyhow::anyhow!("No existing snapshot destination ancestor"))?;
    }
    #[cfg(unix)]
    let available = {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(existing.as_os_str().as_bytes())?;
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let stats = unsafe { stats.assume_init() };
        (stats.f_bavail as u64)
            .checked_mul(stats.f_frsize as u64)
            .ok_or_else(|| anyhow::anyhow!("Filesystem availability overflow"))?
    };
    #[cfg(windows)]
    let available = {
        use std::os::windows::ffi::OsStrExt;
        let path: Vec<u16> = existing.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0;
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        available
    };
    if available < bytes + HEADROOM {
        bail!("Directory materialization requires payload space plus 64 MiB free headroom");
    }
    Ok(())
}

pub(super) fn copy(
    source: &Path,
    destination: &Path,
    paths: &BTreeMap<String, String>,
    entries: &Entries,
) -> Result<()> {
    let mut budget = Budget::default();
    for (output, input) in paths {
        budget.entry(output, input)?;
        let output = join_key(destination, output);
        match entries.get(input) {
            Some(Entry::Directory) => {
                std::fs::create_dir(output)?;
            }
            Some(Entry::File(_)) => {
                let mut input = ordinary_file(&join_key(source, input))?;
                let mut output = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(output)?;
                copy_bytes(&mut input, &mut output, &mut budget)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    output.set_permissions(std::fs::Permissions::from_mode(0o444))?;
                }
            }
            _ => bail!("Directory materialization requires ordinary source entries"),
        }
    }
    Ok(())
}

fn copy_bytes(input: &mut impl Read, output: &mut impl Write, budget: &mut Budget) -> Result<()> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = match input.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return Ok(());
        }
        budget.bytes(count as u64)?;
        output.write_all(&buffer[..count])?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_budget_accepts_exact_limits_and_refuses_each_excess() {
        let mut nodes = Budget {
            nodes: MAX_NODES - 1,
            ..Budget::default()
        };
        nodes.entry("a", "a").unwrap();
        assert!(nodes.entry("b", "b").is_err());
        let mut paths = Budget {
            paths: MAX_PATH_BYTES - 2,
            ..Budget::default()
        };
        paths.entry("a", "a").unwrap();
        assert!(paths.entry("b", "b").is_err());
        let mut bytes = Budget {
            bytes: MAX_BYTES - 3,
            ..Budget::default()
        };
        bytes.bytes(3).unwrap();
        assert!(bytes.bytes(1).is_err());
    }

    #[test]
    fn directory_copy_and_physical_reader_consume_actual_bytes() {
        let root =
            std::env::temp_dir().join(format!("am-directory-budget-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("payload"), b"four").unwrap();
        let mut budget = Budget {
            bytes: MAX_BYTES - 3,
            ..Budget::default()
        };
        let mut output = Vec::new();
        assert!(copy_bytes(
            &mut std::fs::File::open(root.join("payload")).unwrap(),
            &mut output,
            &mut budget
        )
        .is_err());
        assert!(
            output.is_empty(),
            "Bytes beyond the limit must not be written"
        );
        let mut budget = Budget {
            bytes: MAX_BYTES - 4,
            ..Budget::default()
        };
        copy_bytes(
            &mut std::fs::File::open(root.join("payload")).unwrap(),
            &mut output,
            &mut budget,
        )
        .unwrap();
        assert_eq!(output, b"four");
        let mut budget = Budget {
            bytes: MAX_BYTES - 3,
            ..Budget::default()
        };
        assert!(
            super::super::snapshot_v2::collect_entries(&root, true, Some(&mut budget)).is_err()
        );
        let mut budget = Budget {
            bytes: MAX_BYTES - 4,
            ..Budget::default()
        };
        assert_eq!(
            super::super::snapshot_v2::collect_entries(&root, true, Some(&mut budget))
                .unwrap()
                .len(),
            1
        );
        let sparse = std::fs::OpenOptions::new()
            .write(true)
            .open(root.join("payload"))
            .unwrap();
        sparse.set_len(MAX_BYTES + 1).unwrap();
        let entries = [("payload".into(), Entry::File("unused".into()))]
            .into_iter()
            .collect();
        let paths = [("payload".into(), "payload".into())].into_iter().collect();
        let destination = root.join("destination");
        let error = preflight(&root, &destination, &paths, &entries).unwrap_err();
        assert!(error.to_string().contains("512 MiB"));
        assert!(!destination.exists());
        drop(sparse);
        std::fs::remove_dir_all(root).unwrap();
    }
}
