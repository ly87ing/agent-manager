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
