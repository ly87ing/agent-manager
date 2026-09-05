//! Python-compatible canonical JSON (json.dumps(sort_keys=True)) + sha256
//! helpers and contract-style JSON equality (tests/contract_validator.py
//! semantics: booleans never equal numbers, 1 equals 1.0).

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Serialize a string exactly like Python's json.dumps(ensure_ascii=False):
/// escape only '"', '\\' and control characters; non-ASCII stays raw UTF-8.
fn escape_string(value: &str, out: &mut String) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_number(value: &serde_json::Number, out: &mut String) {
    if let Some(i) = value.as_i64() {
        out.push_str(&i.to_string());
    } else if let Some(u) = value.as_u64() {
        out.push_str(&u.to_string());
    } else if let Some(f) = value.as_f64() {
        // Python renders integral floats with a trailing .0; serde_json prints
        // the shortest form. JSON payloads here are int-dominated; mirror that.
        if f.fract() == 0.0 && f.is_finite() && f.abs() < 1e15 {
            out.push_str(&format!("{:.1}", f));
        } else {
            out.push_str(&f.to_string());
        }
    }
}

/// Canonical form used for after_digest: Python json.dumps(obj, sort_keys=True)
/// with default separators (', ' between items, ': ' after keys).
pub fn python_canonical(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(n, out),
        Value::String(s) => escape_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                escape_string(key, out);
                out.push_str(": ");
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn sha256_canonical(value: &Value) -> String {
    format!("sha256:{}", sha256_hex(python_canonical(value).as_bytes()))
}

/// sha256 digest of a file's bytes; None when unreadable.
pub fn sha256_file(path: &std::path::Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(format!("sha256:{}", sha256_hex(&bytes)))
}

/// Contract-style JSON equality (mirrors tests/contract_validator.py).
pub fn json_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Bool(_), _) | (_, Value::Bool(_)) => false,
        (Value::Number(a), Value::Number(b)) => {
            match (a.as_f64(), b.as_f64()) {
                (Some(x), Some(y)) => x == y,
                _ => a == b,
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| json_equal(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.keys()
                    .all(|key| b.get(key).map_or(false, |other| json_equal(&a[key], other)))
        }
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_uses_python_separators_and_sorted_keys() {
        let value = json!({"b": [1, true], "a": {"x": null}});
        assert_eq!(python_canonical(&value), "{\"a\": {\"x\": null}, \"b\": [1, true]}");
    }

    #[test]
    fn canonical_keeps_non_ascii_raw() {
        let value = json!({"提示": "占位符"});
        assert_eq!(python_canonical(&value), "{\"提示\": \"占位符\"}");
    }

    #[test]
    fn json_equal_matches_contract_semantics() {
        assert!(json_equal(&json!(1), &json!(1.0)));
        assert!(!json_equal(&json!(1), &json!(true)));
        assert!(json_equal(&json!({"a": [1]}), &json!({"a": [1.0]})));
        assert!(!json_equal(&json!({"a": 1}), &json!({"a": 2})));
    }

    #[test]
    fn sha256_canonical_matches_python() {
        // python3: hashlib.sha256(json.dumps({"a": 1}, sort_keys=True).encode()).hexdigest()
        let value = json!({"a": 1});
        assert_eq!(
            sha256_canonical(&value),
            "sha256:f9d86028c6e0d64e225186f96acb69338b2c59764df79162107f5c4bb34d1310"
        );
        let non_ascii = json!({"提示": "占位符"});
        assert_eq!(
            sha256_canonical(&non_ascii),
            "sha256:8bfebce8c16f28477007e1c797346b3f1d11e128dc2d029e27cf66487092ce01"
        );
    }
}
