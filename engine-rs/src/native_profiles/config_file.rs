//! 按实例定义定位并只读回读实际配置文件；原文仅供用户主动查看。

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::model;
use crate::native::{Context, Request};

const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) fn read(ctx: &Context, req: &Request) -> Result<Value> {
    if !matches!(req.args.len(), 2 | 4)
        || req.args[0] != "--only"
        || (req.args.len() == 4 && req.args[2] != "--file")
    {
        bail!("profiles:config-file 只接受 --only profiles:<实例名> [--file <文件>]");
    }
    let id = req.args[1]
        .strip_prefix("profiles:")
        .filter(|id| model::valid_profile_id(id))
        .ok_or_else(|| anyhow!("需要 --only profiles:<实例名>"))?;
    let policy = crate::store::effective_policy(ctx)?;
    let def = model::load_definitions(&policy)
        .into_iter()
        .filter_map(Result::ok)
        .find(|def| def.id == id)
        .ok_or_else(|| anyhow!("没有可用的独立环境：{id}"))?;
    let dir = model::validate_config_dir(&def, &ctx.home_dir, &ctx.data_dir)?;
    let requested = req.args.get(3).map(String::as_str).unwrap_or("main");
    let (name, format) = match (def.kind.as_str(), requested) {
        (_, "main") => (
            def.kind.target_file(),
            if def.kind.as_str() == "codex" {
                "toml"
            } else {
                "json"
            },
        ),
        ("claude", "mcp") => (".claude.json", "json"),
        ("claude", "rules") => (model::FILE_CLAUDE_RULES, "text"),
        ("codex", "hooks") => (model::FILE_CODEX_HOOKS, "json"),
        ("codex", "rules") => (model::FILE_CODEX_RULES, "text"),
        _ => bail!("这个独立环境没有可查看的指定文件"),
    };
    let path = dir.join(name);
    let real = crate::native_agent_settings::resolve_write_target(&path)?;
    let bytes = match std::fs::metadata(&real) {
        Ok(meta) => {
            if meta.len() > MAX_FILE_BYTES {
                bail!("配置文件超过安全读取上限");
            }
            Some(std::fs::read(&real)?)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };
    if bytes
        .as_ref()
        .is_some_and(|bytes| bytes.len() as u64 > MAX_FILE_BYTES)
    {
        bail!("配置文件超过安全读取上限");
    }
    let content = bytes
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()
        .ok()
        .flatten();
    Ok(json!({
        "element": format!("profiles:{id}"),
        "path": format!("{}/{}", def.config_dir_raw, name),
        "format": format,
        "exists": bytes.is_some(),
        "content": content,
        "blocked_reason": if bytes.is_some() && content.is_none() { Some("文件不是 UTF-8 文本") } else { None },
    }))
}
