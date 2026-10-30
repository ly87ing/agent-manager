use super::{Brand, Sample};
use crate::native_environment::{check, Check, Target};
use std::collections::BTreeMap;

fn brands(values: &[Brand]) -> String {
    let mut pairs: Vec<_> = values
        .iter()
        .map(|b| (b.brand.as_str(), b.version.as_str()))
        .collect();
    pairs.sort_unstable();
    serde_json::to_string(&pairs).unwrap()
}

fn header_brands(value: &str) -> Option<String> {
    // The browser's structured brand list can contain GREASE brands and reorder entries.
    let re = regex::Regex::new(r#""([^"\\]+)"\s*;\s*v="([^"\\]+)""#).ok()?;
    let mut values = Vec::new();
    for entry in value.split(',') {
        let entry = entry.trim();
        let found = re.captures(entry)?;
        if found.get(0)?.as_str() != entry {
            return None;
        }
        values.push(Brand {
            brand: found[1].into(),
            version: found[2].into(),
        });
    }
    if values.is_empty() {
        None
    } else {
        Some(brands(&values))
    }
}

fn primary_language(value: &str) -> Option<String> {
    let mut best = None;
    let mut best_quality = 0.0;
    for entry in value.split(',') {
        let mut fields = entry.split(';');
        let language = fields.next()?.trim();
        if language.is_empty()
            || !language
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'*')
        {
            return None;
        }
        let quality = match fields.next() {
            None => 1.0,
            Some(value) => value.trim().strip_prefix("q=")?.parse::<f64>().ok()?,
        };
        if fields.next().is_some() || !quality.is_finite() || !(0.0..=1.0).contains(&quality) {
            return None;
        }
        if quality > best_quality {
            best = Some(language.to_ascii_lowercase());
            best_quality = quality;
        }
    }
    best.filter(|s| s != "*")
}

pub(super) fn evaluate(
    target: &Target,
    sample: Option<&Sample>,
    headers: &BTreeMap<String, String>,
) -> Vec<Check> {
    let row =
        |id, actual, expected, source, note| check(id, "browser", actual, expected, source, note);
    let mut rows = vec![
        row(
            "browser.timezone",
            sample.and_then(|s| s.timezone.clone()),
            target.timezone.clone(),
            "Chrome Intl.DateTimeFormat",
            "profile_timezone_exact",
        ),
        row(
            "browser.languages",
            sample.map(|s| s.languages.join(", ")),
            (!target.languages.is_empty()).then(|| target.languages.join(", ")),
            "Chrome navigator.languages",
            "profile_ordered_languages",
        ),
        row(
            "browser.intl_locale",
            sample.and_then(|s| s.intl_locale.clone()),
            None,
            "Chrome Intl.DateTimeFormat",
            "browser_inventory",
        ),
        row(
            "browser.screen",
            sample.map(|s| serde_json::to_string(&s.screen).unwrap()),
            None,
            "Chrome screen, window, visualViewport",
            "css_pixels_and_scale_inventory",
        ),
        row(
            "browser.client_hints",
            sample
                .and_then(|s| s.client_hints.as_ref())
                .map(|s| serde_json::to_string(s).unwrap()),
            None,
            "Chrome navigator.userAgentData",
            "browser_inventory",
        ),
    ];
    if let Some(actual) = sample {
        if !target.languages.is_empty()
            && actual
                .languages
                .iter()
                .map(|s| s.to_ascii_lowercase())
                .eq(target.languages.iter().map(|s| s.to_ascii_lowercase()))
        {
            rows[1].state = "match";
        }
    }
    let hints = sample.and_then(|s| s.client_hints.as_ref());
    rows.push(row(
        "browser.platform_native",
        hints.map(|s| s.platform.clone()).filter(|s| !s.is_empty()),
        Some("macOS".into()),
        "native macOS / Chrome navigator.userAgentData",
        "native_platform_comparison",
    ));
    let ua_major = sample.and_then(|s| {
        regex::Regex::new(r"(?:Chrome|HeadlessChrome)/(\d+)\.")
            .ok()?
            .captures(&s.user_agent)
            .map(|v| v[1].to_string())
    });
    let hint_major = hints
        .and_then(|s| {
            s.brands
                .iter()
                .find(|b| b.brand == "Google Chrome" || b.brand == "Chromium")
        })
        .map(|b| b.version.clone());
    let ua_major = if hint_major.as_ref().is_some_and(|s| !s.is_empty()) {
        ua_major
    } else {
        None
    };
    rows.push(row(
        "browser.ua_version",
        ua_major,
        Some(hint_major.unwrap_or_else(|| "available Chrome major version".into())),
        "Chrome User-Agent / UA-CH major version",
        "reduced_user_agent_major_only",
    ));
    let header = |key: &str| headers.get(key).cloned();
    let quoted = |key: &str| header(key).and_then(|s| serde_json::from_str::<String>(&s).ok());
    let expected = |value: Option<String>| {
        Some(value.unwrap_or_else(|| "available JavaScript evidence".into()))
    };
    rows.push(row(
        "browser.ua_http",
        header("user-agent"),
        expected(sample.map(|s| s.user_agent.clone())),
        "same-origin HTTP User-Agent / Chrome navigator.userAgent",
        "same_context_comparison",
    ));
    let primary = header("accept-language").and_then(|s| primary_language(&s));
    rows.push(row(
        "browser.language_http",
        primary,
        expected(
            sample
                .and_then(|s| s.languages.first())
                .map(|s| s.to_ascii_lowercase()),
        ),
        "same-origin HTTP Accept-Language / Chrome navigator.languages",
        "primary_language_only",
    ));
    rows.push(row(
        "browser.ch_platform",
        hints
            .filter(|h| !h.platform.is_empty())
            .and_then(|_| quoted("sec-ch-ua-platform")),
        expected(hints.map(|s| s.platform.clone())),
        "same-origin HTTP Sec-CH-UA-Platform / JavaScript",
        "same_context_comparison",
    ));
    rows.push(row(
        "browser.ch_brands",
        hints
            .filter(|h| !h.brands.is_empty())
            .and_then(|_| header("sec-ch-ua"))
            .and_then(|s| header_brands(&s)),
        expected(hints.map(|s| brands(&s.brands))),
        "same-origin HTTP Sec-CH-UA / JavaScript",
        "brand_order_ignored",
    ));
    rows.push(row(
        "browser.ch_mobile",
        header("sec-ch-ua-mobile").and_then(|s| match s.as_str() {
            "?0" => Some("false".into()),
            "?1" => Some("true".into()),
            _ => None,
        }),
        expected(hints.map(|s| s.mobile.to_string())),
        "same-origin HTTP Sec-CH-UA-Mobile / JavaScript",
        "same_context_comparison",
    ));
    for (id, key, value) in [
        (
            "browser.ch_architecture",
            "sec-ch-ua-arch",
            hints.and_then(|s| s.architecture.clone()),
        ),
        (
            "browser.ch_bitness",
            "sec-ch-ua-bitness",
            hints.and_then(|s| s.bitness.clone()),
        ),
        (
            "browser.ch_platform_version",
            "sec-ch-ua-platform-version",
            hints.and_then(|s| s.platform_version.clone()),
        ),
    ] {
        let value = value.filter(|s| !s.is_empty());
        let actual = if value.is_some() {
            quoted(key).filter(|s| !s.is_empty())
        } else {
            None
        };
        rows.push(row(
            id,
            actual,
            expected(value),
            "same-origin HTTP high-entropy client hints / JavaScript",
            "restricted_evidence_stays_unknown",
        ));
    }
    rows.push(row(
        "browser.ch_full_versions",
        hints
            .and_then(|h| h.full_version_list.as_ref())
            .filter(|list| !list.is_empty())
            .and_then(|_| header("sec-ch-ua-full-version-list"))
            .and_then(|s| header_brands(&s)),
        expected(
            hints
                .and_then(|s| s.full_version_list.as_ref())
                .map(|s| brands(s)),
        ),
        "same-origin HTTP Sec-CH-UA-Full-Version-List / JavaScript",
        "brand_order_ignored",
    ));
    // Missing JavaScript must not turn a present HTTP value into a mismatch or a match.
    if sample.is_none() {
        for row in &mut rows {
            row.observed = None;
            row.state = "unknown";
        }
    } else if hints.is_none() {
        for row in rows.iter_mut().filter(|r| r.id.starts_with("browser.ch_")) {
            row.observed = None;
            row.state = "unknown";
        }
    }
    rows
}
