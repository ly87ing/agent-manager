//! Bounded, read-only entry kinds from full Git v2/v3 indexes.
use anyhow::{bail, Context as _, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

const LIMIT: usize = 4 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Index {
    pub(super) links: BTreeSet<Vec<u8>>,
    pub(super) gitlinks: BTreeSet<Vec<u8>>,
}

pub(super) fn read(repo: &Path) -> Result<Index> {
    let (_repo_handle, repo_id) = super::snapshot_storage::open_directory(repo)?;
    let git = repo.join(".git");
    let (_git_handle, git_id) = super::snapshot_storage::open_directory(&git)?;
    match std::fs::symlink_metadata(git.join("commondir")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => bail!("Shared Git metadata is not supported by read-only skill inspection"),
    }
    let mut file = super::snapshot_materialized::ordinary_file(&git.join("index"))?;
    let size = file.metadata()?.len();
    if size > LIMIT as u64 {
        bail!("Git index exceeds the 4 MiB inspection limit");
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != size || file.metadata()?.len() != size {
        bail!("Git index changed size during inspection");
    }
    let result = decode(&bytes)?;
    super::snapshot_storage::unchanged(&git, git_id)?;
    super::snapshot_storage::unchanged(repo, repo_id)?;
    Ok(result)
}

fn decode(bytes: &[u8]) -> Result<Index> {
    if bytes.len() > LIMIT || bytes.get(..4) != Some(b"DIRC") {
        bail!("Invalid or oversized Git index header");
    }
    let version = u32::from_be_bytes(
        bytes
            .get(4..8)
            .context("Truncated Git index header")?
            .try_into()?,
    );
    if !matches!(version, 2 | 3) {
        bail!("Git index version is not supported by read-only skill inspection");
    }
    let sha1 = decode_width(bytes, version, 20).ok();
    let sha256 = decode_width(bytes, version, 32).ok();
    match (sha1, sha256) {
        (Some(index), None) | (None, Some(index)) => Ok(index),
        _ => bail!(
            "Git index kinds are unknown: invalid, ambiguous, unresolved, split or sparse index"
        ),
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .context("Git index length overflow")?;
        let value = self
            .bytes
            .get(self.offset..end)
            .context("Truncated Git index")?;
        self.offset = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into()?))
    }
}

fn decode_width(bytes: &[u8], version: u32, width: usize) -> Result<Index> {
    let end = bytes
        .len()
        .checked_sub(width)
        .context("Missing Git index checksum")?;
    let (payload, checksum) = bytes.split_at(end);
    // Git index.skipHash writes a zero footer. Neither kind of checksum is authentication.
    if checksum.iter().any(|byte| *byte != 0) {
        let expected = if width == 20 {
            sha1::Sha1::digest(payload).to_vec()
        } else {
            Sha256::digest(payload).to_vec()
        };
        if checksum != expected {
            bail!("Git index checksum mismatch");
        }
    }
    let mut cursor = Cursor {
        bytes: payload,
        offset: 8,
    };
    let count = cursor.u32()? as usize;
    let minimum = (40 + width + 2 + 1).next_multiple_of(8);
    if count > payload.len().saturating_sub(12) / minimum {
        bail!("Git index entry count exceeds its bytes");
    }
    let mut result = Index {
        links: BTreeSet::new(),
        gitlinks: BTreeSet::new(),
    };
    let mut previous: &[u8] = b"";
    let mut paths = BTreeSet::new();
    for _ in 0..count {
        let (path, mode) = entry(&mut cursor, version, width)?;
        if path <= previous {
            bail!("Git index paths must be unique and sorted at stage zero");
        }
        previous = path;
        paths.insert(path.to_vec());
        match mode {
            0o120000 => {
                result.links.insert(path.to_vec());
            }
            0o160000 => {
                result.gitlinks.insert(path.to_vec());
            }
            0o100644 | 0o100755 => {}
            _ => bail!("Unsupported Git index mode"),
        }
    }
    // One ordered search per path avoids rescanning every long directory prefix.
    for path in &paths {
        let mut prefix = path.clone();
        prefix.push(b'/');
        if paths
            .range(prefix.clone()..)
            .next()
            .is_some_and(|child| child.starts_with(&prefix))
        {
            bail!("Git index has a file/directory path conflict");
        }
    }
    while cursor.offset < payload.len() {
        let signature = cursor.take(4)?;
        let length = cursor.u32()? as usize;
        if !signature[0].is_ascii_uppercase() {
            bail!("Unsupported mandatory Git index extension");
        }
        cursor.take(length)?;
    }
    Ok(result)
}

fn entry<'a>(cursor: &mut Cursor<'a>, version: u32, width: usize) -> Result<(&'a [u8], u32)> {
    let start = cursor.offset;
    let stat = cursor.take(40)?;
    let mode = u32::from_be_bytes(stat[24..28].try_into()?);
    cursor.take(width)?;
    let flags = cursor.u16()?;
    if flags & 0x3000 != 0 {
        bail!("Unresolved Git index entry");
    }
    if flags & 0x4000 != 0 && (version == 2 || cursor.u16()? & !0x6000 != 0) {
        bail!("Unsupported Git index entry flags");
    }
    let name_start = cursor.offset;
    let length = cursor.bytes[name_start..]
        .iter()
        .position(|byte| *byte == 0)
        .context("Unterminated Git index path")?;
    let path = cursor.take(length)?;
    if path.is_empty()
        || (flags as usize & 0xfff) != length.min(0xfff)
        || path
            .split(|byte| *byte == b'/')
            .any(|part| part.is_empty() || matches!(part, b"." | b".." | b".git"))
    {
        bail!("Invalid Git index path");
    }
    let padding = 8 - (cursor.offset - start) % 8;
    if cursor.take(padding)?.iter().any(|byte| *byte != 0) {
        bail!("Invalid Git index entry padding");
    }
    Ok((path, mode))
}

#[cfg(test)]
#[path = "git_index_tests.rs"]
mod tests;
