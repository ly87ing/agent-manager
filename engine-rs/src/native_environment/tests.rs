use super::*;

#[test]
fn region_parsing_separates_encoding_script_and_modifiers() {
    for (locale, expected) in [
        ("en_US.UTF-8", Some("US")),
        ("sr_RS.UTF-8@latin", Some("RS")),
        ("zh_Hant_TW@calendar=chinese", Some("TW")),
        ("en-GB", Some("GB")),
        ("C.UTF-8", None),
    ] {
        assert_eq!(region(locale).as_deref(), expected);
    }
}

#[test]
fn clock_tolerance_keeps_overlapping_uncertainty_inconclusive() {
    assert_eq!(clock_state(5.0, 2.0, 10), "match");
    assert_eq!(clock_state(-15.0, 2.0, 10), "mismatch");
    assert_eq!(clock_state(10.0, 2.0, 10), "unknown");
    assert_eq!(clock_state(0.0, 20.0, 10), "unknown");
}

#[test]
fn kernel_sync_is_independent_of_running_service_and_reference_accuracy() {
    let target = Target {
        time_synchronized: Some(true),
        ..Target::default()
    };
    let mut actual = observation();
    actual.time_service = Some(true);
    actual.automatic_time = Some(true);
    for (value, expected_state) in [
        (None, "unknown"),
        (Some(false), "mismatch"),
        (Some(true), "match"),
    ] {
        actual.time_synchronized = value;
        let rows = evaluate(&actual, &target);
        let sync = rows
            .iter()
            .find(|row| row.id == "ntp.synchronized")
            .unwrap();
        assert_eq!(sync.state, expected_state);
        assert_eq!(sync.expected.as_deref(), Some("synchronized"));
        assert_eq!(
            rows.iter()
                .find(|row| row.id == "ntp.kernel_max_error")
                .unwrap()
                .state,
            "unknown"
        );
    }
    assert_eq!(
        evaluate(&actual, &Target::default())
            .iter()
            .find(|row| row.id == "ntp.synchronized")
            .unwrap()
            .state,
        "observed"
    );
}

#[test]
fn strict_locale_reveals_overridden_conflicts_and_allows_unset_variables() {
    let target = Target {
        locale: Some("en_GB.UTF-8".into()),
        strict_locale: true,
        ..Target::default()
    };
    let mut actual = observation();
    let row = |rows: Vec<Check>| {
        rows.into_iter()
            .find(|row| row.id == "process.raw_locale")
            .unwrap()
    };
    assert_eq!(row(evaluate(&actual, &target)).state, "match");
    actual.env.insert("LC_ALL".into(), "en_GB.UTF-8".into());
    actual.env.insert("LANG".into(), "fr_FR.UTF-8".into());
    let rows = evaluate(&actual, &target);
    assert!(rows
        .iter()
        .filter(|row| row.id.starts_with("LC_"))
        .all(|row| row.state == "match"));
    let strict = row(rows);
    assert_eq!(strict.state, "mismatch");
    assert!(strict.observed.unwrap().contains("fr_FR.UTF-8"));
}

fn observation() -> Observation {
    Observation {
        timezone: Some("Europe/London".into()),
        languages: Some(vec!["en-GB".into(), "fr-FR".into()]),
        apple_locale: Some("en_GB".into()),
        ntp_server: Some("example.org".into()),
        env: [("LANG".into(), "en_GB.UTF-8".into())].into(),
        ..Observation::default()
    }
}

#[test]
fn inherits_timezone_and_resolves_locale_categories_without_requiring_lc_all() {
    let target = Target {
        timezone: Some("Europe/London".into()),
        locale: Some("en_GB.utf8".into()),
        ..Target::default()
    };
    let rows = evaluate(&observation(), &target);
    for row in rows.iter().filter(|row| row.expected.is_some()) {
        assert_eq!(row.state, "match", "{}", row.id);
    }
    let mut changed = observation();
    changed.env.insert("LC_TIME".into(), "de_DE.UTF-8".into());
    let rows = evaluate(&changed, &target);
    assert_eq!(
        rows.iter().find(|row| row.id == "LC_TIME").unwrap().state,
        "mismatch"
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.id == "LC_NUMERIC")
            .unwrap()
            .state,
        "match"
    );
    changed.env.insert("LC_ALL".into(), "en_GB.UTF-8".into());
    assert!(evaluate(&changed, &target)
        .iter()
        .filter(|row| row.id.starts_with("LC_"))
        .all(|row| row.state == "match"));
}

#[test]
fn keeps_system_process_language_order_and_missing_evidence_distinct() {
    let mut actual = observation();
    actual.env.insert("TZ".into(), "Asia/Tokyo".into());
    let target = Target {
        timezone: Some("Europe/London".into()),
        languages: vec!["fr-FR".into(), "en-GB".into()],
        automatic_time: Some(true),
        ..Target::default()
    };
    let rows = evaluate(&actual, &target);
    let state = |id| rows.iter().find(|row| row.id == id).unwrap().state;
    assert_eq!(state("time.timezone"), "match");
    assert_eq!(state("process.timezone"), "mismatch");
    assert_eq!(state("language.preferred"), "mismatch");
    assert_eq!(state("ntp.automatic_time"), "unknown");
    assert_eq!(state("language.region"), "observed");
}

#[test]
fn validates_inputs_before_any_reads_and_never_echoes_unknown_fields() {
    for input in [
        json!({"timezone":"../../etc"}),
        json!({"ntp_server":"https://example.org"}),
        json!({"region":"USA"}),
        json!({"password":"plain-private-content"}),
    ] {
        let request = Request {
            command: "environment:check".into(),
            args: vec!["--spec".into(), input.to_string()],
            run_id: "invalid".into(),
        };
        let result = execute(
            &Context {
                home_dir: std::env::temp_dir().join("environment-invalid-home"),
                data_dir: std::env::temp_dir().join("environment-invalid-data"),
            },
            &request,
        );
        assert!(!result.ok);
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("plain-private-content"));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn configured_server_rejects_ambiguous_files_and_ignores_comments() {
    assert_eq!(
        macos::configured_server("# example\nserver EXAMPLE.ORG. # primary\n"),
        Some("example.org".into())
    );
    assert!(macos::configured_server("server one.org\nserver two.org").is_none());
}
