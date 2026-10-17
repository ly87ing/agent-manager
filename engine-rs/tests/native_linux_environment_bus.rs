#![cfg(target_os = "linux")]

#[path = "support/linux_timesync.rs"]
mod timesync;

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

struct PendingRepair(Option<Child>);
impl Drop for PendingRepair {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct Fixture(std::path::PathBuf);

struct Timedated {
    automatic: std::sync::Arc<std::sync::atomic::AtomicBool>,
    writes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
#[zbus::interface(name = "org.freedesktop.timedate1")]
impl Timedated {
    #[zbus(property, name = "NTP")]
    fn ntp(&self) -> bool {
        self.automatic.load(std::sync::atomic::Ordering::SeqCst)
    }
    #[zbus(name = "SetNTP")]
    fn set_ntp(&self, automatic: bool, _interactive: bool) {
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.automatic
            .store(automatic, std::sync::atomic::Ordering::SeqCst);
    }
}

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
fn inspection_does_not_activate_stopped_time_or_locale_services() {
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
    let locale_marker = root.path().join("locale-activated");
    std::fs::write(
        services.join("org.freedesktop.Accounts.service"),
        format!(
            "[D-BUS Service]\nName=org.freedesktop.Accounts\nExec=/usr/bin/touch \"{}\"\n",
            locale_marker.display()
        ),
    )
    .unwrap();
    let timezone_marker = root.path().join("timezone-activated");
    std::fs::write(
        services.join("org.freedesktop.timedate1.service"),
        format!(
            "[D-BUS Service]\nName=org.freedesktop.timedate1\nExec=/usr/bin/touch \"{}\"\n",
            timezone_marker.display()
        ),
    )
    .unwrap();
    let system_locale_marker = root.path().join("system-locale-activated");
    std::fs::write(
        services.join("org.freedesktop.locale1.service"),
        format!(
            "[D-BUS Service]\nName=org.freedesktop.locale1\nExec=/usr/bin/touch \"{}\"\n",
            system_locale_marker.display()
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
        .env("XDG_CURRENT_DESKTOP", "GNOME")
        .env("XDG_CONFIG_HOME", root.path().join("missing-settings"))
        .env("GSETTINGS_BACKEND", "keyfile")
        .env_remove("GSETTINGS_SCHEMA_DIR")
        .env("DBUS_SESSION_BUS_ADDRESS", ready.trim())
        .output()
        .unwrap();
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{result}");
    assert_eq!(result["ok"], true);
    assert!(!home.exists());
    assert!(!data.exists());
    assert!(!marker.exists(), "Inspection activated the time service");
    assert!(
        !locale_marker.exists(),
        "Inspection activated AccountsService"
    );
    assert!(!root.path().join("missing-settings").exists());
    assert!(
        !timezone_marker.exists(),
        "Inspection activated the timezone service"
    );
    assert!(
        !system_locale_marker.exists(),
        "Inspection activated system-localed"
    );

    let preview = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .args([
            "environment:repair-preview",
            "--spec",
            r#"{"timezone":"Etc/UTC","automatic_time":true}"#,
            "--json",
        ])
        .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
        .env("DBUS_SESSION_BUS_ADDRESS", ready.trim())
        .env("HOME", &home)
        .env("AGENT_MANAGER_DATA_DIR", &data)
        .env("XDG_CURRENT_DESKTOP", "")
        .output()
        .unwrap();
    let result: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert!(preview.status.success(), "{result}");
    assert_eq!(result["ok"], true, "{result}");
    let timezone = result["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "time.timezone")
        .unwrap();
    assert_eq!(timezone["status"], "unavailable", "{result}");
    let automatic_time = result["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "ntp.automatic_time")
        .unwrap();
    assert_eq!(automatic_time["before"], serde_json::Value::Null);
    assert_eq!(automatic_time["status"], "unavailable");
    assert!(
        !timezone_marker.exists(),
        "Repair preview activated the timezone service"
    );
    assert!(!home.exists());
    assert!(!data.exists());

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
    // The former inspection used this same activation-capable call. Prove the
    // locale marker is viable rather than accepting an absent service file.
    let uid = unsafe { libc::geteuid() }.to_string();
    let control = Command::new("/usr/bin/busctl")
        .args([
            "--system",
            "--auto-start=yes",
            "--timeout=1",
            "call",
            "org.freedesktop.Accounts",
            "/org/freedesktop/Accounts",
            "org.freedesktop.Accounts",
            "FindUserById",
            "x",
            &uid,
        ])
        .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
        .output()
        .unwrap();
    assert!(!control.status.success());
    assert!(
        locale_marker.exists(),
        "The locale activation control did not run"
    );
    let control = Command::new("/usr/bin/busctl")
        .args([
            "--system",
            "--auto-start=yes",
            "--timeout=1",
            "call",
            "org.freedesktop.timedate1",
            "/org/freedesktop/timedate1",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            "s",
            "org.freedesktop.timedate1",
        ])
        .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
        .output()
        .unwrap();
    assert!(!control.status.success());
    assert!(
        timezone_marker.exists(),
        "The timezone activation control did not run"
    );
    let control = Command::new("/usr/bin/busctl")
        .args([
            "--system",
            "--auto-start=yes",
            "--timeout=1",
            "call",
            "org.freedesktop.locale1",
            "/org/freedesktop/locale1",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            "s",
            "org.freedesktop.locale1",
        ])
        .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
        .output()
        .unwrap();
    assert!(!control.status.success());
    assert!(
        system_locale_marker.exists(),
        "The system-locale activation control did not run"
    );

    let automatic = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let daemon = zbus::blocking::connection::Builder::address(ready.trim())
        .unwrap()
        .name("org.freedesktop.timedate1")
        .unwrap()
        .serve_at(
            "/org/freedesktop/timedate1",
            Timedated {
                automatic: automatic.clone(),
                writes: writes.clone(),
            },
        )
        .unwrap()
        .build()
        .unwrap();
    let cli = |verb: &str, requested: bool, flags: &[&str]| {
        let spec = format!("{{\"automatic_time\":{requested}}}");
        let output = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
            .args([verb, "--spec", &spec, "--json"])
            .args(flags)
            .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
            .env("DBUS_SESSION_BUS_ADDRESS", ready.trim())
            .env("HOME", &home)
            .env("AGENT_MANAGER_DATA_DIR", &data)
            .env("XDG_CURRENT_DESKTOP", "")
            .output()
            .unwrap();
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(!home.exists());
        assert!(!data.exists());
        assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 0);
        result
    };
    for actual in [false, true] {
        automatic.store(actual, std::sync::atomic::Ordering::SeqCst);
        let preview = cli("environment:repair-preview", actual, &[]);
        assert_eq!(preview["ok"], true, "{preview}");
        assert_eq!(preview["data"]["items"][0]["before"], actual);
        assert_eq!(preview["data"]["items"][0]["status"], "unchanged");
        let base = preview["data"]["base"].as_str().unwrap();
        let result = cli("environment:repair", actual, &["--base", base, "--confirm"]);
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["data"]["status"], "not_needed");
        assert!(!result["data"]["verification_limits"]
            .as_array()
            .unwrap()
            .is_empty());

        let different = cli("environment:repair-preview", !actual, &[]);
        assert_eq!(different["data"]["items"][0]["before"], actual);
        assert_eq!(different["data"]["items"][0]["status"], "unavailable");
        let base = different["data"]["base"].as_str().unwrap();
        let result = cli(
            "environment:repair",
            !actual,
            &["--base", base, "--confirm"],
        );
        assert_eq!(result["data"]["status"], "partial");
    }
    let preview = cli("environment:repair-preview", true, &[]);
    let base = preview["data"]["base"].as_str().unwrap();
    automatic.store(false, std::sync::atomic::Ordering::SeqCst);
    let stale = cli("environment:repair", true, &["--base", base, "--confirm"]);
    assert_eq!(stale["ok"], false, "{stale}");
    assert!(stale["errors"].as_array().unwrap().iter().any(|error| {
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("changed after preview"))
    }));
    assert!(daemon.release_name("org.freedesktop.timedate1").unwrap());
    drop(daemon);
    let missing = cli("environment:repair-preview", true, &[]);
    assert_eq!(
        missing["data"]["items"][0]["before"],
        serde_json::Value::Null
    );
    assert_eq!(missing["data"]["items"][0]["status"], "unavailable");
}

struct Accounts {
    uid: u32,
    path: zbus::zvariant::OwnedObjectPath,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

fn gnome_formats_repair_preserves_other_preferences_and_requires_fresh_confirmation() {
    let root = Fixture(std::env::temp_dir().join(format!(
        "am-gnome-formats-{}",
        agent_manager_engine::store::uuid_v4()
    )));
    std::fs::create_dir(&root.0).unwrap();
    let home = root.path().join("missing-home");
    let data = root.path().join("data");
    let settings = root.path().join("settings");
    let command = |program: &str, args: &[&str], desktop: &str| {
        let mut command = Command::new(program);
        command
            .args(args)
            .env("HOME", &home)
            .env("AGENT_MANAGER_DATA_DIR", &data)
            .env("XDG_CONFIG_HOME", &settings)
            .env("GSETTINGS_BACKEND", "keyfile")
            .env("XDG_CURRENT_DESKTOP", desktop)
            .env_remove("GSETTINGS_SCHEMA_DIR")
            .env(
                "DBUS_SYSTEM_BUS_ADDRESS",
                format!("unix:path={}/absent-system-bus", root.path().display()),
            )
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}/absent-user-bus", root.path().display()),
            );
        command.output().unwrap()
    };
    let settings_call = |args: &[&str]| {
        let output = command("/usr/bin/gsettings", args, "GNOME");
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    let locales = Command::new("/usr/bin/locale").arg("-a").output().unwrap();
    assert!(
        String::from_utf8(locales.stdout)
            .unwrap()
            .lines()
            .any(|locale| {
                matches!(
                    locale
                        .to_ascii_lowercase()
                        .replace("utf-8", "utf8")
                        .as_str(),
                    "en_us.utf8"
                )
            }),
        "Native CI fixture requires its installed en_US UTF-8 locale"
    );
    settings_call(&["set", "org.gnome.system.locale", "region", "'en_GB.UTF-8'"]);
    settings_call(&[
        "set",
        "org.gnome.desktop.interface",
        "clock-show-seconds",
        "true",
    ]);
    let keyfile = settings.join("glib-2.0/settings/keyfile");
    let initial = std::fs::read(&keyfile).unwrap();
    let spec = r#"{"region":"US"}"#;
    let cli = |verb: &str, flags: &[&str], spec: &str, desktop: &str| {
        let mut args = vec![verb, "--spec", spec, "--json"];
        args.extend_from_slice(flags);
        let output = command(env!("CARGO_BIN_EXE_agent-manager-engine"), &args, desktop);
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let preview = cli("environment:repair-preview", &[], spec, "GNOME");
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["data"]["items"][0]["status"], "change");
    assert_eq!(preview["data"]["items"][0]["before"], "en_GB.UTF-8");
    assert_eq!(preview["data"]["items"][0]["expected"], "en_US.UTF-8");
    assert_eq!(std::fs::read(&keyfile).unwrap(), initial);
    assert!(!data.exists());
    assert!(!home.exists());
    let base = preview["data"]["base"].as_str().unwrap();
    assert_eq!(
        cli("environment:repair", &["--base", base], spec, "GNOME")["ok"],
        false
    );
    assert_eq!(std::fs::read(&keyfile).unwrap(), initial);
    assert!(!data.exists());
    settings_call(&["set", "org.gnome.system.locale", "region", "'en_CA.UTF-8'"]);
    assert_eq!(
        cli(
            "environment:repair",
            &["--base", base, "--confirm"],
            spec,
            "GNOME"
        )["ok"],
        false
    );
    assert_eq!(
        settings_call(&["get", "org.gnome.system.locale", "region"]),
        "'en_CA.UTF-8'"
    );
    assert!(!data.exists());
    let preview = cli("environment:repair-preview", &[], spec, "GNOME");
    let base = preview["data"]["base"].as_str().unwrap();
    let repaired = cli(
        "environment:repair",
        &["--base", base, "--confirm"],
        spec,
        "GNOME",
    );
    assert_eq!(repaired["ok"], true, "{repaired}");
    assert_eq!(repaired["data"]["status"], "applied", "{repaired}");
    assert_eq!(repaired["data"]["items"][0]["status"], "verified");
    assert_eq!(repaired["data"]["relogin_required"], true);
    assert!(!repaired["data"]["verification_limits"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        settings_call(&["get", "org.gnome.system.locale", "region"]),
        "'en_US.UTF-8'"
    );
    assert_eq!(
        settings_call(&["get", "org.gnome.desktop.interface", "clock-show-seconds"]),
        "true"
    );
    let database = data.join("native-manager.db");
    let before_noop = std::fs::read(&database).unwrap();
    let preview = cli("environment:repair-preview", &[], spec, "GNOME");
    let base = preview["data"]["base"].as_str().unwrap();
    let noop = cli(
        "environment:repair",
        &["--base", base, "--confirm"],
        spec,
        "GNOME",
    );
    assert_eq!(noop["data"]["status"], "not_needed", "{noop}");
    assert_eq!(std::fs::read(&database).unwrap(), before_noop);
    for (spec, desktop) in [(r#"{"region":"ZZ"}"#, "GNOME"), (spec, "KDE")] {
        let preview = cli("environment:repair-preview", &[], spec, desktop);
        assert_eq!(
            preview["data"]["items"][0]["status"], "unavailable",
            "{preview}"
        );
    }
    assert!(!home.exists());
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
    change_uid_on_language: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
#[zbus::interface(name = "org.freedesktop.Accounts.User")]
impl User {
    #[zbus(property)]
    fn uid(&self) -> u64 {
        self.state.lock().unwrap().0
    }
    #[zbus(property)]
    fn language(&self) -> String {
        let mut state = self.state.lock().unwrap();
        if self
            .change_uid_on_language
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            state.0 += 1;
        }
        state.1.clone()
    }
    #[zbus(property)]
    fn languages(&self) -> Vec<String> {
        self.extra_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        vec![self.language(), "de_DE.UTF-8".into()]
    }
}

#[test]
fn gnome_inspection_and_formats_repair_preserve_user_evidence_and_external_changes() {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    let change_uid_on_language = Arc::new(AtomicBool::new(false));
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
                change_uid_on_language: change_uid_on_language.clone(),
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
    change_uid_on_language.store(true, Ordering::Relaxed);
    let raced = inspect("GNOME");
    assert!(raced["language.preferred"].is_null(), "{raced:?}");
    assert!(raced["language.region"].is_null(), "{raced:?}");
    state.lock().unwrap().0 = uid as u64;
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
    *state.lock().unwrap() = (uid as u64, "en".into());
    let countryless = inspect("GNOME");
    assert_eq!(countryless["language.preferred"], "en");
    assert!(countryless["language.region"].is_null());
    *state.lock().unwrap() = (uid as u64, "en_US.UTF-8".into());
    state.lock().unwrap().0 = uid as u64 + 1;
    let rows = inspect("GNOME");
    assert!(rows["language.preferred"].is_null());
    assert!(rows["language.region"].is_null());
    let unknown = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .args([
            "environment:repair-preview",
            "--spec",
            r#"{"region":"US"}"#,
            "--json",
        ])
        .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
        .env("HOME", &home)
        .env("AGENT_MANAGER_DATA_DIR", &data)
        .env("XDG_CURRENT_DESKTOP", "GNOME")
        .env("GSETTINGS_BACKEND", "keyfile")
        .env("XDG_CONFIG_HOME", &settings)
        .output()
        .unwrap();
    let unknown: serde_json::Value = serde_json::from_slice(&unknown.stdout).unwrap();
    assert_eq!(
        unknown["data"]["items"][0]["before"],
        serde_json::Value::Null
    );
    assert_eq!(unknown["data"]["items"][0]["status"], "unavailable");
    assert!(!data.exists());
    assert!(!settings.exists());
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

    // Exercise inheritance through the native UID/owner-bound reader. A private
    // database lock holds the confirmed CLI after initial base validation, so an
    // external same-value change cannot be absorbed as a new execution baseline.
    *state.lock().unwrap() = (uid as u64, "en_GB.UTF-8".into());
    let configured = |program: &str| {
        let mut command = Command::new(program);
        command
            .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
            .env("HOME", &home)
            .env("AGENT_MANAGER_DATA_DIR", &data)
            .env("XDG_CURRENT_DESKTOP", "GNOME")
            .env("GSETTINGS_BACKEND", "keyfile")
            .env("XDG_CONFIG_HOME", &settings)
            .env("LANG", "de_DE.UTF-8");
        command
    };
    let set = |value: &str| {
        assert!(configured("/usr/bin/gsettings")
            .args(["set", "org.gnome.system.locale", "region", value])
            .status()
            .unwrap()
            .success());
    };
    set("''");
    let preview = configured(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .args([
            "environment:repair-preview",
            "--spec",
            r#"{"region":"US"}"#,
            "--json",
        ])
        .output()
        .unwrap();
    let preview: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["data"]["items"][0]["before"], "en_GB.UTF-8");
    assert_eq!(preview["data"]["items"][0]["status"], "change", "{preview}");
    assert!(!data.exists());
    let context = agent_manager_engine::Context {
        home_dir: home.clone(),
        data_dir: data.clone(),
    };
    let conn = agent_manager_engine::store::open_write(&context).unwrap();
    conn.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let mut child = PendingRepair(Some(
        configured(env!("CARGO_BIN_EXE_agent-manager-engine"))
            .args([
                "environment:repair",
                "--spec",
                r#"{"region":"US"}"#,
                "--base",
                preview["data"]["base"].as_str().unwrap(),
                "--confirm",
                "--json",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let database = data.join("native-manager.db");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let opened = std::fs::read_dir(format!("/proc/{}/fd", child.0.as_ref().unwrap().id()))
            .unwrap()
            .flatten()
            .any(|entry| std::fs::read_link(entry.path()).is_ok_and(|path| path == database));
        if opened {
            break;
        }
        if std::time::Instant::now() >= deadline
            || child.0.as_mut().unwrap().try_wait().unwrap().is_some()
        {
            let _ = child.0.as_mut().unwrap().kill();
            let output = child.0.take().unwrap().wait_with_output().unwrap();
            panic!("Confirmed CLI did not reach its audit database: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // HOME/data isolation does not change the intentional per-UID mutation
    // lock. A concurrent request must refuse, including a different app data dir.
    let concurrent_data = root.path().join("concurrent-data");
    let concurrent = configured(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .env("AGENT_MANAGER_DATA_DIR", &concurrent_data)
        .args([
            "environment:repair",
            "--spec",
            r#"{"region":"US"}"#,
            "--base",
            preview["data"]["base"].as_str().unwrap(),
            "--confirm",
            "--json",
        ])
        .output()
        .unwrap();
    let concurrent: serde_json::Value = serde_json::from_slice(&concurrent.stdout).unwrap();
    assert_eq!(concurrent["ok"], false, "{concurrent}");
    assert!(
        concurrent["errors"].to_string().contains("already running"),
        "{concurrent}"
    );
    assert!(!concurrent_data.exists());
    set("'en_GB.UTF-8'"); // Same effective locale, different raw inheritance.
    conn.execute_batch("ROLLBACK").unwrap();
    let output = child.0.take().unwrap().wait_with_output().unwrap();
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["data"]["status"], "partial", "{result}");
    assert_eq!(result["data"]["items"][0]["status"], "stale", "{result}");
    let actual = configured("/usr/bin/gsettings")
        .args(["get", "org.gnome.system.locale", "region"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(actual.stdout).unwrap().trim(),
        "'en_GB.UTF-8'"
    );
    assert!(!home.exists());
    // One end-to-end writer fixture owns this OS user's regional mutation scope;
    // the following independent backend scenario starts after the first exits.
    gnome_formats_repair_preserves_other_preferences_and_requires_fresh_confirmation();
}
