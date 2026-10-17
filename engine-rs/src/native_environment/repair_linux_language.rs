//! AccountsService primary preference, not a desktop-wide fallback language list.
use super::linux_accounts::Primary;

#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct State {
    pub primary: Primary,
    languages: Vec<String>,
}

fn coherent(primary: &Primary, languages: &[String]) -> bool {
    !languages.is_empty()
        && languages.len() <= 16
        && languages.first() == Some(&primary.locale)
        && languages.iter().all(|value| {
            super::Target {
                locale: Some(value.clone()),
                ..Default::default()
            }
            .validate()
            .is_ok()
        })
}

#[cfg(target_os = "linux")]
pub(super) fn read() -> Option<State> {
    let primary = super::linux_accounts::read()?;
    let languages = || -> Option<Vec<String>> {
        let raw = super::command::read(
            "/usr/bin/busctl",
            &[
                "--system",
                "--auto-start=no",
                "--json=short",
                "call",
                &primary.owner,
                &primary.path,
                "org.freedesktop.DBus.Properties",
                "Get",
                "ss",
                "org.freedesktop.Accounts.User",
                "Languages",
            ],
        )?;
        let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
        if value["type"] != "v"
            || value["data"].as_array()?.len() != 1
            || value["data"][0]["type"] != "as"
        {
            return None;
        }
        value["data"][0]["data"]
            .as_array()?
            .iter()
            .map(|value| value.as_str().map(str::to_string))
            .collect()
    };
    let values = languages()?;
    (coherent(&primary, &values)
        && super::linux_accounts::read().as_ref() == Some(&primary)
        && languages()? == values)
        .then_some(State {
            primary,
            languages: values,
        })
}

impl State {
    pub(super) fn selected(&self, locale: &str) -> Self {
        let mut state = self.clone();
        state.primary.locale = locale.into();
        state.languages[0] = locale.into();
        state
    }
}

#[cfg(target_os = "linux")]
pub(super) fn installed(languages: &[String]) -> Option<String> {
    let [language] = languages else {
        return None;
    };
    super::command::read("/usr/bin/locale", &["-a"])?
        .lines()
        .find(|locale| {
            locale
                .split_once('.')
                .is_some_and(|(_, encoding)| encoding.replace('-', "").eq_ignore_ascii_case("utf8"))
                && super::linux::language_tag(locale)
                    .is_some_and(|tag| tag.eq_ignore_ascii_case(language))
                && super::repair_linux::installed_locale(locale)
        })
        .map(str::to_string)
}

#[cfg(target_os = "linux")]
pub(super) fn write(
    before: &State,
    expected: &State,
    authorization: &super::repair_linux_time::Authorization,
    unchanged: impl FnOnce() -> bool,
) -> &'static str {
    use std::time::Duration;
    let connection = zbus::blocking::connection::Builder::system()
        .map(|builder| builder.method_timeout(Duration::from_secs(8)))
        .and_then(|builder| builder.build());
    let Ok(connection) = connection else {
        return "outcome_unknown";
    };
    let Ok(proxy) = zbus::blocking::Proxy::new(
        &connection,
        before.primary.owner.as_str(),
        before.primary.path.as_str(),
        "org.freedesktop.Accounts.User",
    ) else {
        return "outcome_unknown";
    };
    if read().as_ref() != Some(before) || !unchanged() || authorization.failure().is_some() {
        return "stale";
    }
    // No interactive flag: authorization is already over and the baseline fresh.
    match proxy.call_with_flags::<_, _, ()>(
        "SetLanguages",
        zbus::proxy::MethodFlags::NoAutoStart.into(),
        &(&expected.languages,),
    ) {
        Ok(Some(())) => "written",
        Err(zbus::Error::MethodError(name, ..))
            if matches!(
                name.as_str(),
                "org.freedesktop.Accounts.Error.PermissionDenied"
                    | "org.freedesktop.DBus.Error.AccessDenied"
                    | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired"
                    | "org.freedesktop.DBus.Error.InvalidArgs"
            ) =>
        {
            "failed"
        }
        _ => "outcome_unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_selection_preserves_metadata_and_refuses_inconsistent_evidence() {
        let state = State {
            primary: Primary {
                locale: "en_US.UTF-8".into(),
                uid: 1000,
                bus: "YOUR_BUS".into(),
                owner: ":1.23".into(),
                path: "/org/freedesktop/Accounts/User1000".into(),
            },
            languages: vec![
                "en_US.UTF-8".into(),
                "de_DE.UTF-8".into(),
                "en_GB.UTF-8".into(),
            ],
        };
        assert!(coherent(&state.primary, &state.languages));
        assert!(!coherent(&state.primary, &[]));
        assert!(!coherent(&state.primary, &["de_DE.UTF-8".into()]));
        let selected = state.selected("fr_FR.utf8");
        assert_eq!(selected.primary.locale, "fr_FR.utf8");
        assert_eq!(selected.languages[0], "fr_FR.utf8");
        assert_eq!(&selected.languages[1..], &state.languages[1..]);
        assert_eq!(selected.primary.uid, state.primary.uid);
        assert_eq!(selected.primary.owner, state.primary.owner);
        assert!(coherent(&selected.primary, &selected.languages));
    }
}
