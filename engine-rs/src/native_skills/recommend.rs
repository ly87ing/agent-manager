//! 编译内置的推荐技能来源（只读）。
//!
//! 推荐不是集合：不进库、不作为 status 元素、不计入待同步、不参与上游检查，也不自动下载。
//! skills:status 以独立字段 `recommendations` 输出；用户「添加」后才由前端把 `definition`
//! 写成普通集合定义（不限助手、不分配给任何助手），再逐个技能启用。
//!
//! 旧版本曾把 agent-skills 固化进库；库里仍为原值且从未生效（无 skill_bindings）的那一条
//! 在读路径即时按推荐处理（本模块 `unadopted_builtin_collections`），写路径由
//! store::open_write 的迁移步骤移除。

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

use super::{BindingEntry, CollectionDef};
use crate::store;

struct Recommendation {
    name: &'static str,
    description: &'static str,
    url: &'static str,
    skill_paths: &'static [&'static str],
}

const RECOMMENDATIONS: &[Recommendation] = &[Recommendation {
    name: "agent-skills",
    description: "本项目作者维护的通用工作技能：合并审查、方案设计、改动纪律、验证、临时产物整理、面向读者写作、技术图示；另含常驻规则（rules/），取回后在规则页作为模板出现。",
    url: "https://github.com/ly87ing/agent-skills.git",
    skill_paths: &[
        "safe-merge-review",
        "solution-shaping",
        "change-discipline",
        "verification",
        "artifact-hygiene",
        "reader-facing-writing",
        "technical-diagramming",
    ],
}];

/// 旧库里固化、仍为原值且该集合没有任何 skill_bindings 行（从未在任何助手生效）的
/// 内置集合名。读路径据此把它当推荐处理，与写路径迁移同一判定。零写。
pub(crate) fn unadopted_builtin_collections(
    policy: &Value,
    bindings: &BTreeMap<(String, String, String), BindingEntry>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(repos) = policy
        .get("skills")
        .and_then(|v| v.get("repositories"))
        .and_then(Value::as_object)
    else {
        return out;
    };
    for (name, raw) in repos {
        if store::is_legacy_builtin_skill_repo(name, raw)
            && !bindings.keys().any(|(coll, _, _)| coll == name)
        {
            out.insert(name.clone());
        }
    }
    out
}

/// 某个域（规则、别名）没有任何来源内容时给界面的推荐来源：与技能推荐同一份，
/// `collection` 为已登记的同名/同地址集合（取回即可），没登记为 null。只读零写。
pub(crate) fn suggested_sources(ctx: &crate::native::Context) -> anyhow::Result<Value> {
    let meta = super::status::load_metadata(ctx)?;
    let items = RECOMMENDATIONS
        .iter()
        .map(|rec| {
            let collection = meta
                .collections
                .values()
                .find(|c| !c.deleted && (c.name == rec.name || c.url.as_deref() == Some(rec.url)))
                .map(|c| c.name.clone());
            json!({
                "name": rec.name,
                "description": rec.description,
                "url": rec.url,
                "collection": collection,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

/// status 的 `recommendations` 字段。`added`：已有同名或同地址的集合（未删除），
/// 界面据此不再提供「添加」，避免覆盖用户已有的同名集合。
pub(crate) fn recommendations_value(collections: &BTreeMap<String, CollectionDef>) -> Value {
    let items: Vec<Value> = RECOMMENDATIONS
        .iter()
        .map(|rec| {
            let added = collections.values().any(|coll| {
                !coll.deleted && (coll.name == rec.name || coll.url.as_deref() == Some(rec.url))
            });
            json!({
                "name": rec.name,
                "description": rec.description,
                "url": rec.url,
                "skill_paths": rec.skill_paths,
                "added": added,
                // 添加时写入的集合定义：不带 agents（不限助手），也不写任何分配
                "definition": {"url": rec.url, "skill_paths": rec.skill_paths},
            })
        })
        .collect();
    Value::Array(items)
}
