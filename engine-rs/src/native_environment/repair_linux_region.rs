//! GNOME formats remain an explicit preference of the original caller.
use serde::Serialize;

#[derive(Clone, PartialEq, Eq, Serialize)]
pub(super) struct State {
    pub locale: String,
    raw: String,
    writable: bool,
    uid: u32,
    source: std::collections::BTreeMap<String, Option<String>>,
    inherited: Option<Primary>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
struct Primary {
    locale: String,
    uid: u32,
    bus: String,
    owner: String,
    path: String,
}

#[cfg(target_os = "linux")]
fn primary() -> Option<Primary> {
    use std::time::Duration;
    zbus::block_on(futures_lite::future::race(
        async {
            let connection = zbus::Connection::system().await.ok()?;
            let bus = zbus::Proxy::new(
                &connection,
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
            )
            .await
            .ok()?;
            let id: String = bus.call("GetId", &()).await.ok()?;
            let owner: String = bus
                .call("GetNameOwner", &("org.freedesktop.Accounts",))
                .await
                .ok()?;
            let accounts = zbus::Proxy::new(
                &connection,
                owner.as_str(),
                "/org/freedesktop/Accounts",
                "org.freedesktop.Accounts",
            )
            .await
            .ok()?;
            let uid = unsafe { libc::geteuid() };
            let path: zbus::zvariant::OwnedObjectPath = accounts
                .call_with_flags(
                    "FindUserById",
                    zbus::proxy::MethodFlags::NoAutoStart.into(),
                    &(uid as i64,),
                )
                .await
                .ok()??;
            if !path.as_str().starts_with("/org/freedesktop/Accounts/User") {
                return None;
            }
            let properties = zbus::Proxy::new(
                &connection,
                owner.as_str(),
                path.as_str(),
                "org.freedesktop.DBus.Properties",
            )
            .await
            .ok()?;
            async fn get(
                properties: &zbus::Proxy<'_>,
                key: &str,
            ) -> Option<zbus::zvariant::OwnedValue> {
                properties
                    .call_with_flags(
                        "Get",
                        zbus::proxy::MethodFlags::NoAutoStart.into(),
                        &("org.freedesktop.Accounts.User", key),
                    )
                    .await
                    .ok()?
            }
            if u64::try_from(&get(&properties, "Uid").await?).ok()? != uid as u64 {
                return None;
            }
            let value = get(&properties, "Language").await?;
            let locale = locale(<&str>::try_from(&value).ok()?)?;
            if u64::try_from(&get(&properties, "Uid").await?).ok()? != uid as u64 {
                return None;
            }
            let after_id: String = bus.call("GetId", &()).await.ok()?;
            let after_owner: String = bus
                .call("GetNameOwner", &("org.freedesktop.Accounts",))
                .await
                .ok()?;
            (id == after_id && owner == after_owner).then_some(Primary {
                locale,
                uid,
                bus: id,
                owner,
                path: path.to_string(),
            })
        },
        async {
            async_io::Timer::after(Duration::from_secs(8)).await;
            None
        },
    ))
}

fn locale(value: &str) -> Option<String> {
    super::Target {
        locale: Some(value.into()),
        ..Default::default()
    }
    .validate()
    .ok()?;
    super::region(value)?;
    Some(value.into())
}

#[cfg(target_os = "linux")]
pub(super) fn read() -> Option<State> {
    if !super::repair_linux_desktop::gnome(
        &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
    ) {
        return None;
    }
    let source = [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_RUNTIME_DIR",
        "XDG_DATA_DIRS",
        "GSETTINGS_BACKEND",
        "GSETTINGS_SCHEMA_DIR",
        "DCONF_PROFILE",
        "DBUS_SESSION_BUS_ADDRESS",
        "DBUS_SYSTEM_BUS_ADDRESS",
    ]
    .into_iter()
    .map(|key| {
        let value = match std::env::var_os(key) {
            Some(value) => Some(value.into_string().ok()?),
            None => None,
        };
        Some((key.into(), value))
    })
    .collect::<Option<_>>()?;
    let raw = super::linux::gsetting("org.gnome.system.locale", "region")?;
    let raw = raw.strip_prefix('\'')?.strip_suffix('\'')?.to_string();
    let inherited = if raw.is_empty() {
        Some(primary()?)
    } else {
        None
    };
    let effective = inherited
        .as_ref()
        .map_or(raw.as_str(), |primary| primary.locale.as_str());
    let writable = match super::command::read(
        "/usr/bin/gsettings",
        &["writable", "org.gnome.system.locale", "region"],
    )?
    .as_str()
    {
        "true" => true,
        "false" => false,
        _ => return None,
    };
    if super::linux::gsetting("org.gnome.system.locale", "region")? != format!("'{raw}'") {
        return None;
    }
    Some(State {
        locale: locale(&effective)?,
        raw,
        writable,
        uid: unsafe { libc::geteuid() },
        source,
        inherited,
    })
}

impl State {
    pub(super) fn explicit(&self, expected: &str) -> Self {
        Self {
            locale: expected.into(),
            raw: expected.into(),
            inherited: None,
            ..self.clone()
        }
    }
}

#[cfg(target_os = "linux")]
impl State {
    pub(super) fn available(&self, expected: &str) -> bool {
        self.writable
            && locale(expected).is_some()
            && super::repair_linux::installed_locale(expected)
    }
}

#[cfg(target_os = "linux")]
pub(super) fn write(before: &State, expected: &str) -> bool {
    before.available(expected)
        && guarded(before, read().as_ref(), || {
            super::command::read(
                "/usr/bin/gsettings",
                &[
                    "set",
                    "org.gnome.system.locale",
                    "region",
                    &format!("'{expected}'"),
                ],
            )
            .is_some()
        })
}

fn guarded(before: &State, current: Option<&State>, write: impl FnOnce() -> bool) -> bool {
    current == Some(before) && write()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats_need_a_known_country_and_preserve_script_codeset_and_modifier() {
        for value in ["en_GB.UTF-8", "sr_RS@latin", "zh_Hant_TW.UTF-8"] {
            assert_eq!(locale(value).as_deref(), Some(value));
        }
        for value in ["", "C", "C.UTF-8", "en", "en_US;touch /tmp/unexpected"] {
            assert!(locale(value).is_none());
        }
    }
    #[test]
    fn region_write_refuses_changed_inheritance_policy_identity_or_source() {
        let before = State {
            locale: "en_GB.UTF-8".into(),
            raw: "".into(),
            writable: true,
            uid: 1000,
            source: Default::default(),
            inherited: Some(Primary {
                locale: "en_GB.UTF-8".into(),
                uid: 1000,
                bus: "fixture-bus".into(),
                owner: ":1.2".into(),
                path: "/org/freedesktop/Accounts/User1000".into(),
            }),
        };
        let mut variants = vec![before.clone(); 4];
        variants[0].raw = before.locale.clone();
        variants[0].inherited = None;
        variants[1].writable = false;
        variants[2].uid += 1;
        variants[3]
            .source
            .insert("HOME".into(), Some("/YOUR_OTHER_HOME".into()));
        for current in variants {
            assert!(!guarded(&before, Some(&current), || panic!(
                "No stale write"
            )));
        }
        assert!(!guarded(&before, None, || panic!("No unknown write")));
        assert!(guarded(&before, Some(&before), || true));
    }
    #[test]
    fn effective_match_does_not_verify_reset_inheritance_or_a_different_source() {
        let before = State {
            locale: "en_GB.UTF-8".into(),
            raw: "en_GB.UTF-8".into(),
            writable: true,
            uid: 1000,
            source: Default::default(),
            inherited: None,
        };
        let expected = before.explicit("en_US.UTF-8");
        let mut reset = expected.clone();
        reset.raw.clear();
        assert_ne!(
            serde_json::to_value(&reset).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let mut moved = expected.clone();
        moved
            .source
            .insert("XDG_CONFIG_HOME".into(), Some("/YOUR_OTHER_CONFIG".into()));
        assert!(!guarded(&expected, Some(&reset), || panic!(
            "No reset readback"
        )));
        assert!(!guarded(&expected, Some(&moved), || panic!(
            "No source rebind"
        )));
        assert!(guarded(&expected, Some(&expected), || true));
    }
}
