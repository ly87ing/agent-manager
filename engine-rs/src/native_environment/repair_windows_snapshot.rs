//! Pure Windows repair capabilities and preview values from bounded native observations.
use super::{
    repair::{Item, Snapshot},
    Target,
};
use serde_json::{json, Value};

fn configured_peer(value: &Value) -> Option<String> {
    let configured = value.as_str()?;
    let host = configured.split(',').next()?;
    peer_with_host(configured, host)?;
    Some(host.trim_end_matches('.').to_ascii_lowercase())
}

pub(super) fn peer_with_host(configured: &str, host: &str) -> Option<String> {
    if configured.split_whitespace().count() != 1 || !super::time_probe::valid_host(host) {
        return None;
    }
    let (old, flags) = configured
        .split_once(',')
        .map_or((configured, None), |(old, flags)| (old, Some(flags)));
    if !super::time_probe::valid_host(old) {
        return None;
    }
    if let Some(flags) = flags {
        let number = if let Some(hex) = flags
            .strip_prefix("0x")
            .or_else(|| flags.strip_prefix("0X"))
        {
            u8::from_str_radix(hex, 16).ok()?
        } else {
            flags.parse::<u8>().ok()?
        };
        if number > 0xf || number & 0xc == 0xc {
            return None;
        }
        Some(format!("{host},{flags}"))
    } else {
        Some(host.into())
    }
}

pub(super) fn decode(
    raw: &Value,
    target: &Target,
    persistent: bool,
    formats: Option<&Value>,
    zone: Option<String>,
    region: Option<String>,
) -> Snapshot {
    let system = &raw["system"];
    let user = &raw["user"];
    let observed = super::windows::decode(system);
    let mut snapshot = Snapshot {
        region_is_code: true,
        context: json!({"native":raw,"region":region.clone(),"persistent":persistent}),
        ..Snapshot::default()
    };
    let mut languages: Option<Vec<_>> = user["languages"].as_array().and_then(|values| {
        values
            .iter()
            .map(|value| value["LanguageTag"].as_str().map(str::to_string))
            .collect()
    });
    if languages.as_ref().is_some_and(|values| {
        values.len() == target.languages.len()
            && values
                .iter()
                .zip(&target.languages)
                .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
    }) {
        languages = Some(target.languages.clone());
    }
    for (id, value) in [
        ("time.timezone", zone.map(Value::String)),
        (
            "time.automatic_timezone",
            observed.automatic_timezone.map(Value::Bool),
        ),
        (
            "ntp.server",
            configured_peer(&system["ntp"]).map(Value::String),
        ),
        (
            "ntp.automatic_time",
            observed.automatic_time.map(Value::Bool),
        ),
        (
            "language.preferred",
            languages.as_ref().map(|value| json!(value)),
        ),
        ("language.region", region.clone().map(Value::String)),
        (
            "language.independent_formats",
            user["independent_formats"].as_bool().map(Value::Bool),
        ),
    ] {
        if value.is_none() {
            snapshot.unavailable.insert(id.into());
        }
        snapshot.values.insert(id.into(), value);
    }
    if system["policy"] != false || !matches!(system["type"].as_str(), Some("NTP" | "NoSync")) {
        snapshot
            .unavailable
            .extend(["ntp.server".into(), "ntp.automatic_time".into()]);
    }
    if !target.languages.is_empty() && user["language_supported"] != true {
        snapshot.unavailable.insert("language.preferred".into());
    }
    if system["timezone_policy"]["managed"] != false {
        snapshot.unavailable.insert("time.timezone".into());
    }
    let location = &system["zone_policy"];
    let allowed = location["consent"] == "Allow"
        && location["disable_location"] == 0
        && location["disable_provider"] == 0
        && (location["apps"]["managed"] == false
            || (location["apps"]["managed"] == true
                && matches!(location["apps"]["value"].as_i64(), Some(0 | 1))))
        && (location["master"]["managed"] == false
            || (location["master"]["managed"] == true
                && matches!(location["master"]["value"].as_i64(), Some(1 | 2))));
    if system["zone_status"].is_null() || (target.automatic_timezone == Some(true) && !allowed) {
        snapshot
            .unavailable
            .insert("time.automatic_timezone".into());
    }
    if formats.is_some_and(|before| {
        user["independent_formats"] != true
            || before != &json!({"culture":user["culture"],"formats":user["formats"]})
    }) {
        snapshot.values.insert("language.preferred".into(), None);
        snapshot.unavailable.insert("language.preferred".into());
    }
    if !target.languages.is_empty()
        && languages.is_some()
        && !snapshot.unavailable.contains("language.preferred")
        && languages.as_ref() != Some(&target.languages)
    {
        if user["independent_formats"].is_null()
            || user["formats"].is_null()
            || user["culture"].is_null()
        {
            snapshot.unavailable.insert("language.preferred".into());
        } else if user["independent_formats"] == false {
            snapshot.extras.push(Item {
                id: "language.independent_formats".into(),
                scope: "user".into(),
                before: Some(json!(false)),
                expected: json!(true),
                status: "change".into(),
            });
        }
    }
    snapshot.session_available = persistent && user["variables"].is_object();
    snapshot.session_scope = if persistent {"Windows persistent current-user environment; future logins and new Explorer-launched processes; current checker unchanged"} else {"Windows has no equivalent temporary login environment; persistent user variables require a separate explicit selection"}.into();
    for key in super::repair::session_keys(target) {
        let id = format!("session.{key}");
        let raw = &user["variables"][key];
        let value = if user["variables"].is_object()
            && (raw["kind"].is_null()
                || matches!(raw["kind"].as_str(), Some("String" | "ExpandString")))
        {
            Some(json!(raw["value"].as_str().unwrap_or("")))
        } else {
            None
        };
        snapshot.values.insert(id.clone(), value);
        if persistent {
            snapshot.scope_overrides.insert(id, "user".into());
        }
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacing_a_peer_preserves_polling_and_client_flags() {
        for old in [
            "old.example,0x9",
            "old.example,0x1",
            "old.example,0x8",
            "old.example",
        ] {
            assert_eq!(
                peer_with_host(old, "new.example").as_deref(),
                Some(old.replace("old.example", "new.example").as_str())
            );
        }
        for old in [
            "old.example,0xff",
            "old.example,0xc",
            "old.example,wrong",
            "old.example,0x9,0x8",
            "old.example other.example",
        ] {
            assert!(peer_with_host(old, "new.example").is_none());
        }
    }
    fn fixture() -> (Value, Target) {
        let raw = json!({"system":{"zone_start":3,"zone_status":"Stopped","timezone_policy":{"managed":false},"zone_policy":{"consent":"Allow","disable_location":0,"disable_provider":0,"apps":{"managed":false},"master":{"managed":false}},"policy":false,"type":"NTP","ntp":"time.example,0x8","ntp_enabled":1,"time_start":3},"user":{"language_supported":true,"languages":[{"LanguageTag":"en-GB"}],"independent_formats":false,"culture":"en-GB","formats":{"sShortDate":"dd/MM/yyyy"},"variables":{"TZ":{"kind":null,"value":null}}}});
        let target=serde_json::from_value(json!({"timezone":"Europe/London","automatic_timezone":true,"languages":["en-US"],"locale":"en_US.UTF-8","region":"US","ntp_server":"time.example","automatic_time":true})).unwrap();
        (raw, target)
    }
    fn snapshot(
        raw: &Value,
        target: &Target,
        persistent: bool,
        formats: Option<&Value>,
    ) -> Snapshot {
        decode(
            raw,
            target,
            persistent,
            formats,
            Some("Europe/London".into()),
            Some("GB".into()),
        )
    }
    #[test]
    fn missing_language_evidence_never_changes_the_linked_format_policy() {
        let (mut raw, target) = fixture();
        raw["user"]["languages"] = Value::Null;
        let result = snapshot(&raw, &target, false, None);
        assert!(result.extras.is_empty());
        assert!(result.unavailable.contains("language.preferred"));
        let (mut raw, target) = fixture();
        raw["user"]["languages"] = json!([{"LanguageTag":"EN-us"}]);
        assert!(snapshot(&raw, &target, false, None).extras.is_empty());
        let formats = json!({"culture":raw["user"]["culture"],"formats":raw["user"]["formats"]});
        for value in [Value::Null, json!(false)] {
            raw["user"]["independent_formats"] = value;
            assert!(snapshot(&raw, &target, false, Some(&formats))
                .unavailable
                .contains("language.preferred"));
        }
    }
    #[test]
    fn policies_and_privacy_block_enable_without_blocking_explicit_disable() {
        let (raw, target) = fixture();
        assert!(!snapshot(&raw, &target, false, None)
            .unavailable
            .contains("time.automatic_timezone"));
        for (key, value) in [
            ("consent", json!("Deny")),
            ("disable_location", json!(1)),
            ("disable_provider", Value::Null),
            ("apps", json!({"managed":true,"value":2})),
            ("master", json!({"managed":true,"value":0})),
        ] {
            let mut changed = raw.clone();
            changed["system"]["zone_policy"][key] = value;
            assert!(snapshot(&changed, &target, false, None)
                .unavailable
                .contains("time.automatic_timezone"));
            let mut off = target.clone();
            off.automatic_timezone = Some(false);
            assert!(!snapshot(&changed, &off, false, None)
                .unavailable
                .contains("time.automatic_timezone"));
        }
        for policy in [Value::Null, json!({"managed":true,"value":"UTC"})] {
            let mut changed = raw.clone();
            changed["system"]["timezone_policy"] = policy;
            assert!(snapshot(&changed, &target, false, None)
                .unavailable
                .contains("time.timezone"));
        }
        let mut domain = raw.clone();
        domain["system"]["type"] = json!("NT5DS");
        let result = snapshot(&domain, &target, false, None);
        assert!(
            result.unavailable.contains("ntp.server")
                && result.unavailable.contains("ntp.automatic_time")
        );
    }
    #[test]
    fn persistence_requires_a_separate_preview_and_formats_are_an_explicit_item() {
        let (raw, target) = fixture();
        let normal =
            super::super::repair::plan(&target, &snapshot(&raw, &target, false, None)).unwrap();
        let allowed =
            super::super::repair::plan(&target, &snapshot(&raw, &target, true, None)).unwrap();
        assert_ne!(normal.base, allowed.base);
        let variable = |plan: &super::super::repair::Plan| {
            plan.items
                .iter()
                .find(|item| item.id == "session.TZ")
                .unwrap()
                .clone()
        };
        assert_eq!(variable(&normal).status, "unavailable");
        assert_eq!(variable(&allowed).status, "change");
        assert_eq!(variable(&allowed).scope, "user");
        let policy = allowed
            .items
            .iter()
            .position(|item| item.id == "language.independent_formats")
            .unwrap();
        let language = allowed
            .items
            .iter()
            .position(|item| item.id == "language.preferred")
            .unwrap();
        assert!(policy < language);
    }
}
