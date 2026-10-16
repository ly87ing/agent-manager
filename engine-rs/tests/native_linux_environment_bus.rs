#![cfg(target_os = "linux")]

use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

struct PrivateBus(Child);
impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn inspection_does_not_activate_a_stopped_time_service() {
    let root = Fixture(std::env::temp_dir().join(format!(
        "am-timesync-{}",
        agent_manager_engine::store::uuid_v4()
    )));
    std::fs::create_dir(&root.0).unwrap();
    let services = root.path().join("services");
    std::fs::create_dir(&services).unwrap();
    let marker = root.path().join("activated");
    std::fs::write(
        services.join("org.freedesktop.timesync1.service"),
        format!(
            "[D-BUS Service]\nName=org.freedesktop.timesync1\nExec=/usr/bin/touch \"{}\"\n",
            marker.display()
        ),
    )
    .unwrap();
    let address = format!(
        "unix:abstract=am-timesync-{}",
        agent_manager_engine::store::uuid_v4()
    );
    let config = root.path().join("bus.conf");
    let service_dir = services
        .to_str()
        .unwrap()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    std::fs::write(
        &config,
        format!(
            "<busconfig><type>session</type><listen>{address}</listen><auth>EXTERNAL</auth><servicedir>{service_dir}</servicedir><policy context=\"default\"><allow own=\"*\"/><allow send_destination=\"*\"/><allow receive_sender=\"*\"/></policy></busconfig>"
        ),
    )
    .unwrap();
    // No system/session configuration is included. Only this owned bus can
    // activate the marker fixture; it cannot reach the machine's time daemon.
    let mut bus = PrivateBus(
        Command::new("/usr/bin/dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--nopidfile", "--print-address=1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = bus.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(result.map(|_| line));
    });
    let ready = rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    assert!(ready.trim().starts_with(&address));

    let home = root.path().join("missing-home");
    let data = root.path().join("missing-data");
    let output = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .args(["environment:check", "--json"])
        .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
        .env("HOME", &home)
        .env("AGENT_MANAGER_DATA_DIR", &data)
        .env("XDG_CURRENT_DESKTOP", "")
        .output()
        .unwrap();
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{result}");
    assert_eq!(result["ok"], true);
    assert!(!home.exists());
    assert!(!data.exists());
    assert!(!marker.exists(), "Inspection activated the time service");

    // A positive control prevents a broken activation fixture from passing the
    // negative assertion. Its only action is creating a disposable marker.
    let control = Command::new("/usr/bin/busctl")
        .args([
            "--system",
            "--auto-start=yes",
            "--timeout=1",
            "call",
            "org.freedesktop.timesync1",
            "/org/freedesktop/timesync1",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            "s",
            "org.freedesktop.timesync1.Manager",
        ])
        .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
        .output()
        .unwrap();
    assert!(!control.status.success());
    assert!(
        marker.exists(),
        "The activation positive control did not run"
    );
}

struct Accounts {
    uid: u32,
    path: zbus::zvariant::OwnedObjectPath,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
#[zbus::interface(name = "org.freedesktop.Accounts")]
impl Accounts {
    fn find_user_by_id(&self, uid: i64) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
        assert_eq!(uid, self.uid as i64, "Only the caller may be inspected");
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(self.path.clone())
    }
}

struct User {
    state: std::sync::Arc<std::sync::Mutex<(u64, String)>>,
    extra_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
#[zbus::interface(name = "org.freedesktop.Accounts.User")]
impl User {
    #[zbus(property)]
    fn uid(&self) -> u64 {
        self.state.lock().unwrap().0
    }
    #[zbus(property)]
    fn language(&self) -> String {
        self.state.lock().unwrap().1.clone()
    }
    #[zbus(property)]
    fn languages(&self) -> Vec<String> {
        self.extra_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        vec![self.language(), "de_DE.UTF-8".into()]
    }
}

#[test]
fn gnome_inspection_does_not_promote_installer_locales_or_unknown_user_evidence() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    let root = Fixture(std::env::temp_dir().join(format!(
        "am-gnome-language-{}",
        agent_manager_engine::store::uuid_v4()
    )));
    std::fs::create_dir(&root.0).unwrap();
    let address = format!(
        "unix:abstract=am-gnome-language-{}",
        agent_manager_engine::store::uuid_v4()
    );
    let config = root.path().join("bus.conf");
    std::fs::write(&config, format!("<busconfig><type>session</type><listen>{address}</listen><auth>EXTERNAL</auth><policy context=\"default\"><allow own=\"*\"/><allow send_destination=\"*\"/><allow receive_sender=\"*\"/></policy></busconfig>")).unwrap();
    let mut bus = PrivateBus(
        Command::new("/usr/bin/dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--nopidfile", "--print-address=1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = bus.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = tx.send(BufReader::new(stdout).read_line(&mut line).map(|_| line));
    });
    let ready = rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    assert!(ready.trim().starts_with(&address));
    let uid = unsafe { libc::geteuid() };
    let path =
        zbus::zvariant::OwnedObjectPath::try_from(format!("/org/freedesktop/Accounts/User{uid}"))
            .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let extra_reads = Arc::new(AtomicUsize::new(0));
    let state = Arc::new(Mutex::new((uid as u64, "en_US.UTF-8".to_string())));
    let _connection = zbus::blocking::connection::Builder::address(ready.trim())
        .unwrap()
        .name("org.freedesktop.Accounts")
        .unwrap()
        .serve_at(
            "/org/freedesktop/Accounts",
            Accounts {
                uid,
                path: path.clone(),
                calls: calls.clone(),
            },
        )
        .unwrap()
        .serve_at(
            path,
            User {
                state: state.clone(),
                extra_reads: extra_reads.clone(),
            },
        )
        .unwrap()
        .build()
        .unwrap();
    let initial_extra_reads = extra_reads.load(Ordering::Relaxed);
    let home = root.path().join("missing-home");
    let data = root.path().join("missing-data");
    let settings = root.path().join("settings");
    let inspect = |desktop: &str| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"));
        command
            .args(["environment:check", "--json"])
            .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
            .env("HOME", &home)
            .env("AGENT_MANAGER_DATA_DIR", &data)
            .env("XDG_CURRENT_DESKTOP", desktop)
            .env("GSETTINGS_BACKEND", "keyfile")
            .env("XDG_CONFIG_HOME", &settings)
            .env("LANG", "de_DE.UTF-8");
        let output = command.output().unwrap();
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(output.status.success(), "{result}");
        assert_eq!(result["ok"], true);
        assert!(!home.exists());
        assert!(!data.exists());
        result["data"]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["id"].as_str().unwrap().to_string(),
                    row["observed"].clone(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let rows = inspect("GNOME");
    assert_eq!(rows["language.preferred"], "en-US");
    assert_eq!(rows["language.region"], "US");
    assert!(
        !settings.exists(),
        "A read-only GSettings call created configuration"
    );
    assert_eq!(
        extra_reads.load(Ordering::Relaxed),
        initial_extra_reads,
        "Installer interests were queried as desktop language order"
    );
    let control = Command::new("/usr/bin/busctl")
        .args([
            "--system",
            "--json=short",
            "get-property",
            "org.freedesktop.Accounts",
            &format!("/org/freedesktop/Accounts/User{uid}"),
            "org.freedesktop.Accounts.User",
            "Languages",
        ])
        .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
        .output()
        .unwrap();
    assert!(control.status.success());
    let control: serde_json::Value = serde_json::from_slice(&control.stdout).unwrap();
    assert_eq!(
        control["data"],
        serde_json::json!(["en_US.UTF-8", "de_DE.UTF-8"])
    );
    assert_eq!(
        extra_reads.load(Ordering::Relaxed),
        initial_extra_reads + 1,
        "Installer metadata positive control did not run"
    );
    state.lock().unwrap().0 = uid as u64 + 1;
    let rows = inspect("GNOME");
    assert!(rows["language.preferred"].is_null());
    assert!(rows["language.region"].is_null());
    *state.lock().unwrap() = (uid as u64, "sr_RS@latin".into());
    let rows = inspect("GNOME");
    assert!(
        rows["language.preferred"].is_null(),
        "A script modifier was discarded"
    );
    assert_eq!(
        rows["language.region"], "RS",
        "Known formats evidence was lost with unknown UI language"
    );
    let prior = calls.load(Ordering::Relaxed);
    let rows = inspect("KDE");
    assert!(rows["language.preferred"].is_null());
    assert_eq!(
        calls.load(Ordering::Relaxed),
        prior,
        "Another desktop inherited GNOME's account defaults"
    );
}
