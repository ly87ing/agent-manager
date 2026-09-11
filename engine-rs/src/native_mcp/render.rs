//! MCP 目标配置渲染器（plan 与 apply 共享纯渲染逻辑）
//!
//! 渲染时一次性读出目标旧字节并随结果返回：调用方（apply）将该
//! `old_bytes` 作为 `expected_old` 交给 `atomic_write_checked`，在原子
//! 替换前再次读取并逐字节比较，任何外部并发改动都会 fail-closed 拒绝
//! 覆盖；plan 复用同一份旧字节计算 before_digest，两侧语义保持一致。

use anyhow::{bail, Result};
use serde_json::Value;
use std::path::Path;

use super::codecs::{
    json_doc_from_bytes, json_remove_entry, json_set_entry, render_entry_toml,
    toml_remove_server_text,
};
use super::rollback::read_existing_bytes;
use super::targets::{validate_target_path, Entry, Format};

pub struct RenderedTarget {
    pub content: String,
    pub old_bytes: Option<Vec<u8>>,
}

pub fn render_target_content(
    path: &Path,
    format: &Format,
    server: &str,
    entry: Option<&Entry>,
) -> Result<RenderedTarget> {
    validate_target_path(path)?;
    let old_bytes = read_existing_bytes(path)?;
    let content = match format {
        Format::ClaudeJson | Format::OpenCodeJson => {
            let mut doc = match &old_bytes {
                Some(bytes) => json_doc_from_bytes(path, bytes)?,
                None => Default::default(),
            };
            if let Some(e) = entry {
                json_set_entry(&mut doc, format.clone(), server, e);
            } else {
                json_remove_entry(&mut doc, format.clone(), server);
            }
            serde_json::to_string_pretty(&Value::Object(doc))?
        }
        Format::CodexToml => {
            let current_text = match &old_bytes {
                Some(bytes) => String::from_utf8(bytes.clone()).map_err(|exc| {
                    anyhow::anyhow!(
                        "配置文件不是有效 UTF-8，拒绝覆盖: {} ({})",
                        path.display(),
                        exc
                    )
                })?,
                None => String::new(),
            };
            let mut content = toml_remove_server_text(&current_text, server);
            if let Some(e) = entry {
                if !content.ends_with('\n') && !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(&render_entry_toml(server, e));
            }
            content
        }
        Format::Unsupported(fmt) => {
            bail!("不支持的配置格式: {}", fmt);
        }
    };
    Ok(RenderedTarget { content, old_bytes })
}
