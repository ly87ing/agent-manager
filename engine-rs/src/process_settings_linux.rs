//! Restrict the verified GSettings backends' file mutation paths during reads.
#[cfg(test)]
use std::{process::Stdio, time::Duration};

pub(crate) fn deny_settings_file_writes(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;

    // GLib keyfile/dconf getters can create directories and writable files.
    // Constrain only this child before loading its original backend. ABI 3 adds
    // TRUNCATE; unsupported kernels fail the read instead of running unguarded.
    unsafe {
        command.pre_exec(|| {
            let abi = libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<u8>(),
                0usize,
                1u32,
            );
            if abi < 3 {
                return Err(std::io::Error::from_raw_os_error(libc::ENOTSUP));
            }
            #[repr(C)]
            struct Ruleset {
                handled_access_fs: u64,
            }
            // WRITE_FILE, REMOVE/MAKE_*, REFER and TRUNCATE; reads/exec remain
            // allowed. This does not restrict metadata or existing writable FDs.
            let rules = Ruleset {
                handled_access_fs: 0x7ff2,
            };
            let fd = libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &rules,
                std::mem::size_of::<Ruleset>(),
                0u32,
            );
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let result = if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0
                || libc::syscall(libc::SYS_landlock_restrict_self, fd, 0u32) < 0
            {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            };
            libc::close(fd as libc::c_int);
            result
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(
        path: &std::path::Path,
    ) -> std::collections::BTreeMap<std::path::PathBuf, (std::time::SystemTime, Option<Vec<u8>>)>
    {
        let mut result = std::collections::BTreeMap::new();
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.is_symlink());
            result.insert(
                path.clone(),
                (
                    metadata.modified().unwrap(),
                    metadata.is_file().then(|| std::fs::read(&path).unwrap()),
                ),
            );
            if metadata.is_dir() {
                result.extend(tree(&path));
            }
        }
        result
    }

    #[test]
    fn guarded_dconf_reads_preserve_nondefault_values_and_locks_without_runtime_writes() {
        use std::os::unix::fs::PermissionsExt;
        assert!(
            !std::path::Path::new(&format!("/run/dconf/user/{}", unsafe { libc::geteuid() }))
                .exists(),
            "A mandatory dconf profile prevents this isolated fixture"
        );
        let root = std::env::temp_dir().join(format!("am-dconf-read-{}", crate::store::uuid_v4()));
        let config = root.join("config");
        let runtime = root.join("runtime");
        let user = root.join("user.d");
        let system = root.join("system.d");
        for path in [
            &runtime,
            &user,
            &system.join("locks"),
            &config.join("dconf"),
        ] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(user.join("00-settings"), "[org/gnome/desktop/datetime]\nautomatic-timezone=true\n[org/gnome/system/locale]\nregion='fr_FR.UTF-8'\n").unwrap();
        std::fs::write(
            system.join("00-settings"),
            "[org/gnome/desktop/datetime]\nautomatic-timezone=false\n",
        )
        .unwrap();
        std::fs::write(
            system.join("locks/00-lock"),
            "/org/gnome/desktop/datetime/automatic-timezone\n",
        )
        .unwrap();
        let locked = root.join("locked.db");
        for (output, source) in [
            (config.join("dconf/amfixture"), user),
            (locked.clone(), system),
        ] {
            let output = crate::process::launch_command("/usr/bin/dconf")
                .arg("compile")
                .arg(output)
                .arg(source)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
        let profile = root.join("profile");
        std::fs::write(&profile, "user-db:amfixture\n").unwrap();
        let read = |verb: &str, schema: &str, key: &str, guarded: bool| {
            let mut command = crate::process::launch_command("/usr/bin/gsettings");
            command
                .args([verb, schema, key])
                .env("GSETTINGS_BACKEND", "dconf")
                .env("DCONF_PROFILE", &profile)
                .env("XDG_CONFIG_HOME", &config)
                .env("XDG_RUNTIME_DIR", &runtime)
                .env("HOME", root.join("absent-home"))
                // This read must not need a writer or activate a real bus service.
                .env(
                    "DBUS_SESSION_BUS_ADDRESS",
                    format!("unix:path={}/absent-bus", root.display()),
                )
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if guarded {
                deny_settings_file_writes(&mut command);
            }
            let output =
                crate::process::capture_group(&mut command, Duration::from_secs(8), 4096).unwrap();
            assert!(output.status.success(), "{output:?}");
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        };
        let before = tree(&root);
        assert_eq!(
            read(
                "get",
                "org.gnome.desktop.datetime",
                "automatic-timezone",
                true
            ),
            "true"
        );
        assert_eq!(
            read(
                "writable",
                "org.gnome.desktop.datetime",
                "automatic-timezone",
                true
            ),
            "true"
        );
        assert_eq!(
            read("get", "org.gnome.system.locale", "region", true),
            "'fr_FR.UTF-8'"
        );
        assert_eq!(
            read("writable", "org.gnome.system.locale", "region", true),
            "true"
        );
        assert_eq!(tree(&root), before);
        std::fs::write(
            &profile,
            format!("user-db:amfixture\nfile-db:{}\n", locked.display()),
        )
        .unwrap();
        let before = tree(&root);
        assert_eq!(
            read(
                "get",
                "org.gnome.desktop.datetime",
                "automatic-timezone",
                true
            ),
            "false"
        );
        assert_eq!(
            read(
                "writable",
                "org.gnome.desktop.datetime",
                "automatic-timezone",
                true
            ),
            "false"
        );
        assert_eq!(
            read("get", "org.gnome.system.locale", "region", true),
            "'fr_FR.UTF-8'"
        );
        assert_eq!(tree(&root), before);
        assert!(!runtime.join("dconf").exists());
        assert_eq!(
            read("writable", "org.gnome.system.locale", "region", true),
            "true"
        );
        assert_eq!(tree(&root), before);
        read("get", "org.gnome.system.locale", "region", false);
        assert!(
            runtime.join("dconf").exists(),
            "The unguarded positive control did not exercise runtime creation"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn settings_read_child_denies_file_mutation_without_restricting_its_parent() {
        let root =
            std::env::temp_dir().join(format!("am-settings-read-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&root).unwrap();
        let file = root.join("original");
        std::fs::write(&file, "original contents").unwrap();
        let mut command = crate::process::launch_command("/bin/sh");
        command
            .args([
                "-eu",
                "-c",
                "test -w \"$1\"; test -w \"$1/original\"; cat \"$1/original\"; \
                 ! mkdir \"$1/new\"; ! sh -c 'echo changed > \"$1/original\"' sh \"$1\"; \
                 ! truncate -s 0 \"$1/original\"; ! rm \"$1/original\"; \
                 ! mv \"$1/original\" \"$1/renamed\"",
                "sh",
            ])
            .arg(&root)
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        deny_settings_file_writes(&mut command);
        let output = crate::process::capture_group(&mut command, Duration::from_secs(8), 4096)
            .expect("Native Linux verification requires Landlock ABI >= 3");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"original contents");
        assert_eq!(std::fs::read(&file).unwrap(), b"original contents");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        // The caller never enters the child's irreversible restriction domain.
        std::fs::write(root.join("parent-control"), "allowed").unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }
}
