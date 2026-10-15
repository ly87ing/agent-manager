use super::Target;
use crate::{Context, Request};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Item {
    pub id: String,
    pub scope: String,
    pub before: Option<Value>,
    pub expected: Value,
    pub status: String,
}

#[derive(Clone, Default)]
pub(super) struct Snapshot {
    pub values: BTreeMap<String, Option<Value>>,
    pub session_scope: String,
    pub session_available: bool,
}

#[derive(Serialize)]
pub(super) struct Plan {
    pub base: String,
    pub target: Target,
    pub items: Vec<Item>,
    pub session_scope: String,
}

fn region_locale(locale: &str, region: &str) -> Option<String> {
    let current = super::region(locale)?;
    let prefix = locale.split('@').next()?;
    let start = prefix
        .match_indices(&current)
        .find(|(index, _)| {
            *index > 0
                && matches!(prefix.as_bytes()[index - 1], b'_' | b'-')
                && (index + 2 == prefix.len()
                    || matches!(prefix.as_bytes()[index + 2], b'_' | b'-'))
        })?
        .0;
    let mut result = locale.to_string();
    result.replace_range(start..start + 2, region);
    Some(result)
}

pub(super) fn plan(target: &Target, snapshot: &Snapshot) -> Result<Plan> {
    let mut items = Vec::new();
    let mut add = |id: &str, scope: &str, expected: Value| {
        let before = snapshot.values.get(id).cloned().flatten();
        let status = if before.as_ref() == Some(&expected) {
            "unchanged"
        } else {
            "change"
        };
        items.push(Item {
            id: id.into(),
            scope: scope.into(),
            before,
            expected,
            status: status.into(),
        });
    };
    if target.automatic_timezone == Some(false) {
        add("time.automatic_timezone", "system", json!(false));
    }
    if let Some(zone) = &target.timezone {
        add("time.timezone", "system", json!(zone));
    }
    if target.automatic_timezone == Some(true) {
        add("time.automatic_timezone", "system", json!(true));
    }
    if let Some(server) = &target.ntp_server {
        add(
            "ntp.server",
            "system",
            json!(server.trim_end_matches('.').to_ascii_lowercase()),
        );
    }
    if let Some(automatic) = target.automatic_time {
        add("ntp.automatic_time", "system", json!(automatic));
    }
    if !target.languages.is_empty() {
        add("language.preferred", "user", json!(target.languages));
    }
    if let Some(region) = &target.region {
        let locale = snapshot
            .values
            .get("language.region")
            .and_then(Option::as_ref)
            .and_then(Value::as_str)
            .and_then(|locale| region_locale(locale, region));
        add(
            "language.region",
            "user",
            locale
                .as_ref()
                .map_or_else(|| json!(region), |locale| json!(locale)),
        );
        if locale.is_none() {
            items.last_mut().unwrap().status = "unavailable".into();
        }
    }
    if let Some(zone) = &target.timezone {
        add_session(&mut items, snapshot, "TZ", zone);
    }
    if let Some(locale) = &target.locale {
        for key in ["LANG", "LC_ALL"]
            .iter()
            .chain(super::LOCALE_CATEGORIES.iter())
        {
            add_session(&mut items, snapshot, key, locale);
        }
    }
    if items.is_empty() {
        bail!("This profile has no automatically repairable settings.");
    }
    let base = crate::native_mcp::model::sha256_hex(&serde_json::to_string(
        &json!({"target":target, "items":items, "session_scope":snapshot.session_scope}),
    )?);
    Ok(Plan {
        base,
        target: target.clone(),
        items,
        session_scope: snapshot.session_scope.clone(),
    })
}

fn add_session(items: &mut Vec<Item>, snapshot: &Snapshot, key: &str, value: &str) {
    let id = format!("session.{key}");
    let before = snapshot.values.get(&id).cloned().flatten();
    let status = if !snapshot.session_available || before.is_none() {
        "unavailable"
    } else if before.as_ref() == Some(&json!(value)) {
        "unchanged"
    } else {
        "change"
    };
    items.push(Item {
        id,
        scope: "session".into(),
        before,
        expected: json!(value),
        status: status.into(),
    });
}

pub(super) trait Settings {
    fn snapshot(&mut self, target: &Target) -> Result<Snapshot>;
    fn system(&mut self, items: &[Item]) -> Result<(String, Vec<Item>, Option<bool>)>;
    fn write_user(&mut self, item: &Item) -> bool;
}

pub(super) fn apply(
    settings: &mut impl Settings,
    target: &Target,
    base: &str,
    mut audit: impl FnMut(&str, &[Item]) -> Result<()>,
) -> Result<(String, Vec<Item>, Option<bool>)> {
    let current = plan(target, &settings.snapshot(target)?)?;
    if current.base != base {
        bail!("Environment settings changed after preview. Review a new repair preview.");
    }
    let mut items = current.items;
    if items.iter().all(|item| item.status != "change") {
        let status = if items.iter().all(|item| item.status == "unchanged") {
            "not_needed"
        } else {
            "partial"
        };
        return Ok((status.into(), items, None));
    }
    audit("requested", &items)?;
    let system: Vec<_> = items
        .iter()
        .filter(|item| item.scope == "system" && item.status == "change")
        .cloned()
        .collect();
    let mut automatic = None;
    if !system.is_empty() {
        let (status, results, observed) = settings.system(&system)?;
        automatic = observed;
        for item in &mut items {
            if let Some(result) = results.iter().find(|result| result.id == item.id) {
                *item = result.clone();
            }
        }
        if status != "applied" {
            for item in &mut items {
                if item.status == "change" {
                    item.status = "not_applied".into();
                }
            }
            let status = if audit(&status, &items).is_ok() {
                status
            } else {
                "audit_failed".into()
            };
            return Ok((status, items, automatic));
        }
    }
    for item in items
        .iter_mut()
        .filter(|item| item.scope != "system" && item.status == "change")
    {
        let Ok(snapshot) = settings.snapshot(target) else {
            item.status = "outcome_unknown".into();
            break;
        };
        let actual = snapshot.values.get(&item.id).cloned().flatten();
        if actual.as_ref() == Some(&item.expected) {
            item.status = "unchanged".into();
            continue;
        }
        if actual != item.before {
            item.status = "stale".into();
            break;
        }
        if item.scope == "user" && actual.is_none() {
            item.status = "unavailable".into();
            continue;
        }
        let command_ok = settings.write_user(item);
        let after = settings
            .snapshot(target)
            .ok()
            .and_then(|snapshot| snapshot.values.get(&item.id).cloned().flatten());
        item.status = match (command_ok, after) {
            (true, Some(value)) if value == item.expected => "verified",
            (false, Some(value)) if value == item.expected => "verified_after_error",
            (_, None) => "outcome_unknown",
            (true, _) => "unverified",
            (false, _) => "failed",
        }
        .into();
        if item.status != "verified" {
            break;
        }
    }
    for item in &mut items {
        if item.status == "change" {
            item.status = "not_applied".into();
        }
    }
    let status = if items
        .iter()
        .all(|item| matches!(item.status.as_str(), "unchanged" | "verified"))
    {
        "applied"
    } else {
        "partial"
    };
    // A completion-log failure cannot erase settings already changed.
    let status = if audit(status, &items).is_ok() {
        status
    } else {
        "audit_failed"
    };
    Ok((status.into(), items, automatic))
}

fn arguments(req: &Request) -> Result<(Target, Option<String>)> {
    let preview = req.command.replace('.', ":") == "environment:repair-preview";
    let mut spec = None;
    let mut base = None;
    let mut confirm = false;
    let mut args = req.args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--spec" if spec.is_none() => {
                spec = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("Missing repair target."))?,
                )
            }
            "--base" if !preview && base.is_none() => {
                base = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("Missing repair preview."))?
                        .clone(),
                )
            }
            "--confirm" if !preview && !confirm => confirm = true,
            _ => bail!("Unsupported or duplicate repair flag."),
        }
    }
    let spec = spec.ok_or_else(|| anyhow::anyhow!("Choose an explicit environment profile."))?;
    if spec.len() > 8192 {
        bail!("Environment target is too large.");
    }
    let target: Target = serde_json::from_str(spec)
        .map_err(|_| anyhow::anyhow!("Invalid environment target fields."))?;
    target.validate()?;
    if !preview
        && (!confirm
            || !base.as_ref().is_some_and(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            }))
    {
        bail!("Repair requires --confirm and the current --base preview fingerprint.");
    }
    Ok((target, base))
}

pub(super) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let (target, base) = arguments(req)?;
    #[cfg(target_os = "macos")]
    {
        super::repair_macos::validate_runtime_locale(&target)?;
        let mut settings = super::repair_macos::MacSettings;
        if base.is_none() {
            return Ok(serde_json::to_value(plan(
                &target,
                &settings.snapshot(&target)?,
            )?)?);
        }
        let guard = super::repair_macos::mutation_lock()?;
        let (status, items, _automatic) = apply(
            &mut settings,
            &target,
            base.as_deref().unwrap(),
            |status, items| {
                let conn = crate::store::open_write(ctx)?;
                let detail = json!({"status":status, "items":items.iter().map(|item| json!({"id":item.id,"status":item.status})).collect::<Vec<_>>()});
                crate::store::record_operation(
                    &conn,
                    Some(&req.run_id),
                    "environment:repair",
                    Some("macos-environment"),
                    Some(&detail),
                )
            },
        )?;
        let mut clock_error = None;
        let clock_report = if matches!(status.as_str(), "applied" | "not_needed")
            && target.ntp_server.is_some()
            && target.max_clock_offset_ms.is_some()
        {
            match super::time_repair::handle_locked(ctx, req, &target, &guard) {
                Ok(report) => Some(report),
                Err(error) => {
                    clock_error = Some(error.to_string());
                    None
                }
            }
        } else {
            None
        };
        let probe = target.ntp_server.is_some();
        if probe {
            super::time_probe::wait_for_interval();
        }
        let authorized = if matches!(status.as_str(), "applied" | "not_needed")
            && target.automatic_time.is_some()
        {
            super::actions::check_time_settings(&target, probe)
                .ok()
                .filter(|value| value["status"] == "read")
                .map(|value| value["report"].clone())
        } else {
            None
        };
        let mut report = authorized.or_else(|| {
            if probe {
                super::time_probe::wait_for_interval();
            }
            super::report(&target, probe).ok()
        });
        if let (Some(clock), Some(report)) = (clock_report, report.as_mut()) {
            report["time_repair"] = clock["time_repair"].clone();
        }
        drop(guard);
        let restart_required = items.iter().any(|item| {
            matches!(item.scope.as_str(), "session" | "user")
                && matches!(item.status.as_str(), "verified" | "verified_after_error")
        }) || report.as_ref().is_some_and(|report| {
            report["checks"].as_array().is_some_and(|checks| {
                checks
                    .iter()
                    .any(|row| row["group"] == "process" && row["state"] == "mismatch")
            })
        });
        Ok(
            json!({"status":status,"items":items,"report":report,"restart_required":restart_required,"clock_error":clock_error}),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (ctx, target, base);
        bail!("Environment repair currently supports macOS only.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn region_repair_preserves_language_script_and_extensions() {
        assert_eq!(
            region_locale("zh_Hant_TW@calendar=chinese", "SG").as_deref(),
            Some("zh_Hant_SG@calendar=chinese")
        );
        assert_eq!(region_locale("en-US", "SG").as_deref(), Some("en-SG"));
        assert!(region_locale("unreadable", "US").is_none());
    }
    struct Fake {
        snapshot: Snapshot,
        writes: usize,
        cancel: bool,
        fails: bool,
    }
    impl Settings for Fake {
        fn snapshot(&mut self, _: &Target) -> Result<Snapshot> {
            Ok(self.snapshot.clone())
        }
        fn system(&mut self, items: &[Item]) -> Result<(String, Vec<Item>, Option<bool>)> {
            if self.cancel {
                return Ok(("cancelled".into(), vec![], None));
            }
            let mut items = items.to_vec();
            for item in &mut items {
                self.writes += 1;
                self.snapshot
                    .values
                    .insert(item.id.clone(), Some(item.expected.clone()));
                item.status = "verified".into();
            }
            Ok(("applied".into(), items, None))
        }
        fn write_user(&mut self, item: &Item) -> bool {
            self.writes += 1;
            if !self.fails {
                self.snapshot
                    .values
                    .insert(item.id.clone(), Some(item.expected.clone()));
            }
            !self.fails
        }
    }
    fn fake() -> Fake {
        let values = ["TZ", "LANG", "LC_ALL"]
            .iter()
            .chain(super::super::LOCALE_CATEGORIES.iter())
            .map(|key| (format!("session.{key}"), Some(json!(""))))
            .collect();
        Fake {
            snapshot: Snapshot {
                values,
                session_scope: "uid:1000 launchd:10".into(),
                session_available: true,
                ..Snapshot::default()
            },
            writes: 0,
            cancel: false,
            fails: false,
        }
    }
    #[test]
    fn stale_preview_and_failed_audit_stop_before_any_write() {
        let target: Target = serde_json::from_value(json!({"locale":"en_US.UTF-8"})).unwrap();
        let mut fake = fake();
        let base = plan(&target, &fake.snapshot).unwrap().base;
        fake.snapshot
            .values
            .insert("session.LANG".into(), Some(json!("C")));
        assert!(apply(&mut fake, &target, &base, |_, _| panic!(
            "No audit for stale preview"
        ))
        .is_err());
        let base = plan(&target, &fake.snapshot).unwrap().base;
        assert!(apply(&mut fake, &target, &base, |_, _| bail!("audit unavailable")).is_err());
        assert_eq!(fake.writes, 0);
    }
    #[test]
    fn cancellation_preserves_user_settings_and_locale_repairs_are_idempotent() {
        let target: Target = serde_json::from_value(
            json!({"timezone":"America/Los_Angeles","locale":"en_US.UTF-8"}),
        )
        .unwrap();
        let mut fake = fake();
        fake.cancel = true;
        let base = plan(&target, &fake.snapshot).unwrap().base;
        assert_eq!(
            apply(&mut fake, &target, &base, |_, _| Ok(())).unwrap().0,
            "cancelled"
        );
        assert_eq!(fake.writes, 0);
        fake.cancel = false;
        let result = apply(&mut fake, &target, &base, |_, _| Ok(())).unwrap();
        assert_eq!(result.0, "applied");
        assert_eq!(fake.writes, 10);
        let base = plan(&target, &fake.snapshot).unwrap().base;
        assert_eq!(
            apply(&mut fake, &target, &base, |_, _| panic!(
                "No audit for no-op"
            ))
            .unwrap()
            .0,
            "not_needed"
        );
        assert_eq!(fake.writes, 10);
    }
    #[test]
    fn unreadable_session_values_are_not_overwritten_and_contexts_bind_the_preview() {
        let target: Target = serde_json::from_value(json!({"locale":"en_US.UTF-8"})).unwrap();
        let mut fake = fake();
        for value in fake.snapshot.values.values_mut() {
            *value = Some(json!("en_US.UTF-8"));
        }
        fake.snapshot.values.insert("session.LC_CTYPE".into(), None);
        let original = plan(&target, &fake.snapshot).unwrap();
        assert_eq!(
            original
                .items
                .iter()
                .find(|item| item.id == "session.LC_CTYPE")
                .unwrap()
                .status,
            "unavailable"
        );
        assert_eq!(
            apply(&mut fake, &target, &original.base, |_, _| panic!(
                "No audit without writable changes"
            ))
            .unwrap()
            .0,
            "partial"
        );
        assert_eq!(fake.writes, 0);
        fake.snapshot.session_scope = "gui/1000 login/another launchd Aqua".into();
        assert!(apply(&mut fake, &target, &original.base, |_, _| panic!(
            "No audit for changed context"
        ))
        .is_err());
        assert_eq!(fake.writes, 0);
    }
    #[test]
    fn partial_failure_stops_remaining_writes_and_preserves_receipts() {
        let target: Target = serde_json::from_value(json!({"locale":"en_US.UTF-8"})).unwrap();
        let mut fake = fake();
        fake.fails = true;
        fake.snapshot
            .values
            .insert("session.LANG".into(), Some(json!("C")));
        let base = plan(&target, &fake.snapshot).unwrap().base;
        let result = apply(&mut fake, &target, &base, |_, _| Ok(())).unwrap();
        assert_eq!(result.0, "partial");
        assert_eq!(fake.writes, 1);
        assert_eq!(result.1[0].status, "failed");
        assert!(result.1[1..]
            .iter()
            .all(|item| item.status == "not_applied"));
    }
}
