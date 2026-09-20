//! 错误消息脱敏：envelope 里的 errors/warnings 在出库前统一走这里，
//! 凭据、私钥、URL userinfo 与敏感查询参数一律掩码，错误因果链保持可读。

use regex::Regex;

/// Centralized redaction function: masks tokens, passwords, keys, bearer tokens,
/// private keys, URL credentials and sensitive query parameters while preserving
/// the overall error context and anyhow cause chains.
pub fn sanitize_diagnostic_message(raw: &str) -> String {
    // 1. Private keys
    let re_privkey = Regex::new(
        r"(?s)-----BEGIN [A-Z0-9_-]+ PRIVATE KEY-----.*?-----END [A-Z0-9_-]+ PRIVATE KEY-----",
    )
    .unwrap();
    let res = re_privkey.replace_all(raw, "[REDACTED PRIVATE KEY]");

    // 2. Authorization headers (Authorization: Bearer <token> or Authorization: <token>)
    let re_auth_header =
        Regex::new(r"(?i)\b(authorization\s*[:=]\s*(?:bearer\s+|basic\s+)?)[A-Za-z0-9_\-\.~+/=]+")
            .unwrap();
    let res = re_auth_header.replace_all(&res, "${1}***");

    // 3. Standalone Bearer tokens (Bearer <token>)
    let re_bearer = Regex::new(r"(?i)\b(bearer\s+)[A-Za-z0-9_\-\.~+/=]+").unwrap();
    let res = re_bearer.replace_all(&res, "${1}***");

    // 4. URL userinfo (http(s)://user:pass@host)
    let re_url_userinfo = Regex::new(r"(https?://[^:/@\s]+):([^@\s]+)@").unwrap();
    let res = re_url_userinfo.replace_all(&res, "${1}:***@");

    // 5. URL query sensitive parameters (?token=... / &api_key=...)
    let re_url_query = Regex::new(
        r"(?i)([?&](?:token|password|pass|secret|api_key|apikey|key|auth|access_token)=)([^&\s]+)",
    )
    .unwrap();
    let res = re_url_query.replace_all(&res, "${1}***");

    // 6. Common key-value credential pairs (double-quoted, single-quoted, and unquoted)
    let re_kv_double = Regex::new(
        r#"(?i)\b(token|password|passwd|api_key|apikey|secret|auth_token)\b(\s*[:=]\s*)"[^"]*""#,
    )
    .unwrap();
    let res = re_kv_double.replace_all(&res, "${1}${2}\"***\"");

    let re_kv_single = Regex::new(
        r"(?i)\b(token|password|passwd|api_key|apikey|secret|auth_token)\b(\s*[:=]\s*)'[^']*'",
    )
    .unwrap();
    let res = re_kv_single.replace_all(&res, "${1}${2}'***'");

    let re_kv_unquoted = Regex::new(
        r#"(?i)\b(token|password|passwd|api_key|apikey|secret|auth_token)\b(\s*[:=]\s*)([^"'\s,;&*]+)"#,
    )
    .unwrap();
    let res = re_kv_unquoted.replace_all(&res, "${1}${2}***");

    // 7. Common prefix-based tokens
    let re_gh = Regex::new(r"\b(gh[pousr]_[A-Za-z0-9_]{36,})\b").unwrap();
    let res = re_gh.replace_all(&res, "gh***");

    let re_sk = Regex::new(r"\b(sk-[A-Za-z0-9_-]{20,})\b").unwrap();
    let res = re_sk.replace_all(&res, "sk-***");

    let re_xox = Regex::new(r"\b(xox[baprs]-[A-Za-z0-9\-]+)\b").unwrap();
    let res = re_xox.replace_all(&res, "xox***");

    res.into_owned()
}
