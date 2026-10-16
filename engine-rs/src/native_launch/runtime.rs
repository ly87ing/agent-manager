use super::{recipe, Recipe};
use crate::{Context, Request};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
#[cfg(windows)]
#[path = "runtime_windows.rs"]
mod windows;

fn catalog(ctx: &Context) -> Value {
    let mut apps = Vec::<Value>::new();
    if cfg!(target_os = "macos") {
        for root in [
            std::path::PathBuf::from("/Applications"),
            ctx.home_dir.join("Applications"),
            std::path::PathBuf::from("/System/Applications"),
        ] {
            scan_apps(&root, 2, &mut apps);
        }
    }
    #[cfg(target_os = "linux")]
    super::catalog::linux(ctx, &mut apps);
    #[cfg(windows)]
    super::catalog::windows(&mut apps);
    apps.sort_by_key(|app| app["name"].as_str().unwrap_or_default().to_lowercase());
    let mut seen = std::collections::BTreeSet::new();
    apps.retain(|app| seen.insert((app["path"].to_string(), app["args"].to_string())));
    json!({"platform":std::env::consts::OS,
        "shell":if cfg!(windows) {"powershell"} else if cfg!(target_os = "macos") {"zsh"} else {"bash"}, "apps":apps})
}

fn scan_apps(root: &Path, depth: usize, apps: &mut Vec<Value>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "app") && path.is_dir() {
            apps.push(
                json!({"name":path.file_stem().unwrap_or_default().to_string_lossy(),"path":path}),
            );
        } else if depth > 0 && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            scan_apps(&path, depth - 1, apps);
        }
    }
}

fn already_running(target: &Path) -> Result<bool> {
    #[cfg(target_os = "macos")]
    {
        let output = crate::process::launch_command("/bin/ps")
            .args(["-axo", "comm="])
            .output()?;
        if !output.status.success() {
            bail!("Unable to check running applications");
        }
        let prefix = target.join("Contents/MacOS").to_string_lossy().into_owned() + "/";
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.trim().starts_with(&prefix)))
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let selected = std::fs::metadata(target)?;
        for entry in std::fs::read_dir("/proc")? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
                .is_none()
            {
                continue;
            }
            if std::fs::metadata(entry.path().join("exe"))
                .is_ok_and(|image| image.dev() == selected.dev() && image.ino() == selected.ino())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    #[cfg(windows)]
    {
        windows::already_running(target)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = target;
        bail!("Unsupported local platform")
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;

    #[test]
    fn owned_image() {
        let Some(marker) = std::env::var_os("AM_LAUNCH_IMAGE_HELPER") else {
            return;
        };
        let marker = std::path::PathBuf::from(marker);
        std::fs::write(&marker, b"ready").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !marker.with_extension("release").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn native_running_images_use_file_identity_through_aliases_and_exit() {
        struct Owned {
            child: Option<std::process::Child>,
            root: std::path::PathBuf,
        }
        impl Drop for Owned {
            fn drop(&mut self) {
                if let Some(child) = self.child.as_mut() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
        let root =
            std::env::temp_dir().join(format!("am-linux-launch-image-{}", crate::store::uuid_v4()));
        let mut owned = Owned { child: None, root };
        for directory in ["first", "second", "alias"] {
            std::fs::create_dir_all(owned.root.join(directory)).unwrap();
        }
        let first = owned.root.join("first/image-probe");
        let second = owned.root.join("second/image-probe");
        let alias = owned.root.join("alias/other-name");
        let renamed = owned.root.join("first/renamed-image");
        std::fs::copy(std::env::current_exe().unwrap(), &first).unwrap();
        std::fs::copy(&first, &second).unwrap();
        std::fs::hard_link(&first, &alias).unwrap();
        let marker = owned.root.join("ready");
        owned.child = Some(
            crate::process::launch_command(&first)
                .args([
                    "--exact",
                    "native_launch::runtime::linux_tests::owned_image",
                    "--nocapture",
                ])
                .env("AM_LAUNCH_IMAGE_HELPER", &marker)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "Owned ELF did not start"
            );
            assert!(owned.child.as_mut().unwrap().try_wait().unwrap().is_none());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(already_running(&first).unwrap());
        assert!(
            already_running(&alias).unwrap(),
            "A hard link identifies the same running file"
        );
        assert!(
            !already_running(&second).unwrap(),
            "The same name does not identify a different file"
        );
        std::fs::rename(&first, &renamed).unwrap();
        assert!(already_running(&alias).unwrap());
        assert!(already_running(&renamed).unwrap());
        std::fs::write(marker.with_extension("release"), b"release").unwrap();
        assert!(owned.child.as_mut().unwrap().wait().unwrap().success());
        owned.child = None;
        assert!(!already_running(&alias).unwrap());
        assert!(!already_running(&renamed).unwrap());
        assert!(!already_running(&second).unwrap());
    }
}

fn run(ctx: &Context, req: &Request, spec: Recipe) -> Result<Value> {
    if !req.flag("--confirm") {
        bail!("Launching requires explicit --confirm");
    }
    if spec.platform != std::env::consts::OS || spec.kind != "app" {
        bail!("Launch once supports local graphical applications; run command-line tools in a terminal");
    }
    let variant = spec
        .variants
        .iter()
        .find(|v| Some(v.name.as_str()) == req.value("--name"))
        .ok_or_else(|| anyhow::anyhow!("Choose a generated launch name"))?;
    let path = Path::new(&spec.target);
    if !path.is_absolute() {
        bail!("Choose an absolute application path");
    }
    let target = path.canonicalize()?;
    if (cfg!(target_os = "macos")
        && (!target.is_dir() || target.extension().is_none_or(|e| e != "app")))
        || (!cfg!(target_os = "macos") && !target.is_file())
    {
        bail!("Choose an installed application");
    }
    if already_running(&target)? {
        bail!("The application is already running. Quit it normally before launching with a different time zone");
    }
    // Audit the explicit request before starting it. Arguments and paths never enter the audit.
    let db = crate::store::open_write(ctx)?;
    crate::store::record_operation(
        &db,
        Some(&req.run_id),
        "launch:run",
        Some("application"),
        Some(&json!({"name":variant.name,"state":"requested"})),
    )?;
    let mut exit_code = None;
    let result = (|| {
        let mut cmd;
        if cfg!(target_os = "macos") {
            cmd = crate::process::launch_command("/usr/bin/open");
            for (name, value) in spec.environment(variant) {
                cmd.arg("--env").arg(format!("{name}={value}"));
            }
            cmd.arg("-a").arg(&target);
            if !spec.args.is_empty() {
                cmd.arg("--args").args(&spec.args);
            }
            let status = cmd
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;
            exit_code = status.code();
            if !status.success() {
                bail!("The system did not accept the application launch request");
            }
            Ok(json!({"state":"requested","name":variant.name,"timezone_verified":false}))
        } else {
            cmd = crate::process::launch_command(&target);
            cmd.args(&spec.args).envs(spec.environment(variant));
            // GUI applications are independent of the manager's temporary command jobs.
            let mut child = cmd
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let pid = child.id();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(json!({"state":"requested","name":variant.name,"pid":pid,"timezone_verified":false}))
        }
    })();
    let audit_recorded = crate::store::record_operation(
        &db,
        Some(&req.run_id),
        "launch:run",
        Some("application"),
        Some(&json!({"name":variant.name,
            "state":if result.is_ok() {"accepted"} else {"failed"},
            "exit_code":exit_code})),
    )
    .is_ok();
    match result {
        Ok(mut value) => {
            value["audit_recorded"] = json!(audit_recorded);
            Ok(value)
        }
        Err(error) if !audit_recorded => bail!("{error}; the launch result could not be recorded"),
        Err(error) => Err(error),
    }
}

pub(super) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    if req.command.replace('.', ":") == "launch:catalog" {
        if !req.args.is_empty() {
            bail!("catalog accepts no arguments");
        }
        return Ok(catalog(ctx));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut args = req.args.iter();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--spec" | "--name" | "--confirm") || !seen.insert(flag) {
            bail!("Unknown or repeated argument");
        }
        if flag != "--confirm" && args.next().is_none() {
            bail!("Missing argument value");
        }
    }
    run(ctx, req, recipe(req)?)
}
