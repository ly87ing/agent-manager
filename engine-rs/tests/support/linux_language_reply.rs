use super::{Fixture, PrivateBus};
use std::collections::HashMap;
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

struct Authority;
#[zbus::interface(name = "org.freedesktop.PolicyKit1.Authority")]
impl Authority {
    fn check_authorization(
        &self,
        _subject: (String, HashMap<String, OwnedValue>),
        action: &str,
        _details: HashMap<String, String>,
        _flags: u32,
        _cancellation: &str,
    ) -> ((bool, bool, HashMap<String, String>),) {
        assert_eq!(action, "org.freedesktop.accounts.change-own-user-data");
        ((true, false, HashMap::new()),)
    }
}
struct Accounts {
    uid: u32,
    path: OwnedObjectPath,
}
#[zbus::interface(name = "org.freedesktop.Accounts")]
impl Accounts {
    fn find_user_by_id(&self, uid: i64) -> OwnedObjectPath {
        assert_eq!(uid, self.uid as i64);
        self.path.clone()
    }
}
struct State {
    languages: Vec<String>,
    lose_reply: bool,
    writes: usize,
    reset_formats: Option<std::path::PathBuf>,
}
struct User {
    uid: u32,
    state: Arc<Mutex<State>>,
}
#[zbus::interface(name = "org.freedesktop.Accounts.User")]
impl User {
    #[zbus(property)]
    fn uid(&self) -> u64 {
        self.uid as u64
    }
    #[zbus(property)]
    fn language(&self) -> String {
        self.state.lock().unwrap().languages[0].clone()
    }
    #[zbus(property)]
    fn languages(&self) -> Vec<String> {
        self.state.lock().unwrap().languages.clone()
    }
    fn set_languages(
        &self,
        languages: Vec<String>,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let flags = header.primary().flags();
        assert!(flags.contains(zbus::message::Flags::NoAutoStart));
        assert!(!flags.contains(zbus::message::Flags::AllowInteractiveAuth));
        let mut state = self.state.lock().unwrap();
        state.languages = languages;
        state.writes += 1;
        if let Some(settings) = &state.reset_formats {
            assert!(Command::new("/usr/bin/gsettings")
                .env("GSETTINGS_BACKEND", "keyfile")
                .env("XDG_CONFIG_HOME", settings)
                .args(["reset", "org.gnome.system.locale", "region"])
                .status()
                .unwrap()
                .success());
        }
        if state.lose_reply {
            Err(zbus::fdo::Error::NoReply(
                "Private lost-reply control".into(),
            ))
        } else {
            Ok(())
        }
    }
}

#[test]
#[ignore = "Requires installed GNOME schemas, Polkit tools and en_US/en_GB/fr_FR UTF-8 locales; GUI CI invokes this explicitly."]
fn language_reply_loss_stops_formats_even_when_primary_readback_matches() {
    assert!(std::path::Path::new("/usr/bin/gsettings").exists());
    let installed = Command::new("/usr/bin/locale").arg("-a").output().unwrap();
    assert!(
        String::from_utf8_lossy(&installed.stdout)
            .lines()
            .any(|value| {
                value.eq_ignore_ascii_case("fr_FR.utf8")
                    || value.eq_ignore_ascii_case("fr_FR.UTF-8")
            }),
        "The native language fixture requires an installed fr_FR UTF-8 locale"
    );
    let root = Fixture(std::env::temp_dir().join(format!(
        "am-language-reply-{}",
        agent_manager_engine::store::uuid_v4()
    )));
    std::fs::create_dir(&root.0).unwrap();
    let address = format!(
        "unix:abstract=am-language-reply-{}",
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
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = tx.send(BufReader::new(stdout).read_line(&mut line).map(|_| line));
    });
    let ready = rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    let uid = unsafe { libc::geteuid() };
    let path = OwnedObjectPath::try_from(format!("/org/freedesktop/Accounts/User{uid}")).unwrap();
    let state = Arc::new(Mutex::new(State {
        languages: vec!["en_US.UTF-8".into(), "de_DE.UTF-8".into()],
        lose_reply: true,
        writes: 0,
        reset_formats: None,
    }));
    let connection = zbus::blocking::connection::Builder::address(ready.trim())
        .unwrap()
        .name("org.freedesktop.Accounts")
        .unwrap()
        .serve_at(
            "/org/freedesktop/Accounts",
            Accounts {
                uid,
                path: path.clone(),
            },
        )
        .unwrap()
        .serve_at(
            path,
            User {
                uid,
                state: state.clone(),
            },
        )
        .unwrap()
        .serve_at("/org/freedesktop/PolicyKit1/Authority", Authority)
        .unwrap()
        .build()
        .unwrap();
    connection
        .request_name("org.freedesktop.PolicyKit1")
        .unwrap();
    let home = root.path().join("missing-home");
    let data = root.path().join("data");
    let settings = root.path().join("settings");
    let env = |command: &mut Command| {
        command
            .env("DBUS_SYSTEM_BUS_ADDRESS", ready.trim())
            .env("HOME", &home)
            .env("AGENT_MANAGER_DATA_DIR", &data)
            .env("XDG_CURRENT_DESKTOP", "GNOME")
            .env("GSETTINGS_BACKEND", "keyfile")
            .env("XDG_CONFIG_HOME", &settings);
    };
    let formats = |args: &[&str]| {
        let mut command = Command::new("/usr/bin/gsettings");
        env(&mut command);
        let output = command.args(args).output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    formats(&["set", "org.gnome.system.locale", "region", "'en_GB.UTF-8'"]);
    let cli = |action: &str, flags: &[&str], country: &str| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"));
        env(&mut command);
        let profile = serde_json::json!({"languages":["fr-FR"],"region":country}).to_string();
        let output = command
            .args([action, "--spec", &profile, "--json"])
            .args(flags)
            .output()
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(output.status.success(), "{value}");
        assert_eq!(value["ok"], true, "{value}");
        value["data"].clone()
    };
    for (inherited, lost, reset, country) in [
        (false, true, false, "US"),
        (false, false, false, "US"),
        (true, true, false, "GB"),
        (true, false, false, "GB"),
        (true, false, true, "GB"),
        (true, false, true, "US"),
    ] {
        {
            let mut state = state.lock().unwrap();
            state.languages[0] = "en_US.UTF-8".into();
            state.lose_reply = lost;
            state.reset_formats = reset.then(|| settings.clone());
        }
        if inherited {
            formats(&["reset", "org.gnome.system.locale", "region"]);
        } else {
            formats(&["set", "org.gnome.system.locale", "region", "'en_GB.UTF-8'"]);
        }
        let preview = cli("environment:repair-preview", &[], country);
        assert_eq!(preview["items"][0]["status"], "change", "{preview}");
        assert!(!home.exists());
        let applied = cli(
            "environment:repair",
            &["--base", preview["base"].as_str().unwrap(), "--confirm"],
            country,
        );
        assert_eq!(
            applied["status"],
            if lost || reset { "partial" } else { "applied" },
            "{applied}"
        );
        let items = applied["items"].as_array().unwrap();
        let status = |id: &str| {
            items.iter().find(|item| item["id"] == id).unwrap()["status"]
                .as_str()
                .unwrap()
        };
        assert_eq!(
            status("language.preferred"),
            if lost {
                "outcome_unknown"
            } else if reset {
                "unverified"
            } else {
                "verified"
            },
            "{applied}"
        );
        assert_eq!(
            status("language.region"),
            if reset {
                "unverified"
            } else if lost && !inherited {
                "not_applied"
            } else {
                "verified"
            },
            "{applied}"
        );
        if inherited {
            assert_eq!(
                status("language.independent_formats"),
                if reset { "unverified" } else { "verified" },
                "{applied}"
            );
            if !reset {
                assert_eq!(applied["relogin_required"], true);
            }
        }
        assert_eq!(
            formats(&["get", "org.gnome.system.locale", "region"]),
            if reset {
                "''"
            } else if inherited || lost {
                "'en_GB.UTF-8'"
            } else {
                "'en_US.UTF-8'"
            }
        );
        let state = state.lock().unwrap();
        assert!(state.languages[0].starts_with("fr_FR."));
        assert_eq!(state.languages[1], "de_DE.UTF-8");
    }
    assert_eq!(state.lock().unwrap().writes, 6);
}
