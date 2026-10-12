//! 只读 `system:discover`：扫描本机已存在、但还没纳入 Agent Manager 管理的
//! skills 资源，按来源分类并给出接管方式建议。MCP 条目以各助手配置
//! 文件为准，直接在 MCP 页查看与编辑，不再经过「导入」。
//!
//! 统一分类词汇（`manage` 字段，前端据此决定可做什么）：
//! - `native`：位于助手官方路径、由各自原生安装方案管理 —— 只标识，不接管；
//! - `symlink`：外部路径资源 —— 登记本地来源集合后由 manager 以软链接纳管；
//! - `collection`：已定义集合的计划外链接 —— 走既有集合接管通道；
//! - `cleanup`：断链或旧体系残留 —— 只建议清理。
//!
//! 零写零网络：各域发现复用其 status 侧的只读视图（effective_policy + binding +
//! 目录 symlink 级扫描），不探测助手 CLI，不创建 data_dir/DB/目录，
//! 不取 apply 锁，不写 operation_logs。域内失败不谎报空结果，直接失败。

use anyhow::{bail, Result};
use serde_json::{json, Map, Value};

use crate::native::{Context, Request};

const DOMAINS: &[&str] = &["skills"];

fn parse_only(req: &Request) -> Result<Vec<String>> {
    let mut idx = 0;
    let mut selected: Vec<String> = Vec::new();
    while idx < req.args.len() {
        let arg = &req.args[idx];
        if arg == "--only" {
            let value = req
                .args
                .get(idx + 1)
                .ok_or_else(|| anyhow::anyhow!("--only 缺少取值"))?;
            if !DOMAINS.contains(&value.as_str()) {
                bail!("--only 仅支持 skills，得到 '{}'", value);
            }
            if !selected.contains(value) {
                selected.push(value.clone());
            }
            idx += 2;
            continue;
        }
        bail!("system:discover 不支持参数 '{}'", arg);
    }
    if selected.is_empty() {
        selected = DOMAINS.iter().map(|d| d.to_string()).collect();
    }
    Ok(selected)
}

pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let selected = parse_only(req)?;
    let mut data = Map::new();
    let mut summary = Map::new();

    if selected.iter().any(|d| d == "agents") {
        let agents = crate::native_agents::discover(ctx)?;
        summary.insert(
            "agents".into(),
            json!(agents.as_array().map(Vec::len).unwrap_or(0)),
        );
        data.insert("agents".into(), agents);
    }
    if selected.iter().any(|d| d == "skills") {
        let skills = crate::native_skills::discover_unmanaged(ctx)?;
        let items = skills["items"].as_array().cloned().unwrap_or_default();
        let count = |manage: &str| items.iter().filter(|item| item["manage"] == manage).count();
        summary.insert("skills_symlink".into(), json!(count("symlink")));
        summary.insert("skills_collection".into(), json!(count("collection")));
        summary.insert("skills_native".into(), json!(count("native")));
        summary.insert("skills_cleanup".into(), json!(count("cleanup")));
        data.insert("skills".into(), skills);
    }
    data.insert("domain".into(), json!("discover"));
    data.insert("summary".into(), Value::Object(summary));
    Ok(Value::Object(data))
}
