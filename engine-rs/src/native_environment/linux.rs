//! Linux observations use machine-readable systemd properties, with file fallbacks.
#[cfg(target_os = "linux")]
use super::{command::read, Observation};
use std::collections::BTreeMap;

pub(super) fn properties(text: &str) -> BTreeMap<&str, &str> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .collect()
}

pub(super) fn language_tag(locale: &str) -> Option<String> {
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

fn property(value: &serde_json::Value, signature: &str) -> Option<serde_json::Value> {
    (value["type"] == signature)
        .then(|| value.get("data").cloned())
        .flatten()
}

fn all_properties(text: &str) -> Option<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    (value["type"] == "a{sv}"
        && value["data"].as_array()?.len() == 1
        && value["data"][0].is_object())
    .then(|| value["data"][0].clone())
}

// GetAll preserves typed arrays. RuntimeNTPServers is absent before systemd 251.
fn timesync_values(text: &str) -> Option<BTreeMap<String, Vec<String>>> {
    let values = all_properties(text)?;
    let object = values.as_object()?;
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

fn timesync_server(values: &BTreeMap<String, Vec<String>>) -> Option<String> {
    // A nonempty runtime list supersedes the system list, but link servers
    // still participate. This is configuration evidence, not the current peer.
    let manual = values
        .get("RuntimeNTPServers")
        .filter(|servers| !servers.is_empty())
        .or_else(|| values.get("SystemNTPServers"));
    let servers: Vec<_> = manual
        .into_iter()
        .chain(values.get("LinkNTPServers"))
        .flatten()
        .collect();
    (servers.len() == 1 && super::time_probe::valid_host(servers[0]))
        .then(|| servers[0].trim_end_matches('.').to_ascii_lowercase())
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
fn system_properties(service: &str, path: &str, interface: &str) -> Option<String> {
    // `call` honors NoAutoStart even on systemd 249; get-property does not.
    read(
        "/usr/bin/busctl",
        &[
            "--system",
            "--auto-start=no",
            "--json=short",
            "call",
            service,
            path,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            "s",
            interface,
        ],
    )
}

#[cfg(target_os = "linux")]
pub(super) fn collect() -> Observation {
    let kde = super::linux_kde::kde(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default());
    let values = system_properties(
        "org.freedesktop.timedate1",
        "/org/freedesktop/timedate1",
        "org.freedesktop.timedate1",
    )
    .and_then(|text| all_properties(&text))
    .unwrap_or_default();
    let locale = system_properties(
        "org.freedesktop.locale1",
        "/org/freedesktop/locale1",
        "org.freedesktop.locale1",
    )
    .and_then(|text| all_properties(&text))
    .and_then(|values| property(&values["Locale"], "as"))
    .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok());
    let (mut languages, mut user_locale, automatic_timezone) = if kde {
        (None, None, None)
    } else {
        user_locale()
    };
    let kde_preferences = kde.then(super::linux_kde::read).flatten();
    if let Some(preferences) = &kde_preferences {
        languages = preferences.languages.clone();
        user_locale = preferences.locale.clone();
    }
    let ntp = system_properties(
        "org.freedesktop.timesync1",
        "/org/freedesktop/timesync1",
        "org.freedesktop.timesync1.Manager",
    )
    .and_then(|value| timesync_values(&value))
    .unwrap_or_default();
    let mut observed = Observation {
        platform: Some("linux"),
        timezone: property(&values["Timezone"], "s")
            .and_then(|value| {
                value
                    .as_str()
                    .filter(|zone| !zone.is_empty())
                    .map(str::to_string)
            })
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
        format_overrides: if kde {
            kde_preferences.and_then(|preferences| preferences.formats)
        } else {
            locale.map(|list| serde_json::json!(list))
        },
        automatic_time: property(&values["NTP"], "b").and_then(|value| value.as_bool()),
        time_synchronized: property(&values["NTPSynchronized"], "b")
            .and_then(|value| value.as_bool()),
        ntp_server: timesync_server(&ntp),
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
            "timedate1 Timezone (no service activation); /etc/localtime fallback",
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
            "timesync1 runtime-or-system and link server candidates (no service activation; not current peer)",
        ),
        ("ntp.automatic_time", "timedate1 NTP (no service activation)"),
        ("ntp.service", "time daemon service unavailable"),
        (
            "ntp.synchronized",
            "timedate1 NTPSynchronized (kernel state; no service activation)",
        ),
        ("ntp.kernel_max_error", "kernel error unavailable"),
        ("ntp.kernel_estimated_error", "kernel error unavailable"),
    ]);
    if kde {
        observed.sources.extend([
            ("language.preferred", "KDE plasma-localerc Translations LANGUAGE (KConfig defaults; saved order, session adoption unverified)"),
            ("language.region", "KDE plasma-localerc Formats LANG (KConfig defaults; saved base region)"),
            ("language.locale", "KDE plasma-localerc Formats LANG (saved preference; session adoption unverified)"),
            ("language.formats", "KDE plasma-localerc Formats LC_* (KConfig defaults; saved overrides)"),
        ]);
    }
    observed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_servers_supersede_system_servers_without_hiding_link_candidates() {
        let configured = |runtime: Option<Vec<&str>>, system: Vec<&str>, link: Vec<&str>| {
            let mut values = BTreeMap::from([
                (
                    "SystemNTPServers".into(),
                    system.into_iter().map(str::to_string).collect(),
                ),
                (
                    "LinkNTPServers".into(),
                    link.into_iter().map(str::to_string).collect(),
                ),
            ]);
            if let Some(runtime) = runtime {
                values.insert(
                    "RuntimeNTPServers".into(),
                    runtime.into_iter().map(str::to_string).collect(),
                );
            }
            timesync_server(&values)
        };
        assert_eq!(
            configured(Some(vec!["Current.Example."]), vec!["old.example"], vec![]).as_deref(),
            Some("current.example")
        );
        for runtime in [None, Some(vec![])] {
            assert_eq!(
                configured(runtime.clone(), vec!["system.example"], vec![]).as_deref(),
                Some("system.example")
            );
            assert_eq!(
                configured(runtime, vec![], vec!["link.example"]).as_deref(),
                Some("link.example")
            );
        }
        for (runtime, system, link) in [
            (
                Some(vec!["current.example"]),
                vec!["old.example"],
                vec!["link.example"],
            ),
            (
                Some(vec!["one.example", "two.example"]),
                vec!["old.example"],
                vec![],
            ),
            (
                Some(vec!["https://invalid.example"]),
                vec!["system.example"],
                vec![],
            ),
            (Some(vec![]), vec!["system.example"], vec!["link.example"]),
            (None, vec![], vec![]),
        ] {
            assert!(configured(runtime, system, link).is_none());
        }
    }
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
        let values = all_properties(r#"{"type":"a{sv}","data":[{"Timezone":{"type":"s","data":"Asia/Singapore"},"NTP":{"type":"b","data":true},"NTPSynchronized":{"type":"b","data":false},"Unknown":{"type":"b","data":"false"}}]}"#).unwrap();
        assert_eq!(
            property(&values["Timezone"], "s"),
            Some(serde_json::json!("Asia/Singapore"))
        );
        assert_eq!(
            property(&values["NTP"], "b").and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            property(&values["NTPSynchronized"], "b").and_then(|value| value.as_bool()),
            Some(false)
        );
        assert_eq!(property(&values["missing"], "b"), None);
        assert_eq!(
            property(&values["Unknown"], "b").and_then(|value| value.as_bool()),
            None
        );
        assert!(property(&values["Timezone"], "b").is_none());
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
        let property = |text: &str, signature: &str| {
            super::property(&serde_json::from_str(text).ok()?, signature)
        };
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
