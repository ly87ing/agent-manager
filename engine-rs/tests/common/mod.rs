//! Windows fixtures satisfy the production native-executable and private-source contracts.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Restrict only a newly created direct child of the process's temporary directory.
/// Inheritable ACEs also protect credentials created later by atomic replacement.
pub fn private_fixture_root(root: &Path) {
    use agent_manager_engine::platform::Canon;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Security::*,
        Storage::FileSystem::FILE_ALL_ACCESS,
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    assert_eq!(
        root.parent().unwrap().canon().unwrap(),
        std::env::temp_dir().canon().unwrap(),
        "fixture ACL changes must stay inside a new temporary root"
    );
    assert!(!root.exists(), "fixture root must be newly created");
    std::fs::create_dir(root).unwrap();
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut token = std::ptr::null_mut();
        assert_ne!(
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token),
            0
        );
        let mut needed = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        assert!(needed > 0 && needed <= 64 * 1024);
        let mut user_bytes = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        let read = GetTokenInformation(
            token,
            TokenUser,
            user_bytes.as_mut_ptr().cast(),
            needed,
            &mut needed,
        );
        CloseHandle(token);
        assert_ne!(read, 0);
        let sid = (*(user_bytes.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        assert_ne!(IsValidSid(sid), 0);
        let acl_len = std::mem::size_of::<ACL>() + std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            - std::mem::size_of::<u32>()
            + GetLengthSid(sid) as usize;
        let mut acl_bytes = vec![0u32; acl_len.div_ceil(std::mem::size_of::<u32>())];
        let acl = acl_bytes.as_mut_ptr().cast::<ACL>();
        assert_ne!(
            InitializeAcl(acl, (acl_bytes.len() * 4) as u32, ACL_REVISION),
            0
        );
        assert_ne!(
            AddAccessAllowedAceEx(
                acl,
                ACL_REVISION,
                OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
                FILE_ALL_ACCESS,
                sid
            ),
            0
        );
        let mut descriptor: SECURITY_DESCRIPTOR = std::mem::zeroed();
        let descriptor_ptr = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
        assert_ne!(InitializeSecurityDescriptor(descriptor_ptr, 1), 0);
        assert_ne!(SetSecurityDescriptorDacl(descriptor_ptr, 1, acl, 0), 0);
        assert_ne!(
            SetFileSecurityW(
                wide.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor_ptr
            ),
            0,
            "cannot protect the isolated fixture directory"
        );
    }
}

/// Build one std-only native fixture per integration-test process, never a shell wrapper.
pub fn install_fake_account_clis(home: &Path) {
    static NATIVE_CLI: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let exe = NATIVE_CLI.get_or_init(|| {
        let root = std::env::temp_dir().join(format!(
            "am-native-fake-cli-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        private_fixture_root(&root);
        let exe = root.join("fake-account-cli.exe");
        let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let mut child = std::process::Command::new(compiler)
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/common/fake_account_cli.rs"))
            .args([
                "--crate-name",
                "fake_account_cli",
                "--edition=2021",
                "-C",
                "debuginfo=0",
                "-o",
            ])
            .arg(&exe)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("the installed Rust toolchain must build the native fixture");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "native fake CLI compilation failed: {status}"
                );
                break;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("native fake CLI compilation exceeded its fixture deadline");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        exe
    });
    let bin = home.join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in ["claude.exe", "codex.exe"] {
        std::fs::copy(exe, bin.join(name)).unwrap();
    }
}
