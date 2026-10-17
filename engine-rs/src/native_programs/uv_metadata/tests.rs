use super::*;
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    site: PathBuf,
    metadata: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("am-uv-metadata-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let site = root.join(if cfg!(windows) {
            "Lib/site-packages"
        } else {
            "lib/python3.13/site-packages"
        });
        let metadata = site.join("tool_a-1.2.3.dist-info/METADATA");
        std::fs::create_dir_all(metadata.parent().unwrap()).unwrap();
        std::fs::write(
            &metadata,
            "Metadata-Version: 2.4\nName: tool-a\nVersion: 1.2.3\n\n",
        )
        .unwrap();
        Self {
            root,
            site,
            metadata,
        }
    }

    fn read(&self) -> Result<String> {
        version(&self.root, "tool-a")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn native_metadata_identity_and_email_headers_are_read_without_execution() {
    let fixture = Fixture::new();
    assert_eq!(fixture.read().unwrap(), "1.2.3");
    let contents = "Metadata-Version: 2.6\r\nName:\r\n TOOL_a\r\nVersion:\r\n v1.2.3\r\nSummary: example\r\n continuation\r\n\r\nVersion: 9.9\r\n";
    std::fs::write(&fixture.metadata, contents).unwrap();
    assert_eq!(fixture.read().unwrap(), "1.2.3");
    assert_eq!(
        std::fs::read_to_string(&fixture.metadata).unwrap(),
        contents
    );
    assert_eq!(std::fs::read_dir(&fixture.root).unwrap().count(), 1);
}

#[test]
fn normalization_and_equivalent_directory_versions_use_pep440() {
    let fixture = Fixture::new();
    let renamed = fixture.site.join("Foo._BAR-1.0.0.dist-info");
    std::fs::rename(fixture.metadata.parent().unwrap(), &renamed).unwrap();
    std::fs::write(
        renamed.join("METADATA"),
        "Metadata-Version: 2.4\nName: FOO---bar\nVersion: v1.0\n\n",
    )
    .unwrap();
    assert_eq!(version(&fixture.root, "foo_bar").unwrap(), "1.0");
    for invalid in ["foo/bar", "-foo", "foo-", "fóo", ""] {
        assert!(version(&fixture.root, invalid).is_err());
    }
}

#[test]
fn directory_names_and_description_body_cannot_supply_missing_identity() {
    let fixture = Fixture::new();
    for content in [
        "Metadata-Version: 2.4\nName: tool-a\nVersion: 9.9\n\n",
        "Metadata-Version: 2.4\nName: different\nVersion: 1.2.3\n\n",
        "Metadata-Version: 2.4\nName: tool-a\n\nVersion: 1.2.3\n",
        "Metadata-Version: 2.4\nVersion: 1.2.3\n\nName: tool-a\n",
        "",
    ] {
        std::fs::write(&fixture.metadata, content).unwrap();
        assert!(fixture.read().is_err(), "Accepted {content:?}");
    }
    std::fs::remove_file(&fixture.metadata).unwrap();
    assert!(fixture.read().is_err());
}

#[test]
fn duplicate_malformed_and_unsupported_identity_headers_are_unknown() {
    let fixture = Fixture::new();
    let valid = "Metadata-Version: 2.4\nName: tool-a\nVersion: 1.2.3\n";
    for extra in [
        "nAmE: tool-a\n",
        "VERSION: 1.2.3\n",
        "Metadata-Version: 2.4\n",
        "Malformed\n",
        "Bad Field: value\n",
        " continuation\n",
        "\0Bad: value\n",
    ] {
        std::fs::write(&fixture.metadata, format!("{valid}{extra}\n")).unwrap();
        assert!(fixture.read().is_err(), "Accepted {extra:?}");
    }
    for content in [
        " Name: tool-a\nVersion: 1.2.3\n",
        "Name: tool-a\nVersion: 1.2.3\n",
        "Metadata-Version: 3.0\nName: tool-a\nVersion: 1.2.3\n",
        "Metadata-Version: 2.bad\nName: tool-a\nVersion: 1.2.3\n",
        "Metadata-Version: 2.4\nName: tool-a\nVersion: latest\n",
        "Metadata-Version: 2.4\nName: tool-a\nVersion: 1.2.3 garbage\n",
    ] {
        std::fs::write(&fixture.metadata, content).unwrap();
        assert!(fixture.read().is_err(), "Accepted {content:?}");
    }
}

#[test]
fn hardlinked_cache_metadata_remains_readable_and_unchanged() {
    let fixture = Fixture::new();
    let cache = fixture.root.join("cache-METADATA");
    std::fs::hard_link(&fixture.metadata, &cache).unwrap();
    let before = std::fs::read(&cache).unwrap();
    assert_eq!(fixture.read().unwrap(), "1.2.3");
    assert_eq!(std::fs::read(&cache).unwrap(), before);
    assert_eq!(std::fs::read(&fixture.metadata).unwrap(), before);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(&cache).unwrap().ino(),
            std::fs::metadata(&fixture.metadata).unwrap().ino()
        );
        assert_eq!(std::fs::metadata(&cache).unwrap().nlink(), 2);
    }
}

#[test]
fn duplicate_and_unreadable_matching_candidates_are_never_skipped() {
    let fixture = Fixture::new();
    let duplicate = fixture.site.join("TOOL_A-2.0.DIST-INFO");
    std::fs::create_dir(&duplicate).unwrap();
    assert!(fixture.read().is_err());
    std::fs::write(
        duplicate.join("METADATA"),
        "Metadata-Version: 2.4\nName: tool-a\nVersion: 2.0\n\n",
    )
    .unwrap();
    assert!(fixture.read().is_err());
    std::fs::remove_file(duplicate.join("METADATA")).unwrap();
    std::fs::remove_dir(&duplicate).unwrap();
    std::fs::write(&duplicate, "not a directory").unwrap();
    assert!(fixture.read().is_err());
}

fn link_directory(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    junction::create(target, link).unwrap();
}

fn unlink_directory(link: &Path) {
    #[cfg(unix)]
    std::fs::remove_file(link).unwrap();
    #[cfg(windows)]
    std::fs::remove_dir(link).unwrap();
}

#[test]
fn linked_roots_intermediates_and_distribution_directories_are_refused() {
    let fixture = Fixture::new();
    let root_link = fixture.root.with_extension("linked");
    link_directory(&fixture.root, &root_link);
    assert!(version(&root_link, "tool-a").is_err());
    unlink_directory(&root_link);
    for original in [
        fixture.site.parent().unwrap().to_path_buf(),
        fixture.site.clone(),
        fixture.metadata.parent().unwrap().to_path_buf(),
    ] {
        let parked = original.with_extension("parked");
        std::fs::rename(&original, &parked).unwrap();
        link_directory(&parked, &original);
        assert!(fixture.read().is_err());
        unlink_directory(&original);
        std::fs::rename(&parked, &original).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn native_interpreter_layouts_are_read_without_interpreter_startup() {
    let fixture = Fixture::new();
    std::fs::rename(
        fixture.root.join("lib/python3.13"),
        fixture.root.join("lib/python3.13t"),
    )
    .unwrap();
    assert_eq!(fixture.read().unwrap(), "1.2.3");
    std::fs::rename(
        fixture.root.join("lib/python3.13t"),
        fixture.root.join("lib/pypy3.10"),
    )
    .unwrap();
    assert_eq!(fixture.read().unwrap(), "1.2.3");
}

#[test]
fn held_directories_cannot_redirect_metadata_to_a_replacement() {
    let fixture = Fixture::new();
    let original = fixture.metadata.parent().unwrap();
    let directory = Directory::open(original).unwrap();
    let parked = original.with_extension("parked");
    #[cfg(unix)]
    {
        std::fs::rename(original, &parked).unwrap();
        std::fs::create_dir(original).unwrap();
        std::fs::write(original.join("METADATA"), "replacement").unwrap();
        let (_, actual) = headers(directory.file(OsStr::new("METADATA")).unwrap()).unwrap();
        assert_eq!(actual.to_string(), "1.2.3");
    }
    #[cfg(windows)]
    {
        assert!(std::fs::rename(original, &parked).is_err());
        assert!(std::fs::rename(
            fixture.site.parent().unwrap(),
            fixture.root.join("replaced-parent")
        )
        .is_err());
        assert_eq!(fixture.read().unwrap(), "1.2.3");
    }
    drop(directory);
    #[cfg(windows)]
    std::fs::rename(original, &parked).unwrap();
}

#[cfg(windows)]
#[test]
fn held_ordinary_directory_refuses_native_junction_conversion_until_released() {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;
    use windows_sys::Win32::System::Ioctl::{FSCTL_GET_REPARSE_POINT, FSCTL_SET_REPARSE_POINT};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    let fixture = Fixture::new();
    let empty = fixture.root.join("empty");
    let template = fixture.root.join("template");
    junction::create(&fixture.site, &template).unwrap();
    let open = |path: &Path, access| {
        std::fs::OpenOptions::new()
            .access_mode(access)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    };
    let source = open(&template, windows_sys::Win32::Foundation::GENERIC_READ).unwrap();
    let mut buffer = vec![0u8; 16 * 1024];
    let mut returned = 0;
    assert_ne!(
        unsafe {
            DeviceIoControl(
                source.as_raw_handle(),
                FSCTL_GET_REPARSE_POINT,
                std::ptr::null(),
                0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        },
        0
    );
    buffer.truncate(returned as usize);
    drop(source);
    let set = |file: &std::fs::File| unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr().cast(),
            buffer.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut 0,
            std::ptr::null_mut(),
        )
    };
    std::fs::create_dir(&empty).unwrap();
    let directory = Directory::open(&empty).unwrap();
    assert!(open(&empty, windows_sys::Win32::Foundation::GENERIC_WRITE).is_err());
    let attributes = open(&empty, FILE_READ_ATTRIBUTES).unwrap();
    assert_eq!(
        set(&attributes),
        0,
        "Attribute-only handle converted a held directory"
    );
    drop(attributes);
    assert!(!junction::exists(&empty).unwrap());
    assert!(directory.entries(1).unwrap().is_empty());
    drop(directory);
    let writer = open(&empty, windows_sys::Win32::Foundation::GENERIC_WRITE).unwrap();
    assert_ne!(
        set(&writer),
        0,
        "Released directory must allow the native setter"
    );
    drop(writer);
    assert!(junction::exists(&empty).unwrap());
    std::fs::remove_dir(&empty).unwrap();
    std::fs::remove_dir(&template).unwrap();
}

#[test]
fn header_budget_ignores_description_body_and_refuses_oversized_headers() {
    let fixture = Fixture::new();
    let mut content = std::fs::read_to_string(&fixture.metadata).unwrap();
    content.push_str(&"Version: 9.9\n".repeat(100_000));
    std::fs::write(&fixture.metadata, content).unwrap();
    assert_eq!(fixture.read().unwrap(), "1.2.3");
    std::fs::write(
        &fixture.metadata,
        format!("Summary: {}", "x".repeat(MAX_HEADERS as usize)),
    )
    .unwrap();
    assert!(fixture.read().is_err());
}

#[test]
fn enumeration_budget_and_ordinary_file_requirement_are_enforced() {
    let fixture = Fixture::new();
    let directory = Directory::open(&fixture.site).unwrap();
    std::fs::write(fixture.site.join("other"), "unrelated").unwrap();
    assert!(directory.entries(1).is_err());
    assert_eq!(directory.entries(2).unwrap().len(), 2);
    drop(directory);
    std::fs::remove_file(&fixture.metadata).unwrap();
    std::fs::create_dir(&fixture.metadata).unwrap();
    assert!(fixture.read().is_err());
}

#[cfg(unix)]
#[test]
fn fifo_and_symlink_metadata_are_refused_without_waiting_for_a_writer() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = Fixture::new();
    let original = fixture.root.join("original");
    std::fs::rename(&fixture.metadata, &original).unwrap();
    std::os::unix::fs::symlink(&original, &fixture.metadata).unwrap();
    assert!(fixture.read().is_err());
    std::fs::remove_file(&fixture.metadata).unwrap();
    let name = std::ffi::CString::new(fixture.metadata.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let started = std::time::Instant::now();
    assert!(fixture.read().is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn additional_python_roots_and_case_sensitive_aliases_are_ambiguous() {
    let fixture = Fixture::new();
    let other = fixture
        .root
        .join("lib/python3.12/site-packages/tool_a-1.2.3.dist-info");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::copy(&fixture.metadata, other.join("METADATA")).unwrap();
    assert!(fixture.read().is_err());
    std::fs::remove_dir_all(fixture.root.join("lib/python3.12")).unwrap();
    let alias = fixture.site.join("TOOL_A-1.2.3.DIST-INFO");
    std::fs::create_dir(&alias).unwrap_or_else(|error| {
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    });
    if alias != fixture.metadata.parent().unwrap()
        && std::fs::read_dir(&fixture.site).unwrap().count() == 2
    {
        assert!(fixture.read().is_err());
    }
    std::fs::create_dir(fixture.site.join("中文内容")).unwrap();
    assert!(version(&fixture.root, "different-tool").is_err());
}
