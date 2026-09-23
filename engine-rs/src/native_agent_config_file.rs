//! 助手配置文件原文编辑：`agents:config-file` 读、`agents:edit-config` 写。
//!
//! 表单只覆盖少量常用运行参数；其余设置（权限规则、插件、`modelSettings`、自定义 env……）
//! 直接编辑实际配置文件全文——Claude 的 `~/.claude/settings.json`、Codex 的 `config.toml`。
//! 不在本工具里再实现一遍官方 schema，也不另存副本。
//!
//! 读写都是文件原文：读取原样返回（和用户自己打开文件看到的一样），写回用户给的全文，
//! 只校验能解析（Claude 须是 JSON 对象），不重新排版。写入基准是整个文件的摘要：打开后
//! 文件在别处被改过（包括助手自己改）就拒绝，不覆盖。审计只记改动了哪些顶层键，不记取值。

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::native::{Context, DispatchError, Request};
use crate::native_agent_settings::Kind;

/// 文件不存在时的写入基准。
const BASE_MISSING: &str = "missing";

fn sha256(bytes: &[u8]) -> String {
    crate::native_mcp::model::sha256_hex_bytes(bytes)
}

/// 解析全文，返回顶层键 → 该键内容的文本（只用来比对改了哪些键）。解析不了给出位置，
/// 不带原文片段——里面可能有令牌，错误信息会进界面提示。
fn top_level(kind: Kind, text: &str) -> Result<BTreeMap<String, String>> {
    match kind {
        Kind::Claude => {
            let value: Value = serde_json::from_str(text).map_err(|err| {
                anyhow!(
                    "不是合法的 JSON（第 {} 行第 {} 列）",
                    err.line(),
                    err.column()
                )
            })?;
            let map = value
                .as_object()
                .ok_or_else(|| anyhow!("内容必须是 JSON 对象"))?;
            Ok(map
                .iter()
                .map(|(key, value)| (key.clone(), value.to_string()))
                .collect())
        }
        Kind::Codex => {
            let doc = text
                .parse::<toml_edit::Document>()
                .map_err(|err| match err.span() {
                    Some(span) => {
                        let line = text[..span.start.min(text.len())].matches('\n').count() + 1;
                        anyhow!("不是合法的 TOML（第 {line} 行）")
                    }
                    None => anyhow!("不是合法的 TOML"),
                })?;
            Ok(doc
                .iter()
                .map(|(key, item)| (key.to_string(), item.to_string()))
                .collect())
        }
    }
}

fn element_id(req: &Request) -> Result<String> {
    Ok(req
        .value("--only")
        .ok_or_else(|| anyhow!("需要 --only agents:<助手>"))?
        .trim_start_matches("agents:")
        .to_string())
}

fn format_of(kind: Kind) -> &'static str {
    match kind {
        Kind::Claude => "json",
        Kind::Codex => "toml",
    }
}

fn tildify(ctx: &Context, path: &std::path::Path) -> String {
    match path.strip_prefix(&ctx.home_dir) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// `agents:config-file --only agents:<id>`：只读零写，原样返回文件全文。
pub(crate) fn read(ctx: &Context, req: &Request) -> Result<Value> {
    for arg in req.args.iter().filter(|arg| arg.starts_with("--")) {
        if arg != "--only" {
            bail!("agents:config-file 只接受 --only");
        }
    }
    let id = element_id(req)?;
    let (kind, path, _real, existing) = crate::native_agent_settings::editable_target(ctx, &id)?;
    let base = existing
        .as_deref()
        .map_or_else(|| BASE_MISSING.to_string(), sha256);
    let (content, blocked) = match existing.as_deref().map(std::str::from_utf8) {
        None => (Some(String::new()), None),
        Some(Ok(text)) => (Some(text.to_string()), None),
        Some(Err(_)) => (
            None,
            Some(format!(
                "{} 不是 UTF-8 文本，不能在这里编辑",
                tildify(ctx, &path)
            )),
        ),
    };
    Ok(json!({
        "domain": "agents",
        "element": format!("agents:{id}"),
        "path": tildify(ctx, &path),
        "format": format_of(kind),
        "exists": existing.is_some(),
        "content": content,
        "base": base,
        "blocked_reason": blocked,
    }))
}

/// `agents:edit-config --only agents:<id> --file-base <base> --value <全文>`：整文件 CAS 写入原文。
pub(crate) fn write(ctx: &Context, req: &Request) -> Result<Value, DispatchError> {
    let usage = |message: &str| DispatchError::new("USAGE_ERROR", message.to_string());
    let agent_error = |err: anyhow::Error| DispatchError::new("AGENT_ERROR", format!("{err:#}"));
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--only" | "--file-base" | "--value" => {
                iter.next();
            }
            _ => {
                return Err(usage(
                    "agents:edit-config 只接受 --only、--file-base 与 --value",
                ))
            }
        }
    }
    let id = element_id(req).map_err(agent_error)?;
    let base = req
        .value("--file-base")
        .ok_or_else(|| {
            usage("agents:edit-config 需要 --file-base：先读 agents:config-file，把 base 原样带回")
        })?
        .to_string();
    let text = req
        .value("--value")
        .ok_or_else(|| usage("agents:edit-config 需要 --value（配置文件全文）"))?
        .to_string();

    let (kind, path, real, existing) =
        crate::native_agent_settings::editable_target(ctx, &id).map_err(agent_error)?;
    let file_label = tildify(ctx, &path);
    let current = existing
        .as_deref()
        .map_or_else(|| BASE_MISSING.to_string(), sha256);
    if current != base {
        return Err(DispatchError::new(
            "AGENT_CONFIG_CHANGED",
            format!(
                "{file_label} 已在外部修改（与打开时读到的内容不同），没有写入；请刷新后重新编辑"
            ),
        ));
    }
    let after = top_level(kind, &text).map_err(|err| agent_error(err.context("新内容")))?;
    // 原文件读不懂时当作空配置：改动按新内容的全部顶层键记
    let before = existing
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .and_then(|old| top_level(kind, old).ok())
        .unwrap_or_default();
    let changed: Vec<String> = before
        .keys()
        .chain(after.keys())
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let new_bytes = text.into_bytes();

    if existing.as_deref() == Some(new_bytes.as_slice())
        || (existing.is_none() && changed.is_empty())
    {
        return Ok(json!({
            "domain": "agents", "element": format!("agents:{id}"), "path": file_label,
            "changed": [], "base": current,
        }));
    }
    if let Some(parent) = real.parent() {
        std::fs::create_dir_all(parent).map_err(|err| agent_error(err.into()))?;
    }
    if let Err(err) = crate::native_mcp::rollback::atomic_write_bytes_checked(
        &real,
        &new_bytes,
        existing.as_deref(),
    ) {
        // 只有文件确实变了才算外部修改；权限、磁盘等错误如实报出
        let now = crate::native_mcp::rollback::read_existing_bytes(&real).ok();
        if now.as_ref() != Some(&existing) {
            return Err(DispatchError::new(
                "AGENT_CONFIG_CHANGED",
                format!("{file_label} 已在外部修改（写入前核对不一致），没有写入"),
            ));
        }
        return Err(agent_error(err));
    }
    let audit = (|| -> Result<()> {
        let conn = crate::store::open_write(ctx)?;
        crate::store::record_operation(
            &conn,
            Some(&req.run_id),
            "agents:edit-config",
            Some(&format!("agents:{id}")),
            Some(&json!({"action": "edit-config", "changed": changed})),
        )?;
        Ok(())
    })();
    if let Err(err) = audit {
        // 审计写不进去：把文件还原，不留下没有记录的改动
        let _ = match existing.as_deref() {
            Some(bytes) => std::fs::write(&real, bytes),
            None => std::fs::remove_file(&real),
        };
        return Err(agent_error(err));
    }
    Ok(json!({
        "domain": "agents",
        "element": format!("agents:{id}"),
        "path": file_label,
        "changed": changed,
        "base": sha256(&new_bytes),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 解析失败只报位置_不带原文() {
        let err = top_level(
            Kind::Claude,
            "{\n  \"env\": {\"TOKEN\": \"tok-secret-1\",}\n}",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("第 2 行"), "{err}");
        assert!(!err.contains("tok-secret-1"), "{err}");
        let err = top_level(Kind::Codex, "model = \"o3\"\nkey = tok-secret-2\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("第 2 行"), "{err}");
        assert!(!err.contains("tok-secret-2"), "{err}");
        assert!(top_level(Kind::Claude, "[1]").is_err(), "JSON 必须是对象");
    }

    #[test]
    fn 顶层键比对只看内容() {
        let before = top_level(Kind::Codex, "# 注释\nmodel = \"o3\"\n[x]\na = 1\n").unwrap();
        let after = top_level(Kind::Codex, "# 注释\nmodel = \"o4\"\n[x]\na = 1\n").unwrap();
        let changed: Vec<_> = before
            .keys()
            .filter(|key| before.get(*key) != after.get(*key))
            .collect();
        assert_eq!(changed, vec!["model"]);
    }
}
