//! Actual CLI/typed D-Bus readback on a private bus, never the host time service.
use super::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Clone, PartialEq, Debug)]
struct Lists {
    runtime: Vec<String>,
    system: Vec<String>,
    link: Vec<String>,
}

struct Timesync(Arc<Mutex<Lists>>);
#[zbus::interface(name = "org.freedesktop.timesync1.Manager")]
impl Timesync {
    #[zbus(property, name = "RuntimeNTPServers")]
    fn runtime_ntp_servers(&self) -> Vec<String> {
        self.0.lock().unwrap().runtime.clone()
    }
    #[zbus(property, name = "SystemNTPServers")]
    fn system_ntp_servers(&self) -> Vec<String> {
        self.0.lock().unwrap().system.clone()
    }
    #[zbus(property, name = "LinkNTPServers")]
    fn link_ntp_servers(&self) -> Vec<String> {
        self.0.lock().unwrap().link.clone()
    }
}

#[test]
fn inspection_respects_runtime_server_precedence_and_keeps_peer_unknown() {
    let root = Fixture(std::env::temp_dir().join(format!(
        "am-timesync-precedence-{}",
        agent_manager_engine::store::uuid_v4()
    )));
    std::fs::create_dir(root.path()).unwrap();
    let address = format!(
        "unix:abstract=am-timesync-precedence-{}",
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
    let state = Arc::new(Mutex::new(Lists {
        runtime: vec!["Current.Example.".into()],
        system: vec!["old.example".into()],
        link: vec![],
    }));
    let _service = zbus::blocking::connection::Builder::address(ready.trim())
        .unwrap()
        .name("org.freedesktop.timesync1")
        .unwrap()
        .serve_at("/org/freedesktop/timesync1", Timesync(state.clone()))
        .unwrap()
        .build()
        .unwrap();
    let home = root.path().join("missing-home");
    let data = root.path().join("missing-data");
    let settings = root.path().join("missing-settings");
    let inspect = || {
        let before = state.lock().unwrap().clone();
        let output = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
            .args(["environment:check", "--json"])
            .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
            .env("DBUS_SESSION_BUS_ADDRESS", ready.trim())
            .env("HOME", &home)
            .env("AGENT_MANAGER_DATA_DIR", &data)
            .env("XDG_CONFIG_HOME", &settings)
            .env("GSETTINGS_BACKEND", "keyfile")
            .env("XDG_CURRENT_DESKTOP", "")
            .output()
            .unwrap();
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(output.status.success(), "{result}");
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(*state.lock().unwrap(), before);
        assert!(!home.exists());
        assert!(!data.exists());
        assert!(!settings.exists());
        let checks = result["data"]["checks"].as_array().unwrap();
        let row = |id: &str| checks.iter().find(|row| row["id"] == id).unwrap();
        assert!(row("ntp.actual_source")["observed"].is_null());
        row("ntp.server")["observed"].clone()
    };
    assert_eq!(inspect(), json!("current.example"));
    state.lock().unwrap().link = vec!["link.example".into()];
    assert!(inspect().is_null());
    state.lock().unwrap().runtime.clear();
    assert!(inspect().is_null());
    state.lock().unwrap().link.clear();
    assert_eq!(inspect(), json!("old.example"));
    state.lock().unwrap().system.clear();
    state.lock().unwrap().link = vec!["link.example".into()];
    assert_eq!(inspect(), json!("link.example"));
    state.lock().unwrap().link.clear();
    assert!(inspect().is_null());
}
