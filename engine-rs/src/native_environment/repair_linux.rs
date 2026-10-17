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

fn ntp_extras(
    server: &str,
    saved: Option<&str>,
    directory_missing: Option<bool>,
    available: bool,
) -> Vec<super::repair::Item> {
    let mut items = Vec::new();
    if directory_missing == Some(true) {
        items.push(super::repair::Item {
            id: "ntp.server.directory".into(),
            scope: "system".into(),
            before: Some(json!(false)),
            expected: json!(true),
            status: if available { "change" } else { "unavailable" }.into(),
        });
    }
    items.push(super::repair::Item {
        id: "ntp.server.persistent".into(),
        scope: "system".into(),
        before: saved.map(|server| json!(server)),
        expected: json!(server),
        status: if !available {
            "unavailable"
        } else if saved == Some(server) {
            "unchanged"
        } else {
            "change"
        }
        .into(),
    });
    items
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
pub(super) fn installed_locale(locale: &str) -> bool {
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
    time: Option<super::repair_linux_time::State>,
    desktop: Option<super::repair_linux_desktop::State>,
    region: Option<super::repair_linux_region::State>,
    ntp: Option<super::repair_linux_ntp::State>,
    context: Value,
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
        self.time = target
            .timezone
            .as_ref()
            .and_then(|_| super::repair_linux_time::read());
        let gnome = super::repair_linux_desktop::gnome(
            &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        );
        self.desktop = if target.timezone.is_some() || target.automatic_timezone.is_some() {
            super::repair_linux_desktop::read()
        } else {
            None
        };
        self.region = target
            .region
            .as_ref()
            .and_then(|_| super::repair_linux_region::read());
        self.ntp = target
            .ntp_server
            .as_ref()
            .and_then(|_| super::repair_linux_ntp::read());
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
            context: json!({"manager":self.manager,"locale_installed":locale,"zone_installed":zone,"time":self.time,"gnome":gnome,"desktop_time":self.desktop,"region":self.region,"ntp":self.ntp}),
            ..Snapshot::default()
        };
        snapshot.unavailable.extend(
            [
                "ntp.server",
                "ntp.automatic_time",
                "language.preferred",
                "language.region",
            ]
            .map(str::to_string),
        );
        if let Some(server) = target.ntp_server.as_deref() {
            let server = server.trim_end_matches('.').to_ascii_lowercase();
            let available = self
                .ntp
                .as_ref()
                .is_some_and(|state| state.available(&server));
            snapshot.values.insert(
                "ntp.server".into(),
                self.ntp.as_ref().map(|state| state.runtime_before()),
            );
            if available {
                snapshot.unavailable.remove("ntp.server");
            }
            snapshot.extras = ntp_extras(
                &server,
                self.ntp
                    .as_ref()
                    .and_then(|state| state.configuration.saved.as_deref()),
                self.ntp
                    .as_ref()
                    .map(|state| state.configuration.directory_missing()),
                available,
            );
        }
        snapshot.values.insert(
            "language.region".into(),
            self.region.as_ref().map(|state| json!(state.locale)),
        );
        if self.region.as_ref().is_some_and(|state| {
            target.region.as_ref().is_some_and(|region| {
                super::repair::region_locale(&state.locale, region)
                    .is_some_and(|locale| state.available(&locale))
            })
        }) {
            snapshot.unavailable.remove("language.region");
        }
        snapshot
            .scope_overrides
            .insert("time.automatic_timezone".into(), "user".into());
        snapshot.values.insert(
            "time.automatic_timezone".into(),
            self.desktop.as_ref().map(|state| json!(state.automatic)),
        );
        if !self.desktop.as_ref().is_some_and(|state| {
            target
                .automatic_timezone
                .is_none_or(|value| state.available(value))
        }) {
            snapshot
                .unavailable
                .insert("time.automatic_timezone".into());
        }
        snapshot.values.insert(
            "time.timezone".into(),
            self.time.as_ref().map(|time| json!(time.zone)),
        );
        if self.time.is_none()
            || (target.automatic_timezone.is_some()
                && snapshot.unavailable.contains("time.automatic_timezone"))
            || (gnome && self.desktop.is_none())
            || (self.desktop.as_ref().is_some_and(|state| state.automatic)
                && target.automatic_timezone != Some(false)
                && target.timezone.as_ref() != self.time.as_ref().map(|state| &state.zone))
            || target
                .timezone
                .as_ref()
                .is_some_and(|zone| !super::repair_linux_time::installed(zone))
        {
            snapshot.unavailable.insert("time.timezone".into());
        }
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
        self.context = snapshot.context.clone();
        Ok(snapshot)
    }
    fn in_system_batch(&self, item: &Item) -> bool {
        item.scope == "system"
            || matches!(
                item.id.as_str(),
                "time.automatic_timezone" | "language.region"
            )
    }
    fn system(&mut self, items: &[Item]) -> Result<(String, Vec<Item>, Option<bool>)> {
        if items.iter().any(|item| {
            !matches!(
                item.id.as_str(),
                "time.timezone"
                    | "time.automatic_timezone"
                    | "language.region"
                    | "ntp.server"
                    | "ntp.server.persistent"
                    | "ntp.server.directory"
            )
        }) {
            anyhow::bail!("Unsupported Linux system repair batch.");
        }
        let target = self.target.clone();
        let mut baseline = self.context.clone();
        let ntp_before = self.ntp.clone();
        let has_ntp = items.iter().any(|item| item.id.starts_with("ntp.server"));
        // Obtain every new Settings authorization before any system or user
        // preference write. The NTP helper remains zero-write at READY.
        let ntp_authorization = if has_ntp {
            Some(
                ntp_before
                    .as_ref()
                    .zip(target.ntp_server.as_deref())
                    .ok_or("unavailable")
                    .and_then(|(before, server)| {
                        super::repair_linux_ntp_authorization::authorize(before, server)
                    }),
            )
        } else {
            None
        };
        let ntp_failure = ntp_authorization
            .as_ref()
            .and_then(|result| result.as_ref().err().copied());
        let authorization = items
            .iter()
            .any(|item| item.id == "time.timezone")
            .then_some(())
            .filter(|_| ntp_failure.is_none())
            .map(|_| super::repair_linux_time::authorize());
        let mut results = Vec::new();
        let failure =
            ntp_failure.or_else(|| authorization.as_ref().and_then(|auth| auth.failure()));
        if let Some(status) = failure {
            let mut result = items
                .iter()
                .find(|item| {
                    if ntp_failure.is_some() {
                        item.id.starts_with("ntp.server")
                    } else {
                        item.id == "time.timezone"
                    }
                })
                .unwrap_or(&items[0])
                .clone();
            result.status = if status == "cancelled" {
                "not_applied"
            } else {
                status
            }
            .into();
            results.push(result);
            return Ok((
                if status == "cancelled" {
                    "cancelled"
                } else {
                    "partial"
                }
                .into(),
                results,
                None,
            ));
        }
        if let Some(Ok(authorized)) = ntp_authorization {
            if self.snapshot(&target)?.context != baseline {
                let mut result = items
                    .iter()
                    .find(|item| item.id.starts_with("ntp.server"))
                    .unwrap()
                    .clone();
                result.status = "stale".into();
                return Ok(("partial".into(), vec![result], None));
            }
            let before = ntp_before.as_ref().expect("NTP authorization baseline");
            let server = target.ntp_server.as_deref().expect("NTP target");
            let server = server.trim_end_matches('.').to_ascii_lowercase();
            let outcome = authorized.apply(before, &server, |expected| {
                let mut projected = baseline.clone();
                projected["ntp"] = json!(expected);
                self.snapshot(&target)
                    .is_ok_and(|snapshot| snapshot.context == projected)
            });
            for item in items
                .iter()
                .filter(|item| item.id.starts_with("ntp.server"))
            {
                let mut result = item.clone();
                result.status = match item.id.as_str() {
                    "ntp.server.persistent" => &outcome.saved,
                    "ntp.server.directory" => &outcome.directory,
                    _ => &outcome.runtime,
                }
                .clone();
                results.push(result);
            }
            if !outcome.complete {
                return Ok(("partial".into(), results, None));
            }
            baseline["ntp"] = json!(outcome.expected.expect("independently verified NTP state"));
        }
        for item in items
            .iter()
            .filter(|item| !item.id.starts_with("ntp.server"))
        {
            let mut result = item.clone();
            if let Some(status) = authorization.as_ref().and_then(|auth| auth.failure()) {
                result.status = if status == "cancelled" {
                    "not_applied"
                } else {
                    status
                }
                .into();
                results.push(result);
                return Ok((
                    if status == "cancelled" {
                        "cancelled"
                    } else {
                        "partial"
                    }
                    .into(),
                    results,
                    None,
                ));
            }
            let snapshot = self.snapshot(&target)?;
            if snapshot.context != baseline || snapshot.unavailable.contains(&item.id) {
                result.status = "stale".into();
                results.push(result);
                return Ok(("partial".into(), results, None));
            }
            if item.id == "language.region" {
                let expected = item
                    .expected
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid GNOME formats locale."))?;
                let before = self
                    .region
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("GNOME formats are unavailable."))?;
                let expected_after = before.explicit(expected);
                let written = super::repair_linux_region::write(before, expected);
                result.status = match super::repair_linux_region::read() {
                    Some(after) if after == expected_after => {
                        if written {
                            "verified"
                        } else {
                            "verified_after_error"
                        }
                    }
                    None => "outcome_unknown",
                    _ if written => "unverified",
                    _ => "failed",
                }
                .into();
                let verified = result.status == "verified";
                results.push(result);
                if !verified {
                    return Ok(("partial".into(), results, None));
                }
                baseline["region"] = json!(expected_after);
            } else if item.id == "time.timezone" {
                let before = self
                    .time
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Linux system timezone is unavailable."))?;
                let desktop = self.desktop.clone();
                let (status, result) = super::repair_linux_time::apply(
                    item,
                    before,
                    authorization.as_ref().expect("timezone authorization"),
                    move || super::repair_linux_desktop::read() == desktop,
                );
                results.push(result);
                if status != "applied" {
                    return Ok((status, results, None));
                }
                let snapshot = self.snapshot(&target)?;
                if snapshot.values.get(&item.id).and_then(Option::as_ref) != Some(&item.expected)
                    || snapshot.context["time"]["adjtime"] != baseline["time"]["adjtime"]
                {
                    results.last_mut().unwrap().status = "unverified".into();
                    return Ok(("partial".into(), results, None));
                }
                baseline["time"] = snapshot.context["time"].clone();
            } else {
                let expected = item
                    .expected
                    .as_bool()
                    .ok_or_else(|| anyhow::anyhow!("Invalid automatic timezone target."))?;
                let before = self
                    .desktop
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("GNOME automatic timezone is unavailable."))?;
                let written = super::repair_linux_desktop::write(before, expected, || {
                    authorization
                        .as_ref()
                        .is_none_or(|auth| auth.failure().is_none())
                });
                let mut expected_after = before.clone();
                expected_after.automatic = expected;
                result.status = match super::repair_linux_desktop::read() {
                    Some(after) if after == expected_after => {
                        if written {
                            "verified"
                        } else {
                            "verified_after_error"
                        }
                    }
                    None => "outcome_unknown",
                    _ if written => "unverified",
                    _ if authorization.as_ref().and_then(|auth| auth.failure())
                        == Some("stale") =>
                    {
                        "stale"
                    }
                    _ => "failed",
                }
                .into();
                let verified = result.status == "verified";
                results.push(result);
                if !verified {
                    return Ok(("partial".into(), results, None));
                }
                baseline["desktop_time"] = json!(expected_after);
            }
            let snapshot = self.snapshot(&target)?;
            if snapshot.context != baseline {
                if snapshot.context["region"] != baseline["region"] {
                    for result in &mut results {
                        if result.id == "language.region" && result.status == "verified" {
                            result.status = if self.region.is_some() {
                                "unverified"
                            } else {
                                "outcome_unknown"
                            }
                            .into();
                        }
                    }
                }
                if snapshot.context["time"] != baseline["time"] {
                    for result in &mut results {
                        if result.id == "time.timezone" && result.status == "verified" {
                            result.status = "unverified".into();
                        }
                    }
                }
                return Ok(("partial".into(), results, None));
            }
        }
        Ok(("applied".into(), results, None))
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
    fn persistent_ntp_preview_is_independent_of_the_runtime_noop() {
        let target: super::super::Target =
            serde_json::from_value(json!({"ntp_server":"selected.example"})).unwrap();
        let mut snapshot = super::super::repair::Snapshot::default();
        snapshot
            .values
            .insert("ntp.server".into(), Some(json!("selected.example")));
        snapshot.extras = ntp_extras("selected.example", None, Some(true), true);
        let plan = super::super::repair::plan(&target, &snapshot).unwrap();
        assert_eq!(
            plan.items
                .iter()
                .find(|item| item.id == "ntp.server")
                .unwrap()
                .status,
            "unchanged"
        );
        assert!(plan
            .items
            .iter()
            .filter(|item| item.id != "ntp.server")
            .all(|item| item.status == "change"));
        assert!(plan
            .items
            .iter()
            .find(|item| item.id == "ntp.server.persistent")
            .unwrap()
            .before
            .is_none());
        let saved = ntp_extras(
            "selected.example",
            Some("selected.example"),
            Some(false),
            true,
        );
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].status, "unchanged");
        assert!(ntp_extras("selected.example", None, None, false)
            .iter()
            .all(|item| item.status == "unavailable"));
    }
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
