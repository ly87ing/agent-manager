//! 来源别名：技能来源（git 仓库镜像或本地目录）根下 `aliases/*.json` 提供的推荐快捷命令。
//!
//! 推荐别名不随程序发布：内容在内容仓库里维护（格式与旧版内置的 shell-aliases.json 相同：
//! timezone、lang 与 aliases），本工具与技能共用集合定义、镜像目录与「更新资源库」。按集合名、
//! 文件名顺序读取，同名以先读到的为准；结构不合法的文件整份跳过。只读零写。

use std::path::PathBuf;

use anyhow::Result;
use serde_json::{Map, Value};

use super::model;
use crate::native::Context;

/// 单个别名文件的读取上限：几条快捷命令远小于此。
const MAX_ALIAS_FILE_BYTES: u64 = 64 * 1024;

/// 一条来源推荐（取值已按该文件的 timezone/lang 解析）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceAlias {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) collection: String,
}

pub(crate) fn load(ctx: &Context) -> Result<Vec<SourceAlias>> {
    let meta = crate::native_skills::load_metadata(ctx)?;
    let mut out: Vec<SourceAlias> = Vec::new();
    for coll in meta.collections.values() {
        if coll.deleted || !coll.enabled {
            continue;
        }
        let root = crate::native_skills::collection_source_root(ctx, coll);
        let Ok(entries) = std::fs::read_dir(root.join("aliases")) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        files.sort();
        for path in files {
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            // 符号链接不跟：来源里的文件必须就是来源自己的
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.len() > MAX_ALIAS_FILE_BYTES {
                continue;
            }
            let Some(obj) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .and_then(|value| value.as_object().cloned())
            else {
                continue;
            };
            if !valid_file(&obj) {
                continue;
            }
            for (name, value) in model::source_aliases(&obj) {
                if !out.iter().any(|a| a.name == name) {
                    out.push(SourceAlias {
                        name,
                        value,
                        collection: coll.name.clone(),
                    });
                }
            }
        }
    }
    Ok(out)
}

/// 来源文件与旧来源同一套结构约束，另要求真有 aliases 对象。
fn valid_file(obj: &Map<String, Value>) -> bool {
    obj.get("aliases").is_some_and(Value::is_object) && model::validate_source(obj).is_ok()
}
