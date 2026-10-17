//! Admit default destination policy; never copy opaque filesystem metadata.
use std::fs::File;
use std::os::fd::AsRawFd;

// Linux UAPI linux/fs.h: FS_IOC_FSGETXATTR. Layout counters are observations,
// while flags, allocation hints and project identity are replacement policy.
#[derive(Default)]
#[repr(C)]
struct Fsxattr {
    xflags: u32,
    extsize: u32,
    nextents: u32,
    projid: u32,
    cowextsize: u32,
    pad: [u8; 8],
}

fn default_ext4_policy(directory: bool, flags: u32, attributes: &Fsxattr) -> bool {
    let structural = 0x0008_0000 | if directory { 0x0000_1000 } else { 0 };
    flags & !structural == 0
        && attributes.xflags == 0
        && attributes.extsize == 0
        && attributes.projid == 0
        && attributes.cowextsize == 0
        && attributes.pad == [0; 8]
}

#[cfg(target_os = "linux")]
fn bounded_text(path: &str, limit: usize) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= limit).then_some(())?;
    String::from_utf8(bytes).ok()
}

#[cfg(target_os = "linux")]
fn mount_id(fd: i32) -> Option<u64> {
    let text = bounded_text(&format!("/proc/self/fdinfo/{fd}"), 4096)?;
    let mut ids = text
        .lines()
        .filter_map(|line| line.strip_prefix("mnt_id:\t"));
    let id = ids.next()?.parse().ok()?;
    ids.next().is_none().then_some(id)
}

fn ext4_mount(text: &str, id: u64) -> bool {
    let mut found = false;
    for line in text.lines() {
        let Some((mount, filesystem)) = line.split_once(" - ") else {
            return false;
        };
        let mut fields = mount.split_ascii_whitespace();
        let Some(current) = fields.next().and_then(|value| value.parse::<u64>().ok()) else {
            return false;
        };
        if current == id {
            if found || filesystem.split_ascii_whitespace().next() != Some("ext4") {
                return false;
            }
            found = true;
        }
    }
    found
}

#[cfg(target_os = "linux")]
pub(super) fn plain_metadata(file: &File) -> bool {
    use std::os::unix::fs::MetadataExt;
    let fd = file.as_raw_fd();
    let Ok(meta) = file.metadata() else {
        return false;
    };
    if (!meta.is_file() && !meta.is_dir()) || meta.mode() & 0o7000 != 0 {
        return false;
    }
    // EXT*_SUPER_MAGIC alone cannot distinguish the ext2/3/4 implementations.
    // Bind this opened descriptor's mount ID to the kernel's concrete fs type.
    let Some(id) = mount_id(fd) else {
        return false;
    };
    let Some(mounts) = bounded_text("/proc/self/mountinfo", 1024 * 1024) else {
        return false;
    };
    let mut filesystem = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if !ext4_mount(&mounts, id)
        || unsafe { libc::fstatfs(fd, filesystem.as_mut_ptr()) } != 0
        || unsafe { filesystem.assume_init() }.f_type != 0xef53
        || unsafe { libc::flistxattr(fd, std::ptr::null_mut(), 0) } != 0
    {
        return false;
    }
    // The request number encodes long, but VFS transfers an unsigned int.
    let mut flags: u32 = 0;
    let mut attributes = Fsxattr::default();
    (unsafe { libc::ioctl(fd, libc::FS_IOC_GETFLAGS, &mut flags) }) == 0
        && unsafe { libc::ioctl(fd, libc::_IOR::<Fsxattr>(b'X' as u32, 31), &mut attributes) } == 0
        && default_ext4_policy(meta.is_dir(), flags, &attributes)
        && mount_id(fd) == Some(id)
}

#[cfg(target_os = "macos")]
pub(super) fn plain_metadata(file: &File) -> bool {
    // macOS-only filesystem rehearsal: fresh fixture nodes acquire provenance.
    // This does not exercise Linux policy admission or privileged operations.
    let fd = file.as_raw_fd();
    let count = unsafe { libc::flistxattr(fd, std::ptr::null_mut(), 0, 0) };
    if count == 0 {
        return true;
    }
    if count < 0 || count > 64 * 1024 {
        return false;
    }
    let mut names = vec![0u8; count as usize];
    let read = unsafe { libc::flistxattr(fd, names.as_mut_ptr().cast(), names.len(), 0) };
    read == count
        && names.last() == Some(&0)
        && names
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
            .all(|name| name == b"com.apple.provenance")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ext4_structure_and_layout_counts_are_admitted() {
        assert_eq!(std::mem::size_of::<Fsxattr>(), 28);
        for directory in [false, true] {
            let attributes = Fsxattr {
                nextents: 7,
                ..Default::default()
            };
            assert!(default_ext4_policy(directory, 0, &attributes));
            assert!(default_ext4_policy(directory, 0x80000, &attributes));
            for bit in 0..32 {
                let allowed = bit == 19 || (directory && bit == 12);
                assert_eq!(
                    default_ext4_policy(directory, 1 << bit, &attributes),
                    allowed
                );
            }
        }
        for attributes in [
            Fsxattr {
                xflags: 0x80,
                ..Default::default()
            },
            Fsxattr {
                extsize: 1,
                ..Default::default()
            },
            Fsxattr {
                projid: 1,
                ..Default::default()
            },
            Fsxattr {
                cowextsize: 1,
                ..Default::default()
            },
            Fsxattr {
                pad: [1; 8],
                ..Default::default()
            },
        ] {
            assert!(!default_ext4_policy(false, 0, &attributes));
        }
    }

    #[test]
    fn mount_identity_requires_the_concrete_driver() {
        let ext4 = "42 1 8:1 / / rw - ext4 /dev/example rw\n";
        assert!(ext4_mount(ext4, 42));
        assert!(!ext4_mount(ext4, 43));
        assert!(!ext4_mount(&format!("{ext4}{ext4}"), 42));
        for driver in ["ext2", "ext3", "overlay", "xfs", "btrfs", "fuse.ext4"] {
            assert!(!ext4_mount(
                &ext4.replace(" - ext4 ", &format!(" - {driver} ")),
                42
            ));
        }
        assert!(!ext4_mount("incomplete", 42));
    }
}
