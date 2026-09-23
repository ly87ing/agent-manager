//! rules:save-template / rules:delete-template：只写库（rule_templates），不动任何
//! 规则文件，不取 apply 锁。参数与校验失败在打开写连接前拒绝（零写）；修改用
//! revision CAS；审计只记模板 id 与 revision，不记名称与正文。

use anyhow::{anyhow, bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};

use super::args;
use super::status;
use super::templates::{self, Template};
use crate::native::{Context, Request};
use crate::store;

fn check_against(list: &[Template], args: &args::SaveArgs, name: &str) -> Result<()> {
    let editing = args.existing.as_ref().map(|(id, _)| id.as_str());
    if let Some((id, revision)) = &args.existing {
        let current = list
            .iter()
            .find(|t| &t.id == id)
            .ok_or_else(|| anyhow!("模板不存在或已被删除，请刷新后重试"))?;
        if &current.revision() != revision {
            bail!("模板已被修改，请重新读取后再保存");
        }
    }
    if list
        .iter()
        .any(|t| t.name == name && Some(t.id.as_str()) != editing)
    {
        bail!("已有同名模板，换一个名称");
    }
    Ok(())
}

pub(crate) fn save(ctx: &Context, req: &Request) -> Result<Value> {
    let parsed = args::parse_save_args(req)?;
    let name = templates::validate_name(&parsed.name)?;
    templates::validate_body(&parsed.body)?;
    // 只读预检：过期 revision、同名、不存在的模板在创建任何东西之前拒绝。
    let (_, list) = status::load_snapshot(ctx)?;
    check_against(&list, &parsed, &name)?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    templates::ensure_table(&tx)?;
    let list = templates::load_from(&tx)?;
    check_against(&list, &parsed, &name)?;
    let now = templates::now();
    let template = match &parsed.existing {
        Some((id, _)) => {
            let mut current = list
                .iter()
                .find(|t| &t.id == id)
                .cloned()
                .ok_or_else(|| anyhow!("模板不存在或已被删除，请刷新后重试"))?;
            current.name = name;
            current.body = parsed.body.clone();
            current.updated_at = now;
            templates::update(&tx, &current)?;
            current
        }
        None => {
            let template = Template {
                id: templates::new_id(),
                name,
                body: parsed.body.clone(),
                origin: templates::ORIGIN_USER.to_string(),
                created_at: now.clone(),
                updated_at: now,
            };
            templates::insert(&tx, &template)?;
            template
        }
    };
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "rules:save-template",
        Some(&template.id),
        Some(&json!({ "template": template.id, "revision": template.revision() })),
    )?;
    tx.commit()?;
    Ok(json!({ "domain": "rules", "template": template.to_json() }))
}

pub(crate) fn delete(ctx: &Context, req: &Request) -> Result<Value> {
    let id = args::parse_delete_args(req)?;
    let (_, list) = status::load_snapshot(ctx)?;
    if !list.iter().any(|t| t.id == id) {
        bail!("模板不存在或已被删除");
    }
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    templates::ensure_table(&tx)?;
    if !templates::load_from(&tx)?.iter().any(|t| t.id == id) {
        bail!("模板不存在或已被删除");
    }
    templates::delete(&tx, &id)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "rules:delete-template",
        Some(&id),
        Some(&json!({ "template": id })),
    )?;
    tx.commit()?;
    Ok(json!({ "domain": "rules", "deleted": id }))
}
