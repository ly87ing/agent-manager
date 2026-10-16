//! GNOME preferences belong to the original user's existing session.
use serde::Serialize;

pub(super) fn gnome(desktop: &str) -> bool {
    desktop
        .split(':')
        .any(|part| part.eq_ignore_ascii_case("gnome"))
}

fn boolean(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub(super) struct State {
    bus: String,
    pub automatic: bool,
    writable: bool,
    location: Option<bool>,
    service: Option<Service>,
    allowed: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
struct Service {
    bus: String,
    owner: String,
    pid: u32,
    start: u64,
    uid: u32,
}

impl State {
    pub(super) fn available(&self, target: bool) -> bool {
        self.writable
            && (!target || self.location == Some(true) && self.service.is_some() && self.allowed)
    }
}

#[cfg(target_os = "linux")]
fn service() -> Option<(String, Option<Service>)> {
    use std::time::Duration;
    zbus::block_on(futures_lite::future::race(
        async {
            let connection = zbus::Connection::session().await.ok()?;
            let call = async |method: &str, body: &str| {
                connection
                    .call_method(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        Some("org.freedesktop.DBus"),
                        method,
                        &(body,),
                    )
                    .await
                    .ok()
            };
            let id = async || {
                connection
                    .call_method(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        Some("org.freedesktop.DBus"),
                        "GetId",
                        &(),
                    )
                    .await
                    .ok()?
                    .body()
                    .deserialize::<String>()
                    .ok()
            };
            let bus = id().await?;
            let owner_reply = connection
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "GetNameOwner",
                    &("org.gnome.SettingsDaemon.Datetime",),
                )
                .await;
            let owner_reply = match owner_reply {
                Ok(reply) => reply,
                Err(zbus::Error::MethodError(name, ..))
                    if name.as_str() == "org.freedesktop.DBus.Error.NameHasNoOwner" =>
                {
                    return (id().await? == bus).then_some((bus, None));
                }
                Err(_) => return None,
            };
            let owner: String = owner_reply.body().deserialize().ok()?;
            let values: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
                call("GetConnectionCredentials", &owner)
                    .await?
                    .body()
                    .deserialize()
                    .ok()?;
            let pid = u32::try_from(values.get("ProcessID")?).ok()?;
            let uid = u32::try_from(values.get("UnixUserID")?).ok()?;
            if uid != unsafe { libc::geteuid() } || pid == 0 {
                return None;
            }
            let start = super::repair_linux_time::start_time(
                &std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?,
            )?;
            let after: String = call("GetNameOwner", "org.gnome.SettingsDaemon.Datetime")
                .await?
                .body()
                .deserialize()
                .ok()?;
            (owner == after && id().await? == bus).then_some((
                bus.clone(),
                Some(Service {
                    bus,
                    owner,
                    pid,
                    start,
                    uid,
                }),
            ))
        },
        async {
            async_io::Timer::after(Duration::from_secs(8)).await;
            None
        },
    ))
}

#[cfg(target_os = "linux")]
pub(super) fn read() -> Option<State> {
    if !gnome(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()) {
        return None;
    }
    let automatic = boolean(&super::linux::gsetting(
        "org.gnome.desktop.datetime",
        "automatic-timezone",
    )?)?;
    let writable = boolean(&super::command::read(
        "/usr/bin/gsettings",
        &[
            "writable",
            "org.gnome.desktop.datetime",
            "automatic-timezone",
        ],
    )?)?;
    let location = super::linux::gsetting("org.gnome.system.location", "enabled")
        .as_deref()
        .and_then(boolean);
    let (bus, service) = service()?;
    let allowed = service.as_ref().is_some_and(|service| {
        let subject = format!("{},{},{}", service.pid, service.start, service.uid);
        super::command::read(
            "/usr/bin/pkcheck",
            &[
                "--action-id",
                "org.freedesktop.timedate1.set-timezone",
                "--process",
                &subject,
            ],
        )
        .is_some()
    });
    Some(State {
        bus,
        automatic,
        writable,
        location,
        service,
        allowed,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn write(before: &State, target: bool, authorized: impl FnOnce() -> bool) -> bool {
    before.available(target)
        && guarded_write(before, read().as_ref(), authorized, || {
            super::command::read(
                "/usr/bin/gsettings",
                &[
                    "set",
                    "org.gnome.desktop.datetime",
                    "automatic-timezone",
                    if target { "true" } else { "false" },
                ],
            )
            .is_some()
        })
}

fn guarded_write(
    before: &State,
    fresh: Option<&State>,
    authorized: impl FnOnce() -> bool,
    setter: impl FnOnce() -> bool,
) -> bool {
    fresh == Some(before) && authorized() && setter()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabling_automatic_timezone_preserves_privacy_and_requires_daemon_permission() {
        let mut state = State {
            bus: "test-bus".into(),
            automatic: false,
            writable: true,
            location: None,
            service: None,
            allowed: false,
        };
        assert!(state.available(false));
        assert!(!state.available(true));
        state.location = Some(false);
        assert!(!state.available(true));
        state.location = Some(true);
        assert!(!state.available(true));
        state.allowed = true;
        assert!(!state.available(true));
        state.service = Some(Service {
            bus: "test-bus".into(),
            owner: ":1.2".into(),
            pid: 123,
            start: 456,
            uid: 1000,
        });
        assert!(state.available(true));
        state.writable = false;
        assert!(!state.available(false));
        assert!(!state.available(true));
        state.writable = true;
        for fresh in [None, Some(&state)] {
            assert!(!guarded_write(
                &state,
                fresh,
                || false,
                || panic!("Expired authorization wrote a preference")
            ));
        }
        let mut changed = state.clone();
        changed.automatic = true;
        assert!(!guarded_write(
            &state,
            Some(&changed),
            || true,
            || panic!("Changed preference was overwritten")
        ));
        assert!(guarded_write(&state, Some(&state), || true, || true));
    }

    #[test]
    fn desktop_and_boolean_evidence_are_explicit() {
        assert!(gnome("ubuntu:GNOME"));
        assert!(!gnome("not-gnome:KDE"));
        assert!(!gnome(""));
        assert_eq!(boolean("false"), Some(false));
        for unknown in ["", "yes", "0", "'false'"] {
            assert_eq!(boolean(unknown), None);
        }
    }
}
