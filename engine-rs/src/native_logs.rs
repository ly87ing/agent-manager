use chrono::{NaiveDate, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use crate::native::{Context, DispatchError, DispatchResult, Request};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DiagnosticLogEntry {
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub command: String,
    pub ok: bool,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<SafeDiagnosticItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct SafeDiagnosticItem {
    pub code: String,
    pub message: String,
}

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

/// Determines whether a request warrants writing to the structured diagnostic log.
/// Mutating commands (apply, policy set/delete, sources set, system
/// refresh-upstream, db migrate, runs:clear) are logged.
/// Readonly commands (status, plan, list, get, audit) and dry-run are never logged.
pub fn should_log_request(request: &Request) -> bool {
    if request.flag("--dry-run") {
        return false;
    }
    let norm = request.command.replace('.', ":");
    let (domain, verb) = match norm.split_once(':') {
        Some((d, v)) => (d, v),
        None => return false,
    };
    if verb == "status" || verb == "plan" || verb == "list" || verb == "get" || verb == "audit" {
        return false;
    }
    if domain == "logs" && verb == "clear" {
        return false;
    }
    matches!(
        (domain, verb),
        (_, "apply")
            | ("policy", "set" | "delete")
            | ("sources", "set")
            | ("system", "refresh-upstream")
            | ("db", "migrate")
            | ("runs", "clear")
    )
}

/// Checks if a file name strictly matches the daily log naming format: agent-manager-YYYY-MM-DD.jsonl
pub fn is_app_log_file(file_name: &str) -> bool {
    parse_log_file_date(file_name).is_some()
}

/// Extracts the NaiveDate from a valid daily log file name.
pub fn parse_log_file_date(file_name: &str) -> Option<NaiveDate> {
    let stripped = file_name
        .strip_prefix("agent-manager-")?
        .strip_suffix(".jsonl")?;
    if stripped.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(stripped, "%Y-%m-%d").ok()
}

/// Ensures Unix directory permissions 0700.
#[cfg(unix)]
fn set_dir_perms_0700(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

/// Ensures Unix file permissions 0600.
#[cfg(unix)]
fn set_file_perms_0600(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// Writes a single JSONL entry into the current daily diagnostic log file.
pub fn write_diagnostic_log(
    ctx: &Context,
    entry: &DiagnosticLogEntry,
) -> Result<PathBuf, anyhow::Error> {
    let logs_dir = ctx.data_dir.join("logs");
    if !logs_dir.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&logs_dir)?;
        }
        #[cfg(not(unix))]
        {
            std::fs::create_dir_all(&logs_dir)?;
        }
    }
    #[cfg(unix)]
    set_dir_perms_0700(&logs_dir)?;

    let today_utc = Utc::now().format("%Y-%m-%d").to_string();
    let filename = format!("agent-manager-{today_utc}.jsonl");
    let log_path = logs_dir.join(&filename);

    if log_path.exists() {
        let meta = std::fs::symlink_metadata(&log_path)?;
        if meta.file_type().is_symlink() {
            anyhow::bail!("拒绝向符号链接写入诊断日志：{}", log_path.display());
        }
        if !meta.file_type().is_file() {
            anyhow::bail!("目标日志路径不是普通文件：{}", log_path.display());
        }
    }

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&log_path)?;
    #[cfg(unix)]
    set_file_perms_0600(&log_path)?;

    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    use std::io::Write;
    file.write_all(line.as_bytes())?;
    file.flush()?;

    Ok(log_path)
}

/// Writes the diagnostic log and performs automatic retention pruning.
/// Any warning encountered during settings lookup or pruning is returned in the second element.
pub fn write_and_prune_diagnostic_log(
    ctx: &Context,
    entry: &DiagnosticLogEntry,
) -> (Result<PathBuf, anyhow::Error>, Vec<String>) {
    let mut warnings = Vec::new();
    let write_res = write_diagnostic_log(ctx, entry);
    if write_res.is_ok() {
        let logs_dir = ctx.data_dir.join("logs");
        match crate::store::effective_settings(ctx) {
            Ok(settings) => {
                if settings.log_retention_days > 0 {
                    if let Err(err) =
                        prune_expired_jsonl_logs(&logs_dir, settings.log_retention_days)
                    {
                        warnings.push(format!(
                            "清理过期诊断日志失败：{}",
                            sanitize_diagnostic_message(&err.to_string())
                        ));
                    }
                }
            }
            Err(err) => {
                warnings.push(format!(
                    "读取日志保留设置失败：{}",
                    sanitize_diagnostic_message(&err.to_string())
                ));
            }
        }
    }
    (write_res, warnings)
}

/// Prunes expired JSONL log files based strictly on the daily log filename date (`agent-manager-YYYY-MM-DD.jsonl`).
/// Ignores symlinks and files not matching the strict filename pattern.
pub fn prune_expired_jsonl_logs(
    logs_dir: &Path,
    retention_days: u32,
) -> Result<usize, anyhow::Error> {
    if retention_days == 0 || !logs_dir.exists() {
        return Ok(0);
    }
    let today = Utc::now().date_naive();
    let threshold = today - chrono::Duration::days(retention_days as i64);

    let mut pruned = 0;
    let entries = std::fs::read_dir(logs_dir)
        .map_err(|e| anyhow::anyhow!("读取日志目录失败（{}）：{e}", logs_dir.display()))?;
    for entry_res in entries {
        let entry = entry_res
            .map_err(|e| anyhow::anyhow!("遍历日志条目失败（{}）：{e}", logs_dir.display()))?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| anyhow::anyhow!("获取日志元数据失败（{}）：{e}", path.display()))?;
        // Refuse to touch symlinks or non-regular files
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        let Some(file_date) = parse_log_file_date(&name_str) else {
            continue;
        };

        if file_date < threshold {
            std::fs::remove_file(&path)
                .map_err(|e| anyhow::anyhow!("删除过期日志失败（{}）：{e}", path.display()))?;
            pruned += 1;
        }
    }
    Ok(pruned)
}

/// Startup maintenance: cleans up expired logs without creating missing directories.
pub fn startup_maintenance(ctx: &Context) -> Result<usize, anyhow::Error> {
    let logs_dir = ctx.data_dir.join("logs");
    if !logs_dir.exists() {
        return Ok(0);
    }
    let settings = crate::store::effective_settings(ctx)?;
    if settings.log_retention_days == 0 {
        return Ok(0);
    }
    prune_expired_jsonl_logs(&logs_dir, settings.log_retention_days)
}

/// Native logs:clear command: deletes all regular agent-manager-*.jsonl files.
/// Zero-writes when the directory does not exist.
pub(crate) fn logs_clear(ctx: &Context) -> DispatchResult {
    let logs_dir = ctx.data_dir.join("logs");
    if !logs_dir.exists() {
        return Ok((json!({ "domain": "logs", "deleted": 0 }), vec![]));
    }
    let entries = std::fs::read_dir(&logs_dir).map_err(|err| {
        DispatchError::new(
            "LOGS_CLEAR_ERROR",
            sanitize_diagnostic_message(&format!(
                "读取日志目录失败（{}）：{err}",
                logs_dir.display()
            )),
        )
    })?;
    let mut count = 0;
    for entry_res in entries {
        let entry = entry_res.map_err(|err| {
            DispatchError::new(
                "LOGS_CLEAR_ERROR",
                sanitize_diagnostic_message(&format!(
                    "遍历日志条目失败（{}）：{err}",
                    logs_dir.display()
                )),
            )
        })?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|err| {
            DispatchError::new(
                "LOGS_CLEAR_ERROR",
                sanitize_diagnostic_message(&format!(
                    "获取日志元数据失败（{}）：{err}",
                    path.display()
                )),
            )
        })?;
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !is_app_log_file(&name_str) {
            continue;
        }
        std::fs::remove_file(&path).map_err(|err| {
            DispatchError::new(
                "LOGS_CLEAR_ERROR",
                sanitize_diagnostic_message(&format!(
                    "删除日志文件失败（{}）：{err}",
                    path.display()
                )),
            )
        })?;
        count += 1;
    }
    Ok((json!({ "domain": "logs", "deleted": count }), vec![]))
}
