//! 长操作的进度：按运行编号写一份小文件到系统临时目录，界面用只读的 `runs:progress` 轮询。
//!
//! 只写系统临时目录（不写数据目录、不写库），所以只读的预览也能报进度而不破坏「查询零写」。
//! 进度只是提示：写不进去就算了，不影响操作本身；操作结束时（无论成败）删掉。

use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn dir() -> PathBuf {
    std::env::temp_dir().join("agent-manager-progress")
}

fn path_of(run_id: &str) -> Option<PathBuf> {
    let valid = !run_id.is_empty()
        && run_id.len() <= 64
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    valid.then(|| dir().join(format!("{run_id}.json")))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 一次操作的步骤清单：先声明全部步骤，再逐个推进。
pub(crate) struct Progress {
    path: Option<PathBuf>,
    started: u64,
    /// (键, 显示名, 状态 pending|active|done|failed|skipped, 细节)
    steps: Vec<(String, String, &'static str, Option<String>)>,
}

impl Progress {
    pub(crate) fn new(run_id: &str, steps: &[(&str, &str)]) -> Progress {
        let progress = Progress {
            path: path_of(run_id),
            started: now_secs(),
            steps: steps
                .iter()
                .map(|(key, label)| (key.to_string(), label.to_string(), "pending", None))
                .collect(),
        };
        progress.flush();
        progress
    }

    fn flush(&self) {
        let Some(path) = &self.path else { return };
        let body = json!({
            "started_at": self.started,
            "steps": self.steps.iter().map(|(key, label, state, detail)| json!({
                "key": key, "label": label, "state": state, "detail": detail,
            })).collect::<Vec<_>>(),
        });
        let _ = std::fs::create_dir_all(dir());
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, body.to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    fn set(&mut self, key: &str, state: &'static str, detail: Option<String>) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.0 == key) {
            step.2 = state;
            if detail.is_some() || state != "active" {
                step.3 = detail;
            }
        } else {
            // 没声明过的步骤（如失败后的恢复）追加在最后
            self.steps
                .push((key.to_string(), key.to_string(), state, detail));
        }
        self.flush();
    }

    /// 开始一步；之前还在进行中的步骤一并标为完成。
    pub(crate) fn start(&mut self, key: &str, detail: Option<String>) {
        for step in self.steps.iter_mut() {
            if step.2 == "active" && step.0 != key {
                step.2 = "done";
            }
        }
        self.set(key, "active", detail);
    }

    pub(crate) fn detail(&mut self, key: &str, detail: String) {
        self.set(key, "active", Some(detail));
    }

    /// 加一个显示名明确的步骤（如失败后的「恢复原样」）并开始它。
    pub(crate) fn add_and_start(&mut self, key: &str, label: &str) {
        for step in self.steps.iter_mut() {
            if step.2 == "active" {
                step.2 = "failed";
            }
        }
        self.steps
            .push((key.to_string(), label.to_string(), "active", None));
        self.flush();
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// `runs:progress --run <编号>`：只读；没有进行中的操作时为 null。
pub(crate) fn read(run_id: &str) -> Value {
    let Some(path) = path_of(run_id) else {
        return Value::Null;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Value::Null;
    };
    let mut value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if let Some(started) = value.get("started_at").and_then(Value::as_u64) {
        value["elapsed_sec"] = json!(now_secs().saturating_sub(started));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 进度按步骤推进_结束即删_坏编号不写() {
        let id = format!("test-progress-{}", std::process::id());
        {
            let mut p = Progress::new(&id, &[("a", "第一步"), ("b", "第二步")]);
            assert_eq!(read(&id)["steps"][0]["state"], "pending");
            p.start("a", None);
            p.detail("a", "1/3".into());
            let now = read(&id);
            assert_eq!(now["steps"][0]["state"], "active");
            assert_eq!(now["steps"][0]["detail"], "1/3");
            p.start("b", None);
            let now = read(&id);
            assert_eq!(now["steps"][0]["state"], "done");
            assert_eq!(now["steps"][1]["state"], "active");
            p.add_and_start("rollback", "恢复原样");
            let now = read(&id);
            assert_eq!(now["steps"][1]["state"], "failed");
            assert_eq!(now["steps"][2]["label"], "恢复原样");
        }
        assert_eq!(read(&id), Value::Null, "结束后删掉");
        let _ = Progress::new("../escape", &[("a", "x")]);
        assert_eq!(read("../escape"), Value::Null);
    }
}
