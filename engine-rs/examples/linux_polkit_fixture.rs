//! Disposable CI authentication agent; never bundled or used by the product.
#[cfg(target_os = "linux")]
mod fixture {
    use std::{
        collections::HashMap,
        io::Write,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };
    use zbus::zvariant::OwnedValue;
    const AUTHORITY: &str = "org.freedesktop.PolicyKit1.Authority";
    const PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
    const AGENT: &str = "/io/github/agentmanager/CIFixture";
    const ACTION: &str = "org.freedesktop.timedate1.set-timezone";

    #[derive(Debug, zbus::DBusError)]
    #[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
    enum AuthenticationError {
        Cancelled(String),
        Failed(String),
    }
    impl From<zbus::fdo::Error> for AuthenticationError {
        fn from(error: zbus::fdo::Error) -> Self {
            Self::Failed(error.to_string())
        }
    }

    struct Agent {
        uid: u32,
        pid: u32,
        authority: String,
        external_zone: Option<String>,
        ntp_mode: Option<String>,
        done: Arc<AtomicBool>,
    }
    #[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
    impl Agent {
        #[allow(clippy::too_many_arguments)]
        async fn begin_authentication(
            &self,
            action: &str,
            _message: &str,
            _icon: &str,
            details: HashMap<String, String>,
            cookie: &str,
            identities: Vec<(String, HashMap<String, OwnedValue>)>,
            #[zbus(connection)] connection: &zbus::Connection,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) -> Result<(), AuthenticationError> {
            if header.sender().map(|name| name.as_str()) != Some(self.authority.as_str())
                || details.get("polkit.subject-pid") != Some(&self.pid.to_string())
                || self.done.load(Ordering::Acquire)
                || action
                    != if self.ntp_mode.is_some() {
                        "org.freedesktop.policykit.exec"
                    } else {
                        ACTION
                    }
                || !identities.iter().any(|(kind, fields)| {
                    kind == "unix-user"
                        && fields.get("uid").and_then(|uid| u32::try_from(uid).ok())
                            == Some(self.uid)
                })
            {
                eprintln!("CI authentication rejected: authority={}, subject={}, action={}, self_identity={}",
                    header.sender().map(|name| name.as_str()) == Some(self.authority.as_str()),
                    details.get("polkit.subject-pid") == Some(&self.pid.to_string()),
                    action == if self.ntp_mode.is_some() { "org.freedesktop.policykit.exec" } else { ACTION },
                    identities.iter().any(|(kind, fields)| kind == "unix-user" && fields.get("uid").and_then(|uid| u32::try_from(uid).ok()) == Some(self.uid)));
                return Err(
                    zbus::fdo::Error::AccessDenied("Unexpected CI identity/action".into()).into(),
                );
            }
            if let Some(mode) = &self.ntp_mode {
                if mode == "ntp-cancel" {
                    self.done.store(true, Ordering::Release);
                    return Err(AuthenticationError::Cancelled(
                        "CI authentication dismissed".into(),
                    ));
                }
                if mode == "ntp-change" {
                    let path = "/etc/systemd/timesyncd.conf.d/80-agent-manager-ci.conf";
                    let text = "# Agent Manager isolated NTP fixture v1\n[Time]\nFallbackNTP=198.51.100.2\nPollIntervalMinSec=32\nPollIntervalMaxSec=2048\n";
                    if std::fs::read_to_string(path).ok().as_deref() != Some(text) {
                        return Err(
                            zbus::fdo::Error::Failed("Changed CI NTP baseline".into()).into()
                        );
                    }
                    std::fs::write(path, format!("{text}# CI authorization drift\n"))
                        .map_err(|_| zbus::fdo::Error::Failed("CI NTP drift failed".into()))?;
                }
            }
            if let Some(zone) = &self.external_zone {
                connection
                    .call_method(
                        Some("org.freedesktop.timedate1"),
                        "/org/freedesktop/timedate1",
                        Some("org.freedesktop.timedate1"),
                        "SetTimezone",
                        &(zone, false),
                    )
                    .await
                    .map_err(|_| {
                        zbus::fdo::Error::Failed("CI external timezone change failed".into())
                    })?;
            }
            let identity = (
                "unix-user",
                HashMap::from([("uid", OwnedValue::from(self.uid))]),
            );
            connection
                .call_method(
                    Some("org.freedesktop.PolicyKit1"),
                    PATH,
                    Some(AUTHORITY),
                    "AuthenticationAgentResponse2",
                    &(0u32, cookie, identity),
                )
                .await
                .map_err(|_| {
                    zbus::fdo::Error::Failed("CI authentication response failed".into())
                })?;
            self.done.store(true, Ordering::Release);
            Ok(())
        }
        fn cancel_authentication(
            &self,
            _cookie: &str,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<()> {
            if header.sender().map(|name| name.as_str()) != Some(self.authority.as_str()) {
                return Err(zbus::fdo::Error::AccessDenied(
                    "Unexpected CI authority".into(),
                ));
            }
            self.done.store(true, Ordering::Release);
            Ok(())
        }
    }

    pub fn run() {
        assert_eq!(std::env::var("CI").as_deref(), Ok("true"));
        assert_eq!(unsafe { libc::geteuid() }, 0);
        let args: Vec<_> = std::env::args().skip(1).collect();
        assert_eq!(args.len(), 4);
        let uid: u32 = args[0].parse().expect("Invalid CI UID");
        let pid: u32 = args[1].parse().expect("Invalid CI PID");
        let start: u64 = args[2].parse().expect("Invalid CI process start time");
        assert_ne!(uid, 0);
        let external_zone = match args[3].as_str() {
            "allow" => None,
            "change" => Some("Pacific/Honolulu".into()),
            "ntp-allow" | "ntp-cancel" | "ntp-change" => None,
            _ => panic!("Invalid CI authentication mode"),
        };
        let ntp_mode = args[3].starts_with("ntp-").then(|| args[3].clone());
        assert_eq!(
            std::env::var(if ntp_mode.is_some() {
                "AM_ISOLATED_LINUX_NTP"
            } else {
                "AM_ISOLATED_LINUX_TIMEZONE"
            })
            .as_deref(),
            Ok("1")
        );
        let done = Arc::new(AtomicBool::new(false));
        zbus::block_on(async {
            let connection = zbus::connection::Builder::system()
                .expect("CI system bus unavailable")
                .method_timeout(Duration::from_secs(8))
                .build()
                .await
                .expect("CI agent connection failed");
            let authority: String = connection
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "GetNameOwner",
                    &("org.freedesktop.PolicyKit1",),
                )
                .await
                .expect("CI authority owner unavailable")
                .body()
                .deserialize()
                .expect("CI authority identity invalid");
            connection
                .object_server()
                .at(
                    AGENT,
                    Agent {
                        uid,
                        pid,
                        authority,
                        external_zone,
                        ntp_mode,
                        done: done.clone(),
                    },
                )
                .await
                .expect("CI agent export failed");
            let subject = (
                "unix-process",
                HashMap::from([
                    ("pid", OwnedValue::from(pid)),
                    ("start-time", OwnedValue::from(start)),
                    ("uid", OwnedValue::from(uid as i32)),
                ]),
            );
            connection
                .call_method(
                    Some("org.freedesktop.PolicyKit1"),
                    PATH,
                    Some(AUTHORITY),
                    "RegisterAuthenticationAgent",
                    &(&subject, "C", AGENT),
                )
                .await
                .unwrap_or_else(|_| panic!("CI agent registration failed"));
            let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
            let own_start = stat
                .rsplit_once(')')
                .unwrap()
                .1
                .split_whitespace()
                .nth(19)
                .unwrap();
            println!(
                "{{\"ready\":true,\"pid\":{},\"start\":\"{}\"}}",
                std::process::id(),
                own_start
            );
            std::io::stdout().flush().unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while !done.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                async_io::Timer::after(Duration::from_millis(20)).await;
            }
            assert!(
                done.load(Ordering::Acquire),
                "CI agent never received an authentication request"
            );
            async_io::Timer::after(Duration::from_millis(300)).await;
            let _ = connection
                .call_method(
                    Some("org.freedesktop.PolicyKit1"),
                    PATH,
                    Some(AUTHORITY),
                    "UnregisterAuthenticationAgent",
                    &(&subject, AGENT),
                )
                .await;
        });
    }
}

#[cfg(target_os = "linux")]
fn main() {
    fixture::run();
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This disposable CI fixture requires Linux.");
    std::process::exit(2);
}
