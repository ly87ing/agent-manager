//! Bounded logical evidence for moved packages and launchers; links are never followed.
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

pub(super) fn fingerprint(path: &Path) -> Result<Vec<u8>> {
    fn walk(
        path: &Path,
        hash: &mut Sha256,
        nodes: &mut usize,
        bytes: &mut u64,
        depth: usize,
    ) -> Result<()> {
        if *nodes == 0 || depth > 64 {
            bail!("Program recovery evidence exceeds its entry/depth limit");
        }
        *nodes -= 1;
        let metadata = std::fs::symlink_metadata(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            hash.update(metadata.dev().to_le_bytes());
            hash.update(metadata.ino().to_le_bytes());
            hash.update(metadata.mode().to_le_bytes());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            hash.update(metadata.creation_time().to_le_bytes());
            hash.update(metadata.file_attributes().to_le_bytes());
        }
        if metadata.file_type().is_symlink() {
            hash.update(b"link");
            let target = std::fs::read_link(path)?;
            let bytes = target.as_os_str().as_encoded_bytes();
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        } else if metadata.is_dir() {
            hash.update(b"directory");
            let mut entries = Vec::new();
            for entry in std::fs::read_dir(path)? {
                if entries.len() >= *nodes {
                    bail!("Program recovery evidence exceeds its entry limit");
                }
                entries.push(entry?.path());
            }
            entries.sort();
            for entry in entries {
                let name = entry.file_name().unwrap().as_encoded_bytes();
                hash.update((name.len() as u64).to_le_bytes());
                hash.update(name);
                walk(&entry, hash, nodes, bytes, depth + 1)?;
            }
            hash.update(b"end-directory");
        } else if metadata.is_file() {
            hash.update(b"file");
            hash.update(metadata.len().to_le_bytes());
            if metadata.len() > *bytes {
                bail!("Program recovery evidence exceeds its byte limit");
            }
            use crate::platform::Canon;
            let parent =
                super::uv_metadata_files::Directory::open(&path.parent().unwrap().canon()?)?;
            let mut file = parent.file(path.file_name().unwrap())?.take(*bytes + 1);
            let mut buffer = vec![0; 64 * 1024];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                if count as u64 > *bytes {
                    bail!("Program recovery evidence exceeds its byte limit");
                }
                *bytes -= count as u64;
                hash.update(&buffer[..count]);
            }
        } else {
            bail!("Unsupported object in program recovery evidence");
        }
        Ok(())
    }
    let mut hash = Sha256::new();
    walk(path, &mut hash, &mut 32768, &mut (256 * 1024 * 1024), 0)?;
    Ok(hash.finalize().to_vec())
}

pub(super) fn text(path: &Path) -> Result<String> {
    Ok(fingerprint(path)?
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
