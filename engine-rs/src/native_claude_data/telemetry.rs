//! Retain explicit privacy/reporting settings without exposing their values.
use anyhow::{bail, Result};
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::fmt;

struct Fields(BTreeMap<String, Box<RawValue>>);

impl<'de> Deserialize<'de> for Fields {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct FieldVisitor;
        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = Fields;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an object with unique fields")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Fields, M::Error> {
                let mut fields = BTreeMap::new();
                while let Some((name, value)) = map.next_entry::<String, Box<RawValue>>()? {
                    if fields.insert(name, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate field"));
                    }
                }
                Ok(Fields(fields))
            }
        }
        deserializer.deserialize_map(FieldVisitor)
    }
}

fn environment_key(name: &str) -> bool {
    name.starts_with("OTEL_")
        || name.starts_with("CLAUDE_CODE_OTEL_")
        || matches!(
            name,
            "BETA_TRACING_ENDPOINT"
                | "ENABLE_BETA_TRACING_DETAILED"
                | "CLAUDE_CODE_ENABLE_TELEMETRY"
                | "CLAUDE_CODE_ENHANCED_TELEMETRY_BETA"
                | "ENABLE_ENHANCED_TELEMETRY_BETA"
                | "CLAUDE_CODE_PROPAGATE_TRACEPARENT"
                | "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"
                | "DISABLE_TELEMETRY"
                | "DISABLE_ERROR_REPORTING"
                | "DISABLE_FEEDBACK_COMMAND"
                | "DISABLE_GROWTHBOOK"
                | "DO_NOT_TRACK"
                | "CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY"
                | "CLAUDE_CODE_ENABLE_FEEDBACK_SURVEY_FOR_OTEL"
                | "CLAUDE_CODE_ENABLE_SHARE_BUNDLE"
                | "CLAUDE_CODE_SEND_FEEDBACK"
                | "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST"
                | "CLAUDE_CODE_GZIP_REQUEST_BODIES"
                | "CLAUDE_CODE_GZIP_DATADOG_LOGS"
                | "CLAUDE_CODE_CERT_STORE"
                | "HTTP_PROXY"
                | "HTTPS_PROXY"
                | "ALL_PROXY"
                | "NO_PROXY"
                | "http_proxy"
                | "https_proxy"
                | "all_proxy"
                | "no_proxy"
                | "NODE_EXTRA_CA_CERTS"
                | "SSL_CERT_FILE"
                | "SSL_CERT_DIR"
                | "CLAUDE_CODE_CLIENT_CERT"
                | "CLAUDE_CODE_CLIENT_KEY"
                | "CLAUDE_CODE_CLIENT_KEY_PASSPHRASE"
        )
}

fn settings_key(name: &str) -> bool {
    matches!(
        name,
        "cleanupPeriodDays"
            | "desktopSessionCleanupPeriodDays"
            | "feedbackDrafts"
            | "feedbackSurveyRate"
            | "skipWebFetchPreflight"
            | "otelHeadersHelper"
    )
}

fn fields(bytes: &[u8]) -> Result<Fields> {
    if bytes.len() > 8 * 1024 * 1024 {
        bail!("Configuration exceeds the maintenance read limit; no data was changed.");
    }
    serde_json::from_slice(bytes).map_err(|_| {
        anyhow::anyhow!("Configuration is not an unambiguous JSON object; no data was changed.")
    })
}

pub(super) struct Retained {
    pub bytes: Option<Vec<u8>>,
    pub fields: usize,
}

fn normalized_path(value: &str) -> String {
    let value = value.replace('\\', "/").to_lowercase();
    if let Some(path) = value.strip_prefix("//?/unc/") {
        format!("//{path}")
    } else {
        value.strip_prefix("//?/").unwrap_or(&value).to_owned()
    }
}

pub(super) fn caller_evidence(root: &str) -> Result<Vec<u8>> {
    use sha2::{Digest, Sha256};
    let mut env = BTreeMap::new();
    for (key, value) in std::env::vars_os() {
        let Some(key) = key.to_str().filter(|name| environment_key(name)) else {
            continue;
        };
        let value = value
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("A caller telemetry setting could not be inspected."))?;
        env.insert(key.to_owned(), value.to_owned());
    }
    let bytes = serde_json::to_vec(&serde_json::json!({"env":env}))?;
    retain(&bytes, root)?;
    Ok(Sha256::digest(&bytes).to_vec())
}

pub(super) fn retain(bytes: &[u8], root: &str) -> Result<Retained> {
    let original = fields(bytes)?.0;
    let mut retained: BTreeMap<String, Box<RawValue>> = BTreeMap::new();
    let mut count = 0;
    for (name, value) in &original {
        let lower = name.to_ascii_lowercase();
        if !settings_key(name)
            && (lower.contains("telemetry") || lower.contains("otel") || lower.contains("tracing"))
        {
            bail!("An unrecognized telemetry setting prevents safe cleanup.");
        }
        if settings_key(name) {
            if name == "otelHeadersHelper" && value.get() != "null" && value.get() != "\"\"" {
                bail!("Telemetry uses a headers helper with unknown file dependencies; cleanup is unavailable.");
            }
            retained.insert(name.clone(), value.clone());
            count += 1;
        } else if name == "env" {
            let env = fields(value.get().as_bytes())?.0;
            let mut keep = BTreeMap::new();
            for (key, value) in env {
                if !environment_key(&key) {
                    let known_non_telemetry = matches!(
                        key.as_str(),
                        "CLAUDE_CODE_OAUTH_TOKEN"
                            | "CLAUDE_CODE_SETUP_TOKEN"
                            | "CLAUDE_CODE_USE_BEDROCK"
                            | "CLAUDE_CODE_USE_VERTEX"
                            | "CLAUDE_CODE_USE_FOUNDRY"
                    );
                    if key.contains("TELEMETRY")
                        || key.contains("OTEL")
                        || key.contains("TRACING")
                        || key.contains("DATADOG")
                        || (!known_non_telemetry
                            && (key.starts_with("CLAUDE_CODE_") || key.starts_with("NODE_")))
                    {
                        bail!(
                            "An unclassified runtime or telemetry variable prevents safe cleanup."
                        );
                    }
                    continue;
                }
                let plain: String = serde_json::from_str(value.get()).map_err(|_| {
                    anyhow::anyhow!("A telemetry variable has an unsupported value type; cleanup is unavailable.")
                })?;
                let normalized = normalized_path(&plain);
                let normalized_root = normalized_path(root);
                if normalized.contains(&normalized_root) || normalized.contains("~/.claude") {
                    bail!("A telemetry setting may depend on files in the cleanup directory; cleanup is unavailable.");
                }
                if !plain.is_empty()
                    && (key.ends_with("_CERTIFICATE")
                        || key.ends_with("_CLIENT_KEY")
                        || matches!(
                            key.as_str(),
                            "NODE_EXTRA_CA_CERTS"
                                | "SSL_CERT_FILE"
                                | "SSL_CERT_DIR"
                                | "CLAUDE_CODE_CLIENT_CERT"
                                | "CLAUDE_CODE_CLIENT_KEY"
                        ))
                {
                    let path = std::path::Path::new(&plain);
                    let resolved = dunce::canonicalize(path).ok();
                    if !path.is_absolute()
                        || resolved.as_ref().is_none_or(|p| {
                            let path = normalized_path(&p.to_string_lossy());
                            path == normalized_root
                                || path.starts_with(&format!("{normalized_root}/"))
                        })
                    {
                        bail!("A telemetry file dependency cannot be verified outside the cleanup directory; cleanup is unavailable.");
                    }
                }
                count += 1;
                keep.insert(key, value);
            }
            if !keep.is_empty() {
                retained.insert(name.clone(), serde_json::value::to_raw_value(&keep)?);
            }
        }
    }
    let output = if retained.is_empty() {
        None
    } else if serde_json::from_slice::<serde_json::Value>(bytes)?
        == serde_json::to_value(&retained)?
    {
        Some(bytes.to_vec())
    } else {
        Some(serde_json::to_vec_pretty(&retained)?)
    };
    Ok(Retained {
        bytes: output,
        fields: count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn certificate_aliases_with_different_root_casing_are_preserved_by_refusing_cleanup() {
        let path =
            std::env::temp_dir().join(format!("am-claude-certificate-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(path.join(".CLAUDE")).unwrap();
        let path = dunce::canonicalize(&path).unwrap();
        std::fs::write(
            path.join(".CLAUDE/certificate.pem"),
            b"SYNTHETIC_CERTIFICATE",
        )
        .unwrap();
        std::os::unix::fs::symlink(path.join(".CLAUDE/certificate.pem"), path.join("alias.pem"))
            .unwrap();
        let settings = serde_json::to_vec(
            &serde_json::json!({"env":{"NODE_EXTRA_CA_CERTS":path.join("alias.pem")}}),
        )
        .unwrap();
        assert!(retain(&settings, path.join(".claude").to_str().unwrap()).is_err());
        assert_eq!(
            std::fs::read(path.join(".CLAUDE/certificate.pem")).unwrap(),
            b"SYNTHETIC_CERTIFICATE"
        );
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn shared_transport_controls_remain_and_unclassified_runtime_controls_block_cleanup() {
        let before=br#"{"env":{"CLAUDE_CODE_CERT_STORE":"system","CLAUDE_CODE_GZIP_DATADOG_LOGS":"0","CLAUDE_CODE_CLIENT_KEY_PASSPHRASE":"PRIVATE_SHARED_TELEMETRY"}}"#;
        let result = retain(before, "/YOUR_HOME/.claude").unwrap();
        assert_eq!(result.fields, 3);
        assert_eq!(result.bytes.unwrap(), before);
        assert!(retain(
            br#"{"env":{"CLAUDE_CODE_FUTURE_TRANSPORT":"private"}}"#,
            "/YOUR_HOME/.claude"
        )
        .is_err());
        assert!(retain(
            br#"{"env":{"NODE_OPTIONS":"--require private"}}"#,
            "/YOUR_HOME/.claude"
        )
        .is_err());
        assert!(retain(br#"{"env":{"CLAUDE_CODE_OAUTH_TOKEN":"PRIVATE_TOKEN","CLAUDE_CODE_SETUP_TOKEN":"PRIVATE_TOKEN","CLAUDE_CODE_USE_BEDROCK":"1"}}"#,"/YOUR_HOME/.claude").unwrap().bytes.is_none());
    }

    #[test]
    fn tracing_and_feedback_switches_retain_explicit_off_values() {
        let before=br#"{"feedbackDrafts":"off","env":{"CLAUDE_CODE_ENHANCED_TELEMETRY_BETA":"0","ENABLE_ENHANCED_TELEMETRY_BETA":"0","CLAUDE_CODE_PROPAGATE_TRACEPARENT":"0"}}"#;
        let result = retain(before, "/YOUR_HOME/.claude").unwrap();
        assert_eq!(result.fields, 4);
        assert_eq!(result.bytes.unwrap(), before);
        assert!(retain(
            br#"{"env":{"FUTURE_TELEMETRY_MODE":"off"}}"#,
            "/YOUR_HOME/.claude"
        )
        .is_err());
    }

    #[test]
    fn removes_account_and_runtime_fields_without_changing_telemetry_values() {
        let before = br#"{"oauthAccount":{"token":"PRIVATE_ACCOUNT_SENTINEL"},"userID":"old","model":"old","env":{"ANTHROPIC_API_KEY":"PRIVATE_KEY_SENTINEL","DISABLE_TELEMETRY":"0","DO_NOT_TRACK":"1","OTEL_EXPORTER_OTLP_HEADERS":"PRIVATE_TELEMETRY_SENTINEL","CLAUDE_CODE_OTEL_FLUSH_TIMEOUT_MS":"900"},"feedbackSurveyRate":0.25}"#;
        let result = retain(before, "/YOUR_HOME/.claude").unwrap();
        assert_eq!(result.fields, 5);
        let value: serde_json::Value =
            serde_json::from_slice(result.bytes.as_ref().unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"env":{"DISABLE_TELEMETRY":"0","DO_NOT_TRACK":"1","OTEL_EXPORTER_OTLP_HEADERS":"PRIVATE_TELEMETRY_SENTINEL","CLAUDE_CODE_OTEL_FLUSH_TIMEOUT_MS":"900"},"feedbackSurveyRate":0.25})
        );
        assert!(!String::from_utf8(result.bytes.unwrap())
            .unwrap()
            .contains("PRIVATE_ACCOUNT_SENTINEL"));
    }

    #[test]
    fn already_retained_configuration_is_byte_identical_and_other_data_can_be_removed() {
        let before = b"{ \"env\": { \"DISABLE_TELEMETRY\" : \"1\" } }\n";
        assert_eq!(
            retain(before, "/YOUR_HOME/.claude").unwrap().bytes.unwrap(),
            before
        );
        assert!(retain(
            br#"{"oauthAccount":{"id":"old"},"mcpServers":{}}"#,
            "/YOUR_HOME/.claude"
        )
        .unwrap()
        .bytes
        .is_none());
    }

    #[test]
    fn ambiguous_or_dependent_telemetry_blocks_without_disclosing_contents() {
        let cases: &[(&[u8], &str)] = &[
            (br#"{"env":{"DISABLE_TELEMETRY":"1","DISABLE_TELEMETRY":"0"}}"#, "/YOUR_HOME/.claude"),
            (br#"{"env":{},"env":{}}"#, "/YOUR_HOME/.claude"),
            (br#"{"env":"PRIVATE_INVALID_SENTINEL"}"#, "/YOUR_HOME/.claude"),
            (br#"{"otelHeadersHelper":"PRIVATE_HELPER_SENTINEL"}"#, "/YOUR_HOME/.claude"),
            (br#"{"env":{"OTEL_EXPORTER_OTLP_CERTIFICATE":"/YOUR_HOME/.claude/private.pem"}}"#, "/YOUR_HOME/.claude"),
            (br#"{"env":{"OTEL_EXPORTER_OTLP_CERTIFICATE":"C:\\YOUR_HOME\\.claude\\private.pem"}}"#, "C:\\YOUR_HOME\\.claude"),
        ];
        for (bytes, root) in cases {
            let error = retain(bytes, root).err().unwrap().to_string();
            assert!(!error.contains("PRIVATE_"));
        }
    }
}
