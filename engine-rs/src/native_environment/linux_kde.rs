//! Read Plasma's saved preferences through its native KConfig resolver.
use std::collections::BTreeMap;

const FORMATS: &[&str] = &[
    "LANG",
    "LC_ALL",
    "LC_ADDRESS",
    "LC_COLLATE",
    "LC_CTYPE",
    "LC_IDENTIFICATION",
    "LC_MONETARY",
    "LC_MESSAGES",
    "LC_MEASUREMENT",
    "LC_NAME",
    "LC_NUMERIC",
    "LC_PAPER",
    "LC_TELEPHONE",
    "LC_TIME",
];

pub(super) fn kde(desktop: &str) -> bool {
    desktop
        .split(':')
        .any(|part| part.eq_ignore_ascii_case("KDE"))
}

pub(super) struct Preferences {
    pub languages: Option<Vec<String>>,
    pub locale: Option<String>,
    pub formats: Option<serde_json::Value>,
}

fn locale(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    super::Target {
        locale: Some(value.into()),
        ..Default::default()
    }
    .validate()
    .ok()?;
    Some(value.into())
}

fn preferences(values: &BTreeMap<String, String>) -> Preferences {
    let languages = values.get("LANGUAGE").and_then(|value| {
        if value.is_empty() || value.len() > 1024 {
            return None;
        }
        let languages = value
            .split(':')
            .map(super::linux::language_tag)
            .collect::<Option<Vec<_>>>()?;
        if languages.iter().any(|language| {
            language.split('-').any(|part| {
                part.is_empty()
                    || part.len() > 8
                    || !part.bytes().all(|c| c.is_ascii_alphanumeric())
            })
        }) {
            return None;
        }
        super::Target {
            languages: languages.clone(),
            ..Default::default()
        }
        .validate()
        .ok()?;
        Some(languages)
    });
    let formats = FORMATS
        .iter()
        .filter(|key| **key != "LANG")
        .map(|key| {
            let value = values.get(*key)?;
            if value.is_empty() {
                Some(None)
            } else {
                Some(Some((key.to_string(), locale(value)?)))
            }
        })
        .collect::<Option<Vec<_>>>()
        .map(|values| serde_json::json!(values.into_iter().flatten().collect::<BTreeMap<_, _>>()));
    Preferences {
        languages,
        locale: values.get("LANG").and_then(|value| locale(value)),
        formats,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn read() -> Option<Preferences> {
    use std::{
        os::unix::process::CommandExt,
        process::Stdio,
        time::{Duration, Instant},
    };
    let program = ["/usr/bin/kreadconfig6", "/usr/bin/kreadconfig5"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let read = |group: &str, key: &str| {
        let mut command = crate::process::launch_command(program);
        // Qt derives the application config name from argv[0]. Omitting --file
        // keeps FullConfig (including kdeglobals/immutable defaults), matching
        // Plasma. kreadconfig --file would silently switch to NoGlobals.
        command
            .arg0("plasma-locale")
            .args(["--group", group, "--key", key])
            .env("KDE_HOME_READONLY", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        crate::process::deny_settings_file_writes(&mut command);
        let output = crate::process::capture_group(
            &mut command,
            deadline.checked_duration_since(Instant::now())?,
            4096,
        )
        .ok()?;
        if !output.status.success() || output.stdout.len() >= 4096 {
            return None;
        }
        let value = String::from_utf8(output.stdout).ok()?;
        let value = value.strip_suffix('\n')?;
        (value.len() <= 1024 && !value.contains(['\n', '\r', '\0'])).then(|| value.to_string())
    };
    let snapshot = || {
        let mut values = BTreeMap::new();
        for key in FORMATS {
            values.insert((*key).into(), read("Formats", key)?);
        }
        values.insert("LANGUAGE".into(), read("Translations", "LANGUAGE")?);
        Some(values)
    };
    let before = snapshot()?;
    (snapshot()? == before).then(|| preferences(&before))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(language: &str) -> BTreeMap<String, String> {
        let mut values = FORMATS
            .iter()
            .map(|key| ((*key).into(), String::new()))
            .collect::<BTreeMap<_, _>>();
        values.insert("LANG".into(), "en_GB.UTF-8".into());
        values.insert("LANGUAGE".into(), language.into());
        values
    }

    #[test]
    fn saved_order_and_formats_are_independent() {
        let mut values = values("zh_TW:en_GB");
        values.insert("LC_TIME".into(), "fr_FR.UTF-8".into());
        let result = preferences(&values);
        assert_eq!(result.languages.unwrap(), ["zh-TW", "en-GB"]);
        assert_eq!(result.locale.as_deref(), Some("en_GB.UTF-8"));
        assert_eq!(
            result.formats,
            Some(serde_json::json!({"LC_TIME":"fr_FR.UTF-8"}))
        );
        assert!(kde("Plasma:KDE"));
        assert!(!kde("GNOME:not-KDE"));
    }

    #[test]
    fn unknown_language_never_uses_formats_or_process_defaults() {
        for value in [
            "",
            "sr_RS@latin",
            "en_US:",
            "en_US:EN-us",
            "en__US",
            "C",
            "en_US;bad",
        ] {
            let result = preferences(&values(value));
            assert!(result.languages.is_none(), "{value}");
            assert_eq!(result.locale.as_deref(), Some("en_GB.UTF-8"));
        }
        let mut missing = values("");
        missing.insert("LANG".into(), String::new());
        assert!(preferences(&missing).locale.is_none());
        missing.insert("LC_TIME".into(), "not a locale".into());
        assert!(preferences(&missing).formats.is_none());
    }
}
