//! Windows 上的程序识别（只读）：npm 全局装的命令是 `<prefix>\<命令>.cmd` 转发脚本（cmd-shim），
//! 包在 `<prefix>\node_modules` 下；顺着脚本认出是哪个包、什么版本、归 npm 全局管。
#![cfg(windows)]

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

/// npm 生成的 cmd-shim（节选，保留定位目标的那一行）。
fn cmd_shim(rel: &str) -> String {
    format!(
        "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\
IF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\
endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\{rel}\" %*\r\n"
    )
}

#[test]
fn npm_全局的_cmd_shim_认得出包与版本_查询不写() {
    let base = std::env::temp_dir().join(format!("am-programs-win-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let data = base.join("data");
    // 模拟 %APPDATA%\npm：包在 node_modules 下，命令是同目录的 .cmd
    let prefix = base.join("npm");
    let pkg = prefix.join("node_modules").join("tool-a");
    std::fs::create_dir_all(pkg.join("bin")).unwrap();
    std::fs::write(
        pkg.join("package.json"),
        r#"{"name":"tool-a","version":"1.2.0","bin":{"tool-a":"bin/cli.js"}}"#,
    )
    .unwrap();
    std::fs::write(pkg.join("bin").join("cli.js"), "console.log('a')\n").unwrap();
    std::fs::write(
        prefix.join("tool-a.cmd"),
        cmd_shim(r"node_modules\tool-a\bin\cli.js"),
    )
    .unwrap();
    // 声明需要 tool-a 的技能
    let skill = home.join(".claude").join("skills").join("wecom");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: wecom\ndescription: d\nmetadata:\n  requires:\n    bins: [\"tool-a\"]\n---\n# wecom\n",
    )
    .unwrap();

    let user_path = format!(r"{};C:\windows\system32", prefix.display());
    let out = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .args(["programs:status", "--json"])
        .env("HOME", &home)
        .env("AGENT_MANAGER_DATA_DIR", &data)
        .env("AGENT_MANAGER_USER_PATH", &user_path)
        .env_remove("AGENT_MANAGER_LOCK_HELD")
        .output()
        .unwrap();
    let status: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("不是 JSON：{}", String::from_utf8_lossy(&out.stdout)));
    assert_eq!(status["ok"], true, "{status}");
    let a = status["data"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["program"] == "npm:tool-a")
        .unwrap_or_else(|| panic!("缺 npm:tool-a：{status}"))
        .clone();
    assert_eq!(a["owner"], "npm-global", "{a}");
    assert_eq!(a["mode"], "delegated");
    assert_eq!(a["version"], "1.2.0");
    assert_eq!(a["dependents"][0]["name"], "wecom");
    let path = a["installs"][0]["path"].as_str().unwrap().to_string();
    assert!(path.ends_with("tool-a.cmd"), "{path}");
    assert!(!data.exists(), "查询不建数据目录");
    let _ = std::fs::remove_dir_all(PathBuf::from(&base));
}
