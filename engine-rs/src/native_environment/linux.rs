//! Linux observations use machine-readable systemd properties, with file fallbacks.
#[cfg(target_os = "linux")]
use super::{command::read, Observation};
use std::collections::BTreeMap;

pub(super) fn properties(text: &str) -> BTreeMap<&str, &str> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .collect()
}

fn boolean(value: Option<&&str>) -> Option<bool> {
    match value.copied() {
        Some("yes" | "true") => Some(true),
        Some("no" | "false") => Some(false),
        _ => None,
    }
}

fn language_tag(locale: &str) -> Option<String> {
    // POSIX modifiers can select a script; dropping one can change the language.
    if locale.contains('@') {
        return None;
    }
    let tag = locale.split('.').next()?.replace('_', "-");
    let language = tag.split('-').next()?;
    (language.len() >= 2
        && language.len() <= 8
        && language.bytes().all(|c| c.is_ascii_alphabetic())
        && !matches!(language, "POSIX"))
    .then_some(tag)
}

fn property(text: &str, signature: &str) -> Option<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    (value["type"] == signature)
        .then(|| value.get("data").cloned())
        .flatten()
}

// GetAll preserves typed arrays. RuntimeNTPServers is absent before systemd 251.
fn timesync_values(text: &str) -> Option<BTreeMap<String, Vec<String>>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value["type"] != "a{sv}" || value["data"].as_array()?.len() != 1 {
        return None;
    }
    let object = value["data"][0].as_object()?;
    let mut values = BTreeMap::new();
    for key in ["SystemNTPServers", "LinkNTPServers", "RuntimeNTPServers"] {
        let Some(value) = object.get(key) else {
            if key == "RuntimeNTPServers" {
                continue;
            } else {
                return None;
            }
        };
        if value["type"] != "as" {
            return None;
        }
        values.insert(
            key.into(),
            value["data"]
                .as_array()?
                .iter()
                .map(|value| value.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()?,
        );
    }
    Some(values)
}

#[cfg(target_os = "linux")]
pub(super) fn gsetting(schema: &str, key: &str) -> Option<String> {
    read("/usr/bin/gsettings", &["get", schema, key])
}

#[cfg(target_os = "linux")]
fn user_locale() -> (Option<Vec<String>>, Option<String>, Option<bool>) {
    let gnome = super::repair_linux_desktop::gnome(
        &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
    );
    if !gnome {
        // Other desktops may override AccountsService; their defaults are not
        // evidence of this user's actual desktop configuration.
        return (None, None, None);
    }
    let mut locales = Vec::<String>::new();
    if let Some(primary) = super::linux_accounts::read() {
        locales.push(primary.locale);
        // Languages is installer-interest metadata, not ordered UI fallbacks.
    }
    let region = if gnome {
        gsetting("org.gnome.system.locale", "region")
            .and_then(|value| {
                value
                    .strip_prefix('\'')
                    .and_then(|text| text.strip_suffix('\''))
                    .map(str::to_string)
            })
            .and_then(|locale| {
                if locale.is_empty() {
                    locales.first().cloned()
                } else {
                    Some(locale)
                }
            })
    } else {
        None
    };
    let automatic = if gnome {
        gsetting("org.gnome.desktop.datetime", "automatic-timezone").and_then(|value| {
            match value.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            }
        })
    } else {
        None
    };
    (
        if locales.is_empty() {
            None
        } else {
            locales.iter().map(|value| language_tag(value)).collect()
        },
        region,
        automatic,
    )
}

#[cfg(target_os = "linux")]
pub(super) fn bus_property(
    service: &str,
    path: &str,
    interface: &str,
    key: &str,
    user: bool,
    signature: &str,
) -> Option<serde_json::Value> {
    let text = read(
        "/usr/bin/busctl",
        &[
            if user { "--user" } else { "--system" },
            "--json=short",
            "get-property",
            service,
            path,
            interface,
            key,
        ],
    )?;
    property(&text, signature)
}

#[cfg(target_os = "linux")]
pub(super) fn collect() -> Observation {
    let time = read("/usr/bin/timedatectl", &["show"]).unwrap_or_default();
    let values = properties(&time);
    let locale = bus_property(
        "org.freedesktop.locale1",
        "/org/freedesktop/locale1",
        "org.freedesktop.locale1",
        "Locale",
        false,
        "as",
    )
    .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok());
    let (languages, user_locale, automatic_timezone) = user_locale();
    // show-timesync/get-property can auto-activate a stopped clock service.
    // `call` applies --auto-start=no even on systemd 249; get-property does not.
    let ntp = read(
        "/usr/bin/busctl",
        &[
            "--system",
            "--auto-start=no",
            "--json=short",
            "call",
            "org.freedesktop.timesync1",
            "/org/freedesktop/timesync1",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            "s",
            "org.freedesktop.timesync1.Manager",
        ],
    )
    .and_then(|value| timesync_values(&value))
    .unwrap_or_default();
    let servers: Vec<_> = ["RuntimeNTPServers", "SystemNTPServers", "LinkNTPServers"]
        .iter()
        .filter_map(|key| ntp.get(*key))
        .flat_map(|value| value.iter().map(String::as_str))
        .collect();
    let mut observed = Observation {
        platform: Some("linux"),
        timezone: values
            .get("Timezone")
            .filter(|value| !value.is_empty())
            .map(|value| (*value).into())
            .or_else(|| {
                std::fs::read_link("/etc/localtime")
                    .ok()?
                    .to_str()?
                    .split("/zoneinfo/")
                    .nth(1)
                    .map(str::to_string)
            }),
        apple_locale: user_locale,
        languages,
        automatic_timezone,
        format_overrides: locale.map(|list| serde_json::json!(list)),
        automatic_time: boolean(values.get("NTP")),
        time_synchronized: boolean(values.get("NTPSynchronized")),
        ntp_server: if servers.len() == 1 && super::time_probe::valid_host(servers[0]) {
            Some(servers[0].trim_end_matches('.').to_ascii_lowercase())
        } else {
            None
        },
        local_time: read("/bin/date", &["+%Y-%m-%d %H:%M:%S %Z %z"]),
        os_version: std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|text| {
                properties(&text)
                    .get("PRETTY_NAME")
                    .map(|value| value.trim_matches('"').to_string())
            }),
        env: super::command::process_environment(),
        ..Observation::default()
    };
    observed.sources.extend([
        ("ntp.actual_source", "time service peer unavailable"),
        ("os.version", "/etc/os-release"),
        (
            "time.timezone",
            "timedatectl Timezone; /etc/localtime fallback",
        ),
        (
            "time.automatic_timezone",
            "GNOME automatic-timezone preference; geolocation/switching effect unverified",
        ),
        (
            "language.preferred",
            "GNOME current-UID AccountsService Language (primary UI preference only)",
        ),
        (
            "language.region",
            "GNOME org.gnome.system.locale region; current-user language inheritance",
        ),
        ("language.locale", "GNOME current-user regional locale"),
        (
            "language.formats",
            "systemd-localed Locale (system defaults)",
        ),
        (
            "ntp.server",
            "timesync1 GetAll configured server lists (no service activation)",
        ),
        ("ntp.automatic_time", "timedatectl NTP"),
        ("ntp.service", "time daemon service unavailable"),
        (
            "ntp.synchronized",
            "timedatectl NTPSynchronized (kernel state)",
        ),
        ("ntp.kernel_max_error", "kernel error unavailable"),
        ("ntp.kernel_estimated_error", "kernel error unavailable"),
    ]);
    observed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timesync_typed_arrays_preserve_v249_support_and_reject_uncertain_shapes() {
        let value = serde_json::json!({"type":"a{sv}","data":[{"SystemNTPServers":{"type":"as","data":["time.example"]},"LinkNTPServers":{"type":"as","data":[]}}]});
        let result = timesync_values(&value.to_string()).unwrap();
        assert_eq!(result["SystemNTPServers"], ["time.example"]);
        assert!(!result.contains_key("RuntimeNTPServers"));
        for field in [
            serde_json::json!({"type":"s","data":"time.example"}),
            serde_json::json!({"type":"as","data":[false]}),
            serde_json::Value::Null,
        ] {
            let mut invalid = value.clone();
            invalid["data"][0]["SystemNTPServers"] = field;
            assert!(timesync_values(&invalid.to_string()).is_none());
        }
        for data in [
            serde_json::json!([]),
            serde_json::json!([{}, {}]),
            serde_json::json!(["wrong"]),
        ] {
            let mut invalid = value.clone();
            invalid["data"] = data;
            assert!(timesync_values(&invalid.to_string()).is_none());
        }
    }
    #[test]
    fn missing_properties_are_unknown_and_values_do_not_require_localized_labels() {
        let values = properties("Timezone=Asia/Singapore\nNTP=yes\nNTPSynchronized=no\nUnknown=\n");
        assert_eq!(values["Timezone"], "Asia/Singapore");
        assert_eq!(boolean(values.get("NTP")), Some(true));
        assert_eq!(boolean(values.get("NTPSynchronized")), Some(false));
        assert_eq!(boolean(values.get("missing")), None);
        assert_eq!(boolean(values.get("Unknown")), None);
        assert_eq!(language_tag("en_US.UTF-8").as_deref(), Some("en-US"));
        assert_eq!(
            language_tag("zh_Hant_TW.UTF-8").as_deref(),
            Some("zh-Hant-TW")
        );
        for locale in [
            "C",
            "C.UTF-8",
            "POSIX",
            "",
            "sr_RS@latin",
            "zh_Hant_TW@calendar=chinese",
        ] {
            assert!(language_tag(locale).is_none());
        }
    }
    #[test]
    fn property_signature_preserves_unknown_and_never_coerces_current_user_identity() {
        assert_eq!(
            property(r#"{"type":"t","data":1000}"#, "t"),
            Some(serde_json::json!(1000))
        );
        assert!(property(r#"{"type":"x","data":1000}"#, "t").is_none());
        assert!(property(r#"{"type":"s","data":"1000"}"#, "t").is_none());
        assert!(property(r#"{"type":"t"}"#, "t").is_none());
        assert_eq!(
            property(r#"{"type":"as","data":["LANG=en_US.UTF-8"]}"#, "as"),
            Some(serde_json::json!(["LANG=en_US.UTF-8"]))
        );
    }
}
