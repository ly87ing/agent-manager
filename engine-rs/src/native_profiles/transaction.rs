//! One account mutation coordinates its secret source, real settings, and registration.
//! All plans are read-only. The caller holds the manager write lock, and failures restore
//! only bytes still owned by this operation; external changes are never overwritten.

use anyhow::{anyhow, bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};
use std::path::PathBuf;

use super::apply::{self, Plan};
use super::model::{self, ProfileDef};
use super::persistence::{write_binding, Binding};
use super::status::definition_base;
use crate::native::{Context, Request};
use crate::native_mcp::rollback::{read_existing_bytes, safe_restore_bytes_cas};
use crate::{secret_values, store};

pub(super) struct PendingSecret<'a> {
    name: Option<&'a str>,
    value: Option<&'a str>,
}

impl PendingSecret<'_> {
    pub(super) fn resolve(&self, name: &str) -> Option<String> {
        if self.name == Some(name) {
            self.value.map(str::to_string)
        } else {
            secret_values::lookup_account(name).ok()
        }
    }
}

pub(super) fn pending_secret(req: &Request) -> Result<PendingSecret<'_>> {
    let name = req.value("--secret-var");
    let value = req.value("--secret-value");
    if name.is_some() != value.is_some()
        || name.is_some_and(|name| !secret_values::valid_var_name(name))
        || value.is_some_and(|value| value.is_empty() || value.chars().any(char::is_control))
    {
        bail!("账号密钥必须同时提供有效变量名与非空取值");
    }
    if name.is_some() {
        secret_values::ensure_source_ready()?;
    }
    Ok(PendingSecret { name, value })
}

pub(super) fn prepare_secret(
    ctx: &Context,
    req: &Request,
    def: &ProfileDef,
) -> Result<Option<secret_values::SecretWrite>> {
    let secret = pending_secret(req)?;
    let Some(name) = secret.name else {
        return Ok(None);
    };
    let referenced = def.env_key.as_deref() == Some(name)
        || def.env.iter().any(|(key, value)| {
            crate::native::credential_like_name(key)
                && value
                    .as_str()
                    .is_some_and(|value| value == format!("${{{name}}}"))
        });
    if !referenced {
        bail!("待保存密钥不属于这个账号的凭据引用，没有写入");
    }
    model::validate_secret_ownership(ctx, def)?;
    Ok(Some(secret_values::plan_set(
        ctx,
        name,
        secret.value.unwrap(),
    )?))
}

pub(super) enum DefinitionChange {
    Keep {
        expected: String,
    },
    Replace {
        candidate: Value,
        expected: String,
    },
    Create {
        candidate: Value,
        expected: Option<Value>,
    },
}

impl DefinitionChange {
    fn apply(&self, policy: &mut Value, id: &str) -> Result<bool> {
        let definitions = policy
            .pointer_mut("/agents/definitions")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("账号登记表已在外部修改，没有写入"))?;
        let current = definitions.get(id);
        match self {
            Self::Keep { expected } | Self::Replace { expected, .. } => {
                if current.map(definition_base).transpose()?.as_deref() != Some(expected.as_str()) {
                    bail!("[{id}] 账号定义已修改或移除，没有写入；请刷新后重试");
                }
            }
            Self::Create { expected, .. } => {
                if current != expected.as_ref() {
                    bail!("[{id}] 账号登记已变化，没有创建；请刷新后重试");
                }
            }
        }
        if let Self::Replace { candidate, .. } | Self::Create { candidate, .. } = self {
            definitions.insert(id.to_string(), candidate.clone());
            return Ok(true);
        }
        Ok(false)
    }
}

fn missing_directories(path: &std::path::Path) -> Vec<PathBuf> {
    let mut missing = Vec::new();
    let mut current = Some(path);
    while let Some(path) = current {
        if path.exists() {
            break;
        }
        missing.push(path.to_path_buf());
        current = path.parent();
    }
    missing
}

pub(super) fn commit(
    ctx: &Context,
    req: &Request,
    plan: Plan,
    definition: DefinitionChange,
    secret: Option<secret_values::SecretWrite>,
) -> Result<Value> {
    let _account_lock = crate::native_account_context::lock_directory(ctx, &plan.dir, true)?;
    // A real authentication probe is performed for every API write, even if settings
    // already exist. It uses the explicit account context and never receives the key.
    apply::ensure_no_official_login(ctx, &plan.def, &plan.dir)?;
    apply::check_plan_paths(ctx, &plan)?;
    let new_dirs = missing_directories(&plan.dir);
    let mut wrote_secret = false;
    let mut wrote_file = false;
    let result = (|| -> Result<()> {
        // Reserve both previous and candidate references in the same database
        // transaction before touching either file. Removing a reference does not
        // release the credential name to another account or registration generation.
        let mut conn = store::open_write(ctx)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut policy = store::policy_from_connection(&tx)?;
        let before = policy.clone();
        let changed = definition.apply(&mut policy, &plan.def.id)?;
        super::paths::validate_policy_account_paths(ctx, &policy)?;
        crate::native_secrets::reserve_account_references(
            ctx,
            &tx,
            &plan.def.id,
            &before,
            Some(&policy),
        )?;
        model::validate_secret_ownership_in_policy(ctx, &plan.def, &policy)?;
        let references =
            crate::native_secrets::account_reference_names(ctx, &plan.def.id, &policy)?;
        if changed {
            store::write_policy(&tx, &policy)?;
        }
        crate::native_secrets::claim_variable_ownership(ctx, &tx, &plan.def.id, &references)?;
        if let Some(secret) = &secret {
            secret.commit(ctx)?;
            wrote_secret = true;
        }
        apply::ensure_dir(&plan.dir)?;
        apply::check_plan_paths(ctx, &plan)?;
        if read_existing_bytes(&plan.real)? != plan.previous {
            bail!("账号配置已在外部修改，没有写入；请刷新后重试");
        }
        if !(plan.previous.is_none() && model::is_empty_render(plan.def.kind, &plan.content)) {
            apply::atomic_write(&plan.real, &plan.content, plan.previous.as_deref())?;
            wrote_file = true;
            if read_existing_bytes(&plan.real)?.as_deref() != Some(plan.content.as_slice()) {
                bail!("账号配置写后回读不一致");
            }
        }
        let base = model::live_base(plan.def.kind, Some(&plan.content))?;
        write_binding(
            &tx,
            &Binding {
                profile_id: plan.def.id.clone(),
                file: plan.file.clone(),
                applied_digest: if base == model::BASE_MISSING {
                    crate::native_mcp::model::sha256_hex_bytes(&plan.content)
                } else {
                    base
                },
                updated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                owned: model::owned_paths(&plan.def),
            },
        )?;
        store::record_operation(
            &tx,
            Some(&req.run_id),
            &req.command.replace('.', ":"),
            Some(&plan.def.id),
            Some(&json!({
                "action": req.command.replace('.', ":"),
                "kind": plan.def.kind.as_str(),
                "created": plan.previous.is_none(),
                "files": [plan.file],
                "secret_changed": secret.is_some(),
            })),
        )?;
        apply::check_plan_paths(ctx, &plan)?;
        if wrote_file
            && read_existing_bytes(&plan.real)?.as_deref() != Some(plan.content.as_slice())
        {
            bail!("账号配置在提交前已被外部修改，拒绝登记");
        }
        model::validate_local_auth(
            &ctx.home_dir,
            plan.def.kind,
            Some(&plan.dir),
            model::account_auth_mode(&plan.def),
        )?;
        if let Some(secret) = &secret {
            secret.verify_committed(ctx)?;
        }
        secret_values::refresh(ctx);
        let pending = pending_secret(req)?;
        model::validate_effective_config(&plan.def, &plan.content, &|name| pending.resolve(name))?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = result {
        let mut failures = Vec::new();
        if wrote_file {
            let restore = apply::check_plan_paths(ctx, &plan).and_then(|_| {
                safe_restore_bytes_cas(&plan.real, &plan.content, plan.previous.as_deref())
            });
            if let Err(restore) = restore {
                failures.push(format!("配置回滚冲突: {restore:#}"));
            }
        }
        if wrote_secret {
            if let Some(secret) = &secret {
                if let Err(restore) = secret.rollback(ctx) {
                    failures.push(format!("密钥回滚冲突: {restore:#}"));
                }
            }
        }
        // Only empty directories that were missing before this operation are removed.
        for dir in new_dirs {
            match std::fs::remove_dir(&dir) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => failures.push(format!("新建目录清理失败: {error}")),
            }
        }
        secret_values::refresh(ctx);
        if !failures.is_empty() {
            return Err(anyhow!("执行失败: {err:#}；{}", failures.join("；")));
        }
        return Err(err);
    }
    secret_values::refresh(ctx);
    Ok(json!({
        "domain": "profiles",
        "steps": [{"name": format!("{} {}", req.command.replace('.', ":"), plan.def.id), "ok": true, "exit_code": 0}],
        "skipped": [],
        "base": model::live_base(plan.def.kind, Some(&plan.content))?,
    }))
}
