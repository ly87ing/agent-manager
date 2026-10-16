//! Linux user-manager variables are launch inputs, never a desktop-wide locale.
#[cfg(target_os = "linux")]
use super::{
    repair::{Item, Settings, Snapshot},
    Target,
};
#[cfg(target_os = "linux")]
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn environment(value: &Value, keys: &[&str]) -> Option<BTreeMap<String, String>> {
    if value["type"] != "v"
        || value["data"].as_array()?.len() != 1
        || value["data"][0]["type"] != "as"
    {
        return None;
    }
    let values = value["data"][0]["data"].as_array()?;
    let mut selected: BTreeMap<String, String> = keys
        .iter()
        .map(|key| ((*key).into(), String::new()))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        let value = value.as_str()?;
        let Some((name, value)) = value.split_once('=') else {
            return None;
        };
        if keys.contains(&name) {
            if value.len() > 512 || !seen.insert(name) {
                return None;
            }
            selected.insert(name.into(), value.into());
        }
    }
    Some(selected)
}

fn bind(bound: &mut Option<(String, String)>, identity: (String, String)) -> bool {
    match bound {
        Some(bound) => *bound == identity,
        None => {
            *bound = Some(identity);
            true
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
struct Manager {
    bus_id: String,
    owner: String,
    uid: u32,
    values: BTreeMap<String, String>,
}

#[cfg(target_os = "linux")]
fn call(args: &[&str]) -> Option<String> {
    let mut fixed = vec!["--user", "--auto-start=no", "--json=short", "call"];
    fixed.extend_from_slice(args);
    super::command::read("/usr/bin/busctl", &fixed)
}

#[cfg(target_os = "linux")]
fn bus(args: &[&str]) -> Option<Value> {
    serde_json::from_str(&call(args)?).ok()
}

#[cfg(target_os = "linux")]
fn manager(keys: &[&str]) -> Option<Manager> {
    let identity = || {
        let id = bus(&[
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetId",
        ])?;
        if id["type"] != "s" || id["data"].as_array()?.len() != 1 {
            return None;
        }
        let id = id["data"][0].as_str()?;
        if id.len() != 32 || !id.bytes().all(|value| value.is_ascii_hexdigit()) {
            return None;
        }
        let value = bus(&[
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            "s",
            "org.freedesktop.systemd1",
        ])?;
        if value["type"] != "s" || value["data"].as_array()?.len() != 1 {
            return None;
        }
        let owner = value["data"][0]
            .as_str()
            .filter(|name| name.starts_with(':'))
            .map(str::to_string)?;
        Some((id.to_string(), owner))
    };
    let before = identity()?;
    let credentials = bus(&[
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetConnectionUnixUser",
        "s",
        &before.1,
    ])?;
    if credentials["type"] != "u" || credentials["data"].as_array()?.len() != 1 {
        return None;
    }
    let uid = credentials["data"][0].as_u64()?;
    if uid != unsafe { libc::geteuid() } as u64 {
        return None;
    }
    let raw = bus(&[
        &before.1,
        "/org/freedesktop/systemd1",
        "org.freedesktop.DBus.Properties",
        "Get",
        "ss",
        "org.freedesktop.systemd1.Manager",
        "Environment",
    ])?;
    let values = environment(&raw, keys)?;
    (identity()? == before).then_some(Manager {
        bus_id: before.0,
        owner: before.1,
        uid: uid as u32,
        values,
    })
}

#[cfg(target_os = "linux")]
fn installed_locale(locale: &str) -> bool {
    let listed = super::command::read("/usr/bin/locale", &["-a"]).is_some_and(|values| {
        values
            .lines()
            .any(|value| super::locale_key(value) == super::locale_key(locale))
    });
    if !listed {
        return false;
    }
    use std::process::Stdio;
    let mut command = crate::process::launch_command("/usr/bin/locale");
    command
        .arg("charmap")
        .env("LANG", locale)
        .env("LC_ALL", locale)
        .env_remove("LANGUAGE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::process::capture_group(&mut command, std::time::Duration::from_secs(8), 4096).is_ok_and(
        |output| {
            output.status.success()
                && output.stderr.is_empty()
                && !output.stdout.is_empty()
                && output.stdout.len() < 4096
        },
    )
}

#[cfg(target_os = "linux")]
#[derive(Default)]
pub(super) struct LinuxSettings {
    target: Target,
    manager: Option<Manager>,
    bound: Option<(String, String)>,
}

#[cfg(target_os = "linux")]
impl Settings for LinuxSettings {
    fn snapshot(&mut self, target: &Target) -> Result<Snapshot> {
        self.target = target.clone();
        let keys = super::repair::session_keys(target);
        self.manager = if keys.is_empty() {
            None
        } else {
            manager(&keys).filter(|manager| {
                bind(
                    &mut self.bound,
                    (manager.bus_id.clone(), manager.owner.clone()),
                )
            })
        };
        let locale = target.locale.as_ref().map(|value| installed_locale(value));
        let zone = target.timezone.as_ref().map(|zone| {
            std::fs::metadata(std::path::Path::new("/usr/share/zoneinfo").join(zone))
                .is_ok_and(|meta| meta.is_file())
        });
        let mut snapshot = Snapshot {
            session_available: self.manager.is_some(),
            session_scope: self
                .manager
                .as_ref()
                .map(|manager| {
                    format!(
                        "systemd user manager UID {} ({})",
                        manager.uid, manager.owner
                    )
                })
                .unwrap_or_else(|| "systemd user manager unavailable".into()),
            context: json!({"manager":self.manager,"locale_installed":locale,"zone_installed":zone}),
            ..Snapshot::default()
        };
        snapshot.unavailable.extend(
            [
                "time.timezone",
                "time.automatic_timezone",
                "ntp.server",
                "ntp.automatic_time",
                "language.preferred",
                "language.region",
            ]
            .map(str::to_string),
        );
        for key in &keys {
            let id = format!("session.{key}");
            snapshot.values.insert(
                id.clone(),
                self.manager
                    .as_ref()
                    .and_then(|manager| manager.values.get(*key))
                    .map(|value| json!(value)),
            );
            if if *key == "TZ" {
                zone != Some(true)
            } else {
                locale != Some(true)
            } {
                snapshot.unavailable.insert(id);
            }
        }
        Ok(snapshot)
    }
    fn system(&mut self, _: &[Item]) -> Result<(String, Vec<Item>, Option<bool>)> {
        anyhow::bail!("No verified Linux system setting is available in this preview.");
    }
    fn write_user(&mut self, item: &Item) -> bool {
        let Some(key) = item.id.strip_prefix("session.") else {
            return false;
        };
        let keys = super::repair::session_keys(&self.target);
        if !keys.contains(&key) {
            return false;
        }
        let Some(value) = item.expected.as_str() else {
            return false;
        };
        let Some(current) = manager(&keys) else {
            return false;
        };
        if self.manager.as_ref() != Some(&current) {
            return false;
        }
        call(&[
            &current.owner,
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
            "SetEnvironment",
            "as",
            "1",
            &format!("{key}={value}"),
        ])
        .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repair_never_rebinds_to_a_restarted_manager_or_bus() {
        let mut bound = None;
        assert!(bind(&mut bound, ("bus-a".into(), ":1.2".into())));
        assert!(bind(&mut bound, ("bus-a".into(), ":1.2".into())));
        assert!(!bind(&mut bound, ("bus-a".into(), ":1.3".into())));
        assert!(!bind(&mut bound, ("bus-b".into(), ":1.2".into())));
        assert_eq!(bound, Some(("bus-a".into(), ":1.2".into())));
    }
    #[test]
    fn manager_environment_keeps_only_selected_regional_values_and_known_absence() {
        let value = json!({"type":"v","data":[{"type":"as","data":["LANG=en_GB.UTF-8","LC_TIME=en_GB.UTF-8","OTHER_PRIVATE_VALUE=discarded"]}]});
        let values = environment(&value, &["TZ", "LANG"]).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values["TZ"], "");
        assert_eq!(values["LANG"], "en_GB.UTF-8");
        for data in [
            json!(["LANG=a", "LANG=b"]),
            json!(["LANG"]),
            json!([false]),
            json!([format!("LANG={}", "a".repeat(513))]),
        ] {
            let mut invalid = value.clone();
            invalid["data"][0]["data"] = data;
            assert!(environment(&invalid, &["LANG"]).is_none());
        }
        let mut invalid = value;
        invalid["data"][0]["type"] = json!("s");
        assert!(environment(&invalid, &["LANG"]).is_none());
    }
}
