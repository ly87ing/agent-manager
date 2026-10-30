//! Preserve opaque JSON values; ambiguous objects never become cleanup input.
use anyhow::{bail, Result};
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;
use std::collections::BTreeSet;

pub(super) struct Object<'a>(pub Vec<(String, &'a RawValue)>);

impl<'de> Deserialize<'de> for Object<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Unique;
        impl<'de> Visitor<'de> for Unique {
            type Value = Object<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object with unique field names")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut seen = BTreeSet::new();
                let mut fields = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, &'de RawValue>()? {
                    if !seen.insert(key.clone()) {
                        return Err(serde::de::Error::custom(
                            "Duplicate field; inspection is unavailable.",
                        ));
                    }
                    fields.push((key, value));
                }
                Ok(Object(fields))
            }
        }
        deserializer.deserialize_map(Unique)
    }
}

pub(super) fn environment(bytes: &[u8]) -> Result<Option<(&RawValue, Object<'_>)>> {
    let content = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let root: Object = serde_json::from_slice(content).map_err(|_| {
        anyhow::anyhow!("Settings must contain a valid JSON object with unique fields.")
    })?;
    root.0
        .into_iter()
        .find(|(key, _)| key == "env")
        .map(|(_, raw)| {
            let env: Object = serde_json::from_str(raw.get()).map_err(|_| {
                anyhow::anyhow!("The env field must be an object with unique fields.")
            })?;
            Ok((raw, env))
        })
        .transpose()
}

pub(super) fn root_fields(bytes: &[u8]) -> Result<Vec<String>> {
    let content = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let root: Object = serde_json::from_slice(content).map_err(|_| {
        anyhow::anyhow!("Settings must contain a valid JSON object with unique fields.")
    })?;
    Ok(root
        .0
        .into_iter()
        .filter(|(key, _)| matches!(key.as_str(), "apiKeyHelper" | "PROXY_MANAGED"))
        .map(|(key, _)| key)
        .collect())
}

pub(super) fn remove_root(bytes: &[u8], key: &str) -> Result<Vec<u8>> {
    if !matches!(key, "apiKeyHelper" | "PROXY_MANAGED") {
        bail!("Unknown root connection field.");
    }
    let content = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let raw: &RawValue =
        serde_json::from_slice(content).map_err(|_| anyhow::anyhow!("Invalid settings JSON."))?;
    let fields: Object =
        serde_json::from_str(raw.get()).map_err(|_| anyhow::anyhow!("Ambiguous settings JSON."))?;
    if !fields.0.iter().any(|(name, _)| name == key) {
        bail!("Selected field is absent.");
    }
    let mut replacement = vec![b'{'];
    for (name, value) in fields.0.iter().filter(|(name, _)| name != key) {
        if replacement.len() > 1 {
            replacement.push(b',');
        }
        replacement.extend(serde_json::to_vec(name)?);
        replacement.push(b':');
        replacement.extend(value.get().as_bytes());
    }
    replacement.push(b'}');
    let start = raw.get().as_ptr() as usize - bytes.as_ptr() as usize;
    let mut output = bytes[..start].to_vec();
    output.extend(replacement);
    output.extend(&bytes[start + raw.get().len()..]);
    root_fields(&output)?;
    Ok(output)
}

pub(super) fn remove(bytes: &[u8], key: &str) -> Result<Vec<u8>> {
    let (raw, fields) = environment(bytes)?
        .ok_or_else(|| anyhow::anyhow!("Selected field is absent; request a new preview."))?;
    if !fields.0.iter().any(|(name, _)| name == key) {
        bail!("Selected field is absent; request a new preview.");
    }
    let mut replacement = vec![b'{'];
    for (name, value) in fields.0.iter().filter(|(name, _)| name != key) {
        if replacement.len() > 1 {
            replacement.push(b',');
        }
        replacement.extend(serde_json::to_vec(name)?);
        replacement.push(b':');
        replacement.extend(value.get().as_bytes());
    }
    replacement.push(b'}');
    let start = raw.get().as_ptr() as usize - bytes.as_ptr() as usize;
    let mut output = bytes[..start].to_vec();
    output.extend(replacement);
    output.extend(&bytes[start + raw.get().len()..]);
    // Reparse the changed container; unknown surrounding values remain byte-identical.
    let (_, after) = environment(&output)?.unwrap();
    let expected: Vec<_> = fields
        .0
        .iter()
        .filter(|(name, _)| name != key)
        .map(|(name, value)| (name.as_str(), value.get()))
        .collect();
    let actual: Vec<_> = after
        .0
        .iter()
        .map(|(name, value)| (name.as_str(), value.get()))
        .collect();
    if actual != expected {
        bail!("Unselected fields could not be preserved.");
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_json_and_preserves_unknown_values_and_bom() {
        for bytes in [
            br#"{"env":{},"env":{}}"#.as_slice(),
            br#"{"env":{"K":1,"\u004b":2}}"#,
            br#"{"env":null}"#,
            br#"{"env":{"K":1,}}"#,
        ] {
            assert!(environment(bytes).is_err());
        }
        let before = b"\xef\xbb\xbf{\n \"model\":\"YOUR_MODEL\", \"env\": {\"KEEP\":123456789012345678901234567890,\"ANTHROPIC_API_KEY\":\"YOUR_KEY\",\"NESTED\": { \"v\": 1 }}, \"mcpServers\":{}\n}\n";
        let after = remove(before, "ANTHROPIC_API_KEY").unwrap();
        assert!(after.starts_with(b"\xef\xbb\xbf{\n \"model\":\"YOUR_MODEL\", \"env\": "));
        assert!(after.ends_with(b", \"mcpServers\":{}\n}\n"));
        assert!(std::str::from_utf8(&after[3..])
            .unwrap()
            .contains("123456789012345678901234567890"));
        assert!(std::str::from_utf8(&after[3..])
            .unwrap()
            .contains("{ \"v\": 1 }"));
        assert!(remove(&after, "ANTHROPIC_API_KEY").is_err());
    }
}
