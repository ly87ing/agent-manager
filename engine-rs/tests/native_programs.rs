//! 程序域：技能与 MCP 依赖的程序（docs/PROGRAM-LIFECYCLE.md）。
//!
//! 每个用例一个隔离 HOME 与数据目录，经 CLI 子进程执行（用户 PATH 由 AGENT_MANAGER_USER_PATH
//! 给定，避免并行用例互相影响进程环境）。npm 用一个假脚本代替：按参数在前缀里摆出包目录与命令
//! 链接，版本 `99.0.0` 模拟装不上；不联网。

use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const FAKE_NPM: &str = r#"#!/bin/sh
# 假 npm：view 回最新版本；install 在前缀下摆出包与命令链接
cmd="$1"; shift
if [ "$cmd" = "view" ]; then echo "${FAKE_NPM_LATEST:-2.0.0}"; exit 0; fi
[ "$cmd" = "install" ] || exit 2
global=0; prefix=""; spec=""
while [ $# -gt 0 ]; do
  case "$1" in
    -g) global=1 ;;
    --prefix|--cache) [ "$1" = "--prefix" ] && prefix="$2"; shift ;;
    -*) ;;
    *) spec="$1" ;;
  esac
  shift
done
name="${spec%@*}"; ver="${spec##*@}"
[ "$ver" = "99.0.0" ] && { echo "npm error notarget No matching version found for $spec" >&2; exit 1; }
bin="${name##*/}"
if [ $global = 1 ]; then pkg="$prefix/lib/node_modules/$name"; else pkg="$prefix/node_modules/$name"; fi
mkdir -p "$pkg/bin"
printf '{"name":"%s","version":"%s","bin":{"%s":"bin/cli.js"}}' "$name" "$ver" "$bin" > "$pkg/package.json"
printf '#!/bin/sh\necho %s %s\n' "$bin" "$ver" > "$pkg/bin/cli.js"; chmod +x "$pkg/bin/cli.js"
if [ $global = 1 ]; then mkdir -p "$prefix/bin"; ln -sfn "../lib/node_modules/$name/bin/cli.js" "$prefix/bin/$bin"
else mkdir -p "$prefix/node_modules/.bin"; ln -sfn "../$name/bin/cli.js" "$prefix/node_modules/.bin/$bin"; fi
"#;

struct Env {
    base: PathBuf,
    home: PathBuf,
    data: PathBuf,
    user_path: String,
}

fn setup(tag: &str) -> Env {
    let base = std::env::temp_dir().join(format!(
        "am-programs-{tag}-{}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let home = base.join("home");
    let tools = base.join("tools");
    std::fs::create_dir_all(home.join(".local/bin")).unwrap();
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::write(tools.join("npm"), FAKE_NPM).unwrap();
    std::fs::set_permissions(tools.join("npm"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let user_path = format!(
        "{}:{}:{}:/usr/bin:/bin",
        home.join(".local/bin").display(),
        home.join(".npm-global/bin").display(),
        tools.display()
    );
    Env {
        data: base.join("data"),
        home,
        base,
        user_path,
    }
}

fn skill(env: &Env, name: &str, frontmatter_meta: &str) {
    let dir = env.home.join(".claude/skills").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: d\n{frontmatter_meta}---\n# {name}\n"),
    )
    .unwrap();
}

/// 用假 npm 在「全局」前缀里装一个包（模拟用户自己 npm install -g 过）。
fn global_install(env: &Env, spec: &str) {
    let status = Command::new(env.base.join("tools/npm"))
        .args([
            "install",
            "-g",
            "--prefix",
            env.home.join(".npm-global").to_str().unwrap(),
            spec,
        ])
        .status()
        .unwrap();
    assert!(status.success());
}

fn run(env: &Env, command: &str, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .arg(command)
        .args(args)
        .arg("--json")
        .env("HOME", &env.home)
        .env("AGENT_MANAGER_DATA_DIR", &env.data)
        .env("AGENT_MANAGER_USER_PATH", &env.user_path)
        .env_remove("AGENT_MANAGER_LOCK_HELD")
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        panic!(
            "{command} 输出不是 JSON：{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn element<'a>(status: &'a Value, id: &str) -> &'a Value {
    status["data"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["program"] == id)
        .unwrap_or_else(|| panic!("缺程序 {id}：{status}"))
}

fn codes(element: &Value) -> Vec<String> {
    element["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["code"].as_str().unwrap().to_string())
        .collect()
}

fn version_at(dir: &Path) -> String {
    let pkg: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap()).unwrap();
    pkg["version"].as_str().unwrap().to_string()
}

fn tree_digest(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap().display().to_string();
            let meta = p.symlink_metadata().unwrap();
            if meta.file_type().is_symlink() {
                out.push(format!(
                    "{rel} -> {}",
                    std::fs::read_link(&p).unwrap().display()
                ));
            } else if meta.is_dir() {
                out.push(format!("{rel}/"));
                walk(&p, root, out);
            } else {
                out.push(format!(
                    "{rel} = {}",
                    std::fs::read_to_string(&p).unwrap_or_default()
                ));
            }
        }
    }
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// 查询：认出技能声明（简写与 OpenClaw）、在 PATH 上的归属与全部命令、缺失、候选；零写。
#[test]
fn status_reads_declarations_and_locations_without_writing() {
    let env = setup("status");
    skill(
        &env,
        "wecom",
        "metadata:\n  requires:\n    bins: [\"tool-a\"]\n",
    );
    skill(&env, "fresh", "metadata:\n  openclaw:\n    requires:\n      bins: [tool-b]\n    install:\n      - kind: node\n        package: tool-b\n        bins: [tool-b]\n");
    std::fs::write(
        env.home.join(".claude/skills/fresh/SKILL.md"),
        format!(
            "{}\n```sh\ncommand -v tool-a\n```\n",
            std::fs::read_to_string(env.home.join(".claude/skills/fresh/SKILL.md")).unwrap()
        ),
    )
    .unwrap();
    global_install(&env, "tool-a@1.2.0");
    let home_before = tree_digest(&env.home);
    let status = run(&env, "programs:status", &[]);
    assert_eq!(status["ok"], true, "{status}");
    let a = element(&status, "npm:tool-a");
    assert_eq!(a["mode"], "delegated");
    assert_eq!(a["owner"], "npm-global");
    assert_eq!(a["version"], "1.2.0");
    assert_eq!(a["dependents"][0]["name"], "wecom");
    assert_eq!(a["commands"]["upgrade"], "npm install -g tool-a@latest");
    let b = element(&status, "npm:tool-b");
    assert_eq!(b["mode"], "missing");
    assert_eq!(codes(b), vec!["missing"]);
    // fresh 正文里用到 tool-a 却没声明：作为 fresh 的候选（别的技能声明过也不代表 fresh 声明了）
    assert_eq!(status["data"]["suggestions"][0]["skill"], "fresh");
    assert_eq!(status["data"]["suggestions"][0]["bin"], "tool-a");
    assert_eq!(status["data"]["suggestions"].as_array().unwrap().len(), 1);
    assert!(status["data"]["user_path"]["entry_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "~/.local/bin"));
    assert_eq!(tree_digest(&env.home), home_before, "查询不改用户目录");
    assert!(!env.data.exists(), "查询不建数据目录");
    let _ = std::fs::remove_dir_all(&env.base);
}

/// 代管：改动前整包移到备份，用原包管理器换版本；失败原样恢复；回退零网络、逐字节回来。
#[test]
fn delegated_upgrade_restores_on_failure_and_rolls_back() {
    let env = setup("delegated");
    skill(
        &env,
        "wecom",
        "metadata:\n  requires:\n    bins: [\"tool-a\"]\n",
    );
    global_install(&env, "tool-a@1.2.0");
    let global = env.home.join(".npm-global");
    let original = tree_digest(&global);

    let preview = run(
        &env,
        "programs:preview",
        &[
            "--program",
            "npm:tool-a",
            "--action",
            "upgrade",
            "--version",
            "1.3.0",
        ],
    );
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(
        preview["data"]["manual"].as_array().unwrap().len(),
        1,
        "自己动手只需一条 npm 命令：{preview}"
    );
    assert!(preview["data"]["manual"][0]
        .as_str()
        .unwrap()
        .contains("--ignore-scripts tool-a@1.3.0"));
    assert_eq!(tree_digest(&global), original, "预览不动全局安装");

    let failed = run(
        &env,
        "programs:apply",
        &[
            "--program",
            "npm:tool-a",
            "--action",
            "upgrade",
            "--version",
            "99.0.0",
        ],
    );
    assert_eq!(failed["ok"], false);
    assert!(failed["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("No matching version"));
    assert_eq!(tree_digest(&global), original, "失败后原样恢复");
    assert_eq!(
        run(&env, "programs:status", &[])["data"]["backups"],
        serde_json::json!([]),
        "失败不留空备份"
    );

    let ok = run(
        &env,
        "programs:apply",
        &[
            "--program",
            "npm:tool-a",
            "--action",
            "upgrade",
            "--version",
            "1.3.0",
        ],
    );
    assert_eq!(ok["ok"], true, "{ok}");
    assert_eq!(ok["lock"]["held"], true);
    assert_eq!(version_at(&global.join("lib/node_modules/tool-a")), "1.3.0");
    let status = run(&env, "programs:status", &[]);
    let backup = status["data"]["backups"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(status["data"]["backups"][0]["version"], "1.2.0");

    let restored = run(
        &env,
        "programs:apply",
        &[
            "--program",
            "npm:tool-a",
            "--action",
            "restore",
            "--backup",
            &backup,
        ],
    );
    assert_eq!(restored["ok"], true, "{restored}");
    assert_eq!(tree_digest(&global), original, "回退后与原来逐字节一致");
    let _ = std::fs::remove_dir_all(&env.base);
}

/// 纳入与撤回：任何时刻 PATH 上至少一份；纳入后只剩托管的一份；撤回原样恢复、登记清干净。
/// 入口目录有别人的同名文件时拒绝，一个字节都不动。
#[test]
fn adopt_and_undo_keep_one_copy_and_refuse_conflicts() {
    let env = setup("adopt");
    skill(
        &env,
        "wecom",
        "metadata:\n  requires:\n    bins: [\"tool-a\"]\n",
    );
    global_install(&env, "tool-a@1.2.0");
    let global = env.home.join(".npm-global");
    let original = tree_digest(&global);

    // 冲突：入口目录已有别人的 tool-a
    std::fs::write(env.home.join(".local/bin/tool-a"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(
        env.home.join(".local/bin/tool-a"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let status = run(&env, "programs:status", &[]);
    assert!(codes(element(&status, "npm:tool-a")).contains(&"multiple".to_string()));
    let refused = run(
        &env,
        "programs:apply",
        &[
            "--program",
            "npm:tool-a",
            "--action",
            "adopt",
            "--entry-dir",
            "~/.local/bin",
        ],
    );
    assert_eq!(refused["ok"], false);
    assert!(refused["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("已经有别的文件"));
    assert_eq!(tree_digest(&global), original);
    assert_eq!(
        std::fs::read_to_string(env.home.join(".local/bin/tool-a")).unwrap(),
        "#!/bin/sh\n"
    );
    std::fs::remove_file(env.home.join(".local/bin/tool-a")).unwrap();

    let adopted = run(
        &env,
        "programs:apply",
        &[
            "--program",
            "npm:tool-a",
            "--action",
            "adopt",
            "--entry-dir",
            "~/.local/bin",
        ],
    );
    assert_eq!(adopted["ok"], true, "{adopted}");
    assert!(
        !global.join("lib/node_modules/tool-a").exists(),
        "全局那份移到备份"
    );
    let entry = env.home.join(".local/bin/tool-a");
    let out = Command::new(&entry).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "tool-a 1.2.0",
        "入口链接可用、同一版本"
    );
    let status = run(&env, "programs:status", &[]);
    let a = element(&status, "npm:tool-a");
    assert_eq!(a["mode"], "managed");
    assert_eq!(a["managed_versions"], serde_json::json!(["1.2.0"]));
    assert_eq!(
        a["dependents"][0]["name"], "wecom",
        "托管后仍认得是 wecom 需要的"
    );
    assert!(status["data"]["links"].get("~/.local/bin/tool-a").is_some());
    let backup = status["data"]["backups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["reason"] == "adopt")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let undone = run(
        &env,
        "programs:apply",
        &[
            "--program",
            "npm:tool-a",
            "--action",
            "unadopt",
            "--backup",
            &backup,
        ],
    );
    assert_eq!(undone["ok"], true, "{undone}");
    assert_eq!(tree_digest(&global), original, "撤回后全局安装逐字节恢复");
    assert!(entry.symlink_metadata().is_err(), "入口链接删掉");
    let status = run(&env, "programs:status", &[]);
    assert_eq!(element(&status, "npm:tool-a")["mode"], "delegated");
    assert_eq!(status["data"]["links"], serde_json::json!({}));
    let _ = std::fs::remove_dir_all(&env.base);
}

/// 托管：安装原本没有的程序（安装脚本默认不执行）、升级保留旧版、换版本、删旧版、不能删当前版、整个删除。
#[test]
fn managed_install_switch_and_remove() {
    let env = setup("managed");
    skill(&env, "fresh", "metadata:\n  openclaw:\n    requires:\n      bins: [tool-b]\n    install:\n      - kind: node\n        package: tool-b\n        bins: [tool-b]\n");
    let no_entry = run(
        &env,
        "programs:preview",
        &[
            "--program",
            "npm:tool-b",
            "--action",
            "install",
            "--version",
            "1.0.0",
        ],
    );
    assert_eq!(no_entry["ok"], false, "没选入口目录不能装");
    assert!(no_entry["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("入口目录"));

    let preview = run(
        &env,
        "programs:preview",
        &[
            "--program",
            "npm:tool-b",
            "--action",
            "install",
            "--version",
            "1.0.0",
            "--entry-dir",
            "~/.local/bin",
        ],
    );
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["data"]["scripts_ignored"], true);
    assert!(!env.data.join("programs").exists(), "预览不装");

    let installed = run(
        &env,
        "programs:apply",
        &[
            "--program",
            "npm:tool-b",
            "--action",
            "install",
            "--version",
            "1.0.0",
            "--entry-dir",
            "~/.local/bin",
        ],
    );
    assert_eq!(installed["ok"], true, "{installed}");
    let entry = env.home.join(".local/bin/tool-b");
    assert_eq!(
        String::from_utf8_lossy(&Command::new(&entry).output().unwrap().stdout).trim(),
        "tool-b 1.0.0"
    );
    let upgraded = run(
        &env,
        "programs:apply",
        &["--program", "npm:tool-b", "--action", "upgrade"],
    );
    assert_eq!(upgraded["ok"], true, "{upgraded}");
    assert_eq!(
        upgraded["data"]["target_version"], "2.0.0",
        "不给版本取最新"
    );
    assert_eq!(
        String::from_utf8_lossy(&Command::new(&entry).output().unwrap().stdout).trim(),
        "tool-b 2.0.0"
    );
    let status = run(&env, "programs:status", &[]);
    let b = element(&status, "npm:tool-b");
    assert_eq!(b["managed_current"], "2.0.0");
    let mut versions: Vec<&str> = b["managed_versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    versions.sort();
    assert_eq!(versions, vec!["1.0.0", "2.0.0"]);

    assert_eq!(
        run(
            &env,
            "programs:apply",
            &[
                "--program",
                "npm:tool-b",
                "--action",
                "switch",
                "--version",
                "1.0.0"
            ]
        )["ok"],
        true
    );
    assert_eq!(
        String::from_utf8_lossy(&Command::new(&entry).output().unwrap().stdout).trim(),
        "tool-b 1.0.0",
        "换版本只切指针"
    );
    let refused = run(
        &env,
        "programs:apply",
        &[
            "--program",
            "npm:tool-b",
            "--action",
            "remove",
            "--version",
            "1.0.0",
        ],
    );
    assert_eq!(refused["ok"], false, "不能删当前版本");
    assert_eq!(
        run(
            &env,
            "programs:apply",
            &[
                "--program",
                "npm:tool-b",
                "--action",
                "remove",
                "--version",
                "2.0.0"
            ]
        )["ok"],
        true
    );
    assert!(!env.data.join("programs/npm/tool-b/2.0.0").exists());

    let removed = run(
        &env,
        "programs:apply",
        &["--program", "npm:tool-b", "--action", "remove"],
    );
    assert_eq!(removed["ok"], true, "{removed}");
    assert!(entry.symlink_metadata().is_err(), "入口链接删掉");
    assert!(!env.data.join("programs/npm/tool-b").exists());
    let status = run(&env, "programs:status", &[]);
    assert_eq!(element(&status, "npm:tool-b")["mode"], "missing");
    assert_eq!(status["data"]["links"], serde_json::json!({}));
    let _ = std::fs::remove_dir_all(&env.base);
}

/// 补充声明与允许安装脚本只写库，查询读得回来；声明的命令进入依赖。
#[test]
fn declarations_are_saved_and_read_back() {
    let env = setup("declare");
    skill(&env, "bugloop", "");
    std::fs::write(
        env.home.join(".claude/skills/bugloop/SKILL.md"),
        "---\nname: bugloop\ndescription: d\n---\n```sh\ncommand -v tool-c\n```\n",
    )
    .unwrap();
    global_install(&env, "tool-c@3.0.0");
    let status = run(&env, "programs:status", &[]);
    assert_eq!(status["data"]["suggestions"][0]["bin"], "tool-c");
    assert_eq!(status["data"]["suggestions"][0]["found"], true);
    assert!(
        status["data"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["program"] != "npm:tool-c"),
        "没确认前不算依赖"
    );

    assert_eq!(
        run(
            &env,
            "programs:declare",
            &["--skill", "bugloop", "--bins", "[\"tool-c\"]"]
        )["ok"],
        true
    );
    assert_eq!(
        run(
            &env,
            "programs:declare",
            &["--program", "npm:tool-c", "--allow-scripts", "true"]
        )["ok"],
        true
    );
    let status = run(&env, "programs:status", &[]);
    let c = element(&status, "npm:tool-c");
    assert_eq!(c["dependents"][0]["source"], "user");
    assert_eq!(status["data"]["suggestions"], serde_json::json!([]));
    assert_eq!(
        status["data"]["allow_scripts"],
        serde_json::json!(["npm:tool-c"])
    );
    let preview = run(
        &env,
        "programs:preview",
        &[
            "--program",
            "npm:tool-c",
            "--action",
            "upgrade",
            "--version",
            "3.1.0",
        ],
    );
    assert!(
        !preview["data"]["manual"][0]
            .as_str()
            .unwrap()
            .contains("--ignore-scripts"),
        "允许后不再跳过安装脚本"
    );
    let _ = std::fs::remove_dir_all(&env.base);
}
