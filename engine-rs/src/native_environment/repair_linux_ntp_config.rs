//! Bounded standard timesyncd configuration evidence; no filesystem writes.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, Metadata},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) const NAME: &str = "90-agent-manager.conf";
const ROOTS: [&str; 4] = [
    "etc/systemd",
    "run/systemd",
    "usr/local/lib/systemd",
    "usr/lib/systemd",
];
const DROPINS: &str = "timesyncd.conf.d";
const LIMIT: usize = 4 * 1024 * 1024;
const HEADER: &str = "# Agent Manager manual NTP configuration v1\n[Time]\nNTP=\nNTP=";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Identity {
    pub dev: u64,
    pub ino: u64,
    uid: u32,
    gid: u32,
    mode: u32,
}
impl Identity {
    fn read(meta: &Metadata, uid: u32) -> Option<Self> {
        (meta.uid() == uid && meta.mode() & 0o022 == 0).then_some(Self {
            dev: meta.dev(),
            ino: meta.ino(),
            uid: meta.uid(),
            gid: meta.gid(),
            mode: meta.mode(),
        })
    }
}

pub(super) use super::repair_linux_ntp_metadata::plain_metadata;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Content {
    pub identity: Identity,
    len: u64,
    mtime: (i64, i64),
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct State {
    pub directories: BTreeMap<String, Option<Identity>>,
    pub names: BTreeMap<String, Vec<String>>,
    pub files: BTreeMap<String, Option<Content>>,
    pub saved: Option<String>,
}

impl State {
    pub(super) fn directory_missing(&self) -> bool {
        self.directories
            .get(&format!("{}/{DROPINS}", ROOTS[0]))
            .is_some_and(Option::is_none)
    }

    pub(super) fn file_policy(&self) -> Option<(u32, u32)> {
        let key = format!("{}/{DROPINS}/{NAME}", ROOTS[0]);
        match self.files.get(&key)?.as_ref() {
            Some(file) => Some((file.identity.mode & 0o7777, file.identity.gid)),
            None => Some((
                0o644,
                self.directories
                    .get(&format!("{}/{DROPINS}", ROOTS[0]))
                    .and_then(Option::as_ref)
                    .or_else(|| self.directories.get(ROOTS[0]).and_then(Option::as_ref))?
                    .gid,
            )),
        }
    }

    pub(super) fn with_directory(&self, directory: Identity) -> Option<Self> {
        if !self.directory_missing()
            || directory.uid != self.directories.get(ROOTS[0])?.as_ref()?.uid
            || directory.gid != self.directories.get(ROOTS[0])?.as_ref()?.gid
            || directory.mode & 0o7777 != 0o755
        {
            return None;
        }
        let mut after = self.clone();
        after
            .directories
            .insert(format!("{}/{DROPINS}", ROOTS[0]), Some(directory));
        Some(after)
    }

    // Construct from the original baseline and narrow owned receipts, never by
    // accepting an entire fresh filesystem snapshot after publication.
    pub(super) fn with_file(&self, file: Content, server: &str) -> Option<Self> {
        let text = generated(server)?;
        let (mode, gid) = self.file_policy()?;
        let directory = format!("{}/{DROPINS}", ROOTS[0]);
        let uid = self.directories.get(&directory)?.as_ref()?.uid;
        if file.digest != crate::native_mcp::model::sha256_hex(&text)
            || file.len != text.len() as u64
            || file.identity.uid != uid
            || file.identity.gid != gid
            || file.identity.mode & 0o7777 != mode
        {
            return None;
        }
        let mut after = self.clone();
        after
            .files
            .insert(format!("{directory}/{NAME}"), Some(file));
        let names = after.names.get_mut(&directory)?;
        if !names.iter().any(|name| name == NAME) {
            names.push(NAME.into());
            names.sort();
        }
        after.saved = saved(&text);
        Some(after)
    }
}

pub(super) fn record_directory(file: &File, uid: u32) -> Option<Identity> {
    if !plain_metadata(file) {
        return None;
    }
    let meta = file.metadata().ok()?;
    meta.is_dir().then_some(())?;
    Identity::read(&meta, uid)
}

pub(super) fn record_file(file: &File, uid: u32, expected: &str) -> Option<Content> {
    if !plain_metadata(file) {
        return None;
    }
    let before = file.metadata().ok()?;
    let identity = Identity::read(&before, uid)?;
    if !before.is_file() || before.nlink() != 1 || before.len() != expected.len() as u64 {
        return None;
    }
    let mut input = file.try_clone().ok()?;
    input.seek(SeekFrom::Start(0)).ok()?;
    let mut bytes = Vec::new();
    input
        .take(before.len().checked_add(1)?)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes != expected.as_bytes() || !same_file(&before, &file.metadata().ok()?) {
        return None;
    }
    Some(Content {
        identity,
        len: before.len(),
        mtime: (before.mtime(), before.mtime_nsec()),
        digest: crate::native_mcp::model::sha256_hex_bytes(&bytes),
    })
}

pub(super) fn generated(server: &str) -> Option<String> {
    super::time_probe::valid_host(server).then(|| {
        format!(
            "{HEADER}{}\n",
            server.trim_end_matches('.').to_ascii_lowercase()
        )
    })
}

fn saved(text: &str) -> Option<String> {
    let server = text.strip_prefix(HEADER)?.strip_suffix('\n')?;
    (generated(server).as_deref() == Some(text)).then(|| server.to_string())
}

// A deliberately finite syntax admission, not a general systemd parser. In
// particular a malformed preceding section can prevent reading the drop-in.
fn admitted(text: &str) -> bool {
    if text.contains('\0') {
        return false;
    }
    let mut section = false;
    for line in text.lines() {
        let line = line.trim_matches([' ', '\t', '\r']);
        if line.len() > 4096 || line.ends_with('\\') {
            return false;
        }
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if line == "[Time]" {
            section = true;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        let key = key.trim_matches([' ', '\t']);
        if !section
            || !matches!(
                key,
                "NTP"
                    | "FallbackNTP"
                    | "RootDistanceMaxSec"
                    | "PollIntervalMinSec"
                    | "PollIntervalMaxSec"
                    | "ConnectionRetrySec"
                    | "SaveIntervalSec"
            )
            || !value.is_ascii()
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() && byte != b'\t')
            || value.contains(['\\', '"', '\''])
        {
            return false;
        }
        if matches!(key, "NTP" | "FallbackNTP")
            && !value
                .split_ascii_whitespace()
                .all(super::time_probe::valid_host)
        {
            return false;
        }
    }
    true
}

struct Reader {
    anchor: PathBuf,
    uid: u32,
    remaining: usize,
    started: Instant,
    directories: BTreeMap<String, Option<Identity>>,
}
impl Reader {
    fn spend(&mut self, bytes: usize) -> Option<()> {
        self.remaining = self.remaining.checked_sub(bytes)?;
        (self.started.elapsed() < Duration::from_secs(8)).then_some(())
    }

    fn exact(&mut self, path: &Path) -> Option<()> {
        let requested = path.file_name()?;
        for entry in std::fs::read_dir(path.parent()?).ok()? {
            let name = entry.ok()?.file_name();
            self.spend(name.as_encoded_bytes().len().checked_add(1)?)?;
            if name == requested {
                return Some(());
            }
        }
        None
    }

    fn directory(&mut self, relative: &str) -> Option<bool> {
        let mut current = self.anchor.clone();
        for component in Path::new(relative).components() {
            current.push(component);
            let key = current
                .strip_prefix(&self.anchor)
                .ok()?
                .to_str()?
                .to_string();
            if let Some(identity) = self.directories.get(&key) {
                if identity.is_none() {
                    return Some(false);
                }
                continue;
            }
            self.spend(key.len())?;
            let identity = match std::fs::symlink_metadata(&current) {
                Ok(meta) if meta.is_dir() => {
                    self.exact(&current)?;
                    Some(Identity::read(&meta, self.uid)?)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                _ => return None,
            };
            let exists = identity.is_some();
            self.directories.insert(key, identity);
            if !exists {
                return Some(false);
            }
        }
        Some(true)
    }

    fn names(&mut self, relative: &str) -> Option<Vec<String>> {
        if !self.directory(relative)? {
            return Some(vec![]);
        }
        let mut names = Vec::new();
        for entry in std::fs::read_dir(self.anchor.join(relative)).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name();
            let name = name.to_str()?;
            self.spend(name.len().checked_add(1)?)?;
            if name.eq_ignore_ascii_case(NAME) && name != NAME {
                return None;
            }
            if name.ends_with(".conf") {
                if !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
                {
                    return None;
                }
                names.push(name.to_string());
            }
        }
        names.sort();
        Some(names)
    }

    fn content(&mut self, relative: &str) -> Option<Option<(Content, String)>> {
        self.spend(relative.len())?;
        if !self.directory(Path::new(relative).parent()?.to_str()?)? {
            return Some(None);
        }
        let path = self.anchor.join(relative);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && meta.nlink() == 1 => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(None),
            _ => return None,
        };
        self.exact(&path)?;
        let identity = Identity::read(&meta, self.uid)?;
        self.spend(usize::try_from(meta.len()).ok()?)?;
        let mut input = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&path)
            .ok()?;
        let opened = input.metadata().ok()?;
        if relative == format!("{}/{DROPINS}/{NAME}", ROOTS[0]) && !plain_metadata(&input) {
            return None;
        }
        if Identity::read(&opened, self.uid)? != identity
            || !opened.is_file()
            || opened.nlink() != 1
        {
            return None;
        }
        let mut bytes = Vec::new();
        (&mut input)
            .take(meta.len().checked_add(1)?)
            .read_to_end(&mut bytes)
            .ok()?;
        let after = input.metadata().ok()?;
        let current = std::fs::symlink_metadata(&path).ok()?;
        if bytes.len() as u64 != meta.len()
            || !same_file(&meta, &after)
            || !same_file(&meta, &current)
        {
            return None;
        }
        self.spend(0)?;
        let digest = crate::native_mcp::model::sha256_hex_bytes(&bytes);
        Some(Some((
            Content {
                identity,
                len: meta.len(),
                mtime: (meta.mtime(), meta.mtime_nsec()),
                digest,
            },
            String::from_utf8(bytes).ok()?,
        )))
    }
}

fn same_file(a: &Metadata, b: &Metadata) -> bool {
    a.is_file()
        && b.is_file()
        && a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.uid() == b.uid()
        && a.gid() == b.gid()
        && a.mode() == b.mode()
        && a.len() == b.len()
        && a.nlink() == 1
        && b.nlink() == 1
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}

#[cfg(target_os = "linux")]
pub(super) fn read() -> Option<State> {
    read_at(Path::new("/"), 0)
}

pub(super) fn read_at(anchor: &Path, uid: u32) -> Option<State> {
    let anchor_meta = std::fs::symlink_metadata(anchor).ok()?;
    let anchor_identity = Identity::read(&anchor_meta, uid)?;
    if !anchor_meta.is_dir() {
        return None;
    }
    let mut reader = Reader {
        anchor: anchor.into(),
        uid,
        remaining: LIMIT,
        started: Instant::now(),
        directories: BTreeMap::new(),
    };
    if !reader.directory(ROOTS[0])? {
        return None;
    }
    let mut files = BTreeMap::new();
    for (index, root) in ROOTS.iter().enumerate() {
        reader.directory(root)?;
        let key = format!("{root}/timesyncd.conf");
        let content = reader.content(&key)?;
        // v255 reads only the compile-time /etc main file; later versions search
        // other layers. Refuse additional main files until that graph is supported.
        if index != 0 && content.is_some() {
            return None;
        }
        if let Some((_, text)) = &content {
            if !admitted(text) {
                return None;
            }
        }
        files.insert(key, content.map(|(content, _)| content));
    }
    let mut names = BTreeMap::new();
    let mut selected = BTreeMap::new();
    for (index, root) in ROOTS.iter().enumerate() {
        let dir = format!("{root}/{DROPINS}");
        let entries = reader.names(&dir)?;
        for name in &entries {
            if (index != 0 && name == NAME) || name.as_str() > NAME {
                return None;
            }
            selected
                .entry(name.clone())
                .or_insert_with(|| format!("{dir}/{name}"));
        }
        names.insert(dir, entries);
    }
    let mut saved_server = None;
    for (name, key) in selected {
        let (content, text) = reader.content(&key)??;
        if !admitted(&text) {
            return None;
        }
        if name == NAME {
            if !matches!(content.identity.mode & 0o7777, 0o600 | 0o640 | 0o644) {
                return None;
            }
            saved_server = Some(saved(&text)?);
        }
        files.insert(key, Some(content));
    }
    files
        .entry(format!("{}/{DROPINS}/{NAME}", ROOTS[0]))
        .or_insert(None);
    let destination = if reader
        .directories
        .get(&format!("{}/{DROPINS}", ROOTS[0]))
        .is_some_and(Option::is_some)
    {
        format!("{}/{DROPINS}", ROOTS[0])
    } else {
        ROOTS[0].into()
    };
    for relative in [ROOTS[0], destination.as_str()] {
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(anchor.join(relative))
            .ok()?;
        if !plain_metadata(&directory) {
            return None;
        }
    }
    for (relative, expected) in &names {
        if reader.names(relative)? != *expected {
            return None;
        }
    }
    for (relative, expected) in &files {
        if reader.content(relative)?.map(|(content, _)| content) != *expected {
            return None;
        }
    }
    // Bind all observed directory nodes again. A later native writer still must
    // freshly compare this entire snapshot under its own cooperative host lock.
    for (relative, identity) in &reader.directories {
        let current = match std::fs::symlink_metadata(anchor.join(relative)) {
            Ok(meta) if meta.is_dir() => Some(Identity::read(&meta, uid)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            _ => return None,
        };
        if current != *identity {
            return None;
        }
    }
    if Identity::read(&std::fs::symlink_metadata(anchor).ok()?, uid)? != anchor_identity {
        return None;
    }
    reader.spend(0)?;
    Some(State {
        directories: reader.directories,
        names,
        files,
        saved: saved_server,
    })
}

#[cfg(test)]
#[path = "repair_linux_ntp_config_tests.rs"]
mod tests;
