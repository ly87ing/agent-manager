#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use windows_sys::Win32::Storage::FileSystem::*;

fn identity(file: &File) -> FILE_ID_INFO {
    let mut info = std::mem::MaybeUninit::<FILE_ID_INFO>::uninit();
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                info.as_mut_ptr().cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        },
        0,
        "File identity query failed: {}",
        std::io::Error::last_os_error()
    );
    unsafe { info.assume_init() }
}

#[test]
fn file_id_reference_allows_ancestor_rename_while_preserving_identity() {
    let root = std::env::temp_dir().join(format!("am-directory-id-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let parent = root.join("parent");
    let child = parent.join("child");
    std::fs::create_dir_all(&child).unwrap();
    let open = |path: &std::path::Path| {
        OpenOptions::new()
            .read(true)
            .access_mode(FILE_READ_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .unwrap()
    };
    let original = open(&child);
    let expected = identity(&original);
    let descriptor = FILE_ID_DESCRIPTOR {
        dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32,
        Type: ExtendedFileIdType,
        Anonymous: FILE_ID_DESCRIPTOR_0 {
            ExtendedFileId: expected.FileId,
        },
    };
    let handle = unsafe {
        OpenFileById(
            original.as_raw_handle(),
            &descriptor,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )
    };
    assert_ne!(
        handle,
        windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
        "ID reference failed: {}",
        std::io::Error::last_os_error()
    );
    let reference = unsafe { File::from_raw_handle(handle) };
    let actual = identity(&reference);
    assert_eq!(actual.VolumeSerialNumber, expected.VolumeSerialNumber);
    assert_eq!(actual.FileId.Identifier, expected.FileId.Identifier);
    let metadata = reference.metadata().unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT, 0);
    drop(original);
    let moved = root.join("moved");
    std::fs::rename(&parent, &moved).unwrap();
    let actual = identity(&open(&moved.join("child")));
    assert_eq!(actual.FileId.Identifier, expected.FileId.Identifier);
    std::fs::create_dir_all(&child).unwrap();
    assert_ne!(
        identity(&open(&child)).FileId.Identifier,
        expected.FileId.Identifier
    );
    drop(reference);
    std::fs::remove_dir_all(root).unwrap();
}
