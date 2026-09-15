# GUI 平台状态

本页以当前分支的真实 Rust/Tauri 实现为基准；只有已接线并经过对应验证的能力才标记为完成。

| 范围 | 状态 |
| --- | --- |
| SQLite 核心 | 已完成：`native-manager.db`、settings、policy、runs 和读写边界 |
| CLI / Tauri 共用库 | 已完成：两者调用同一 `engine-rs` |
| Tauri bridge / React 基础 | 已完成：原生桥接、设置与 policy 页面、主要资源页面布局和 plan/apply 交互 |
| Python 清理 | 已完成：根仓库不再依赖 Python 或 `manage.sh` |
| Agent 原生域 | 已完成：定义、实际来源、状态、plan/apply、跨进程锁、结果核验和操作记录 |
| Skills 原生域 | 已完成：已接入 dispatcher；内置默认只读、写入进 SQLite；集合与单技能支持更新/启停/移除；内容快照与安全软连接；零写预览与 apply 排他锁/审计 |
| Runs 操作记录与清理 | 已完成：列表与详情读取 SQLite `operation_logs`；新增操作记录时按设置（`log_retention_days`，0 为不清理）删除过期记录；提供 `runs:clear` 二次确认手动清空（清空自身零审计且不删库改配置） |
| 本地结构化诊断日志与清理 | 已完成：`data_dir/logs/agent-manager-YYYY-MM-DD.jsonl` 日切写入；仅记录脱敏元数据；Unix 权限 0700/0600；只读与演练零写；诊断日志写入后及应用启动时按 `log_retention_days` 清理过期文件；提供 `logs:clear` 原生命令并在设置页支持二次确认手动清空（不碰 SQLite 任务记录） |
| MCP 原生域 | 已完成：Rust dispatcher 已接入本地 stdio `mcp:status` / `mcp:plan` / `mcp:apply`；定义与分配存 SQLite；支持 Claude JSON、Codex TOML、OpenCode JSON（需配置对应 Agent target）；逐助手 add/update/enable/disable/remove，库级 enable/disable/remove；status/plan/dry-run 零写；apply 具备锁、审计、CAS/回滚；env 在 SQLite definition/binding 中只持久化字面量 `${VAR}` 形式的环境变量引用；status、plan、诊断与操作日志不展示或保存解析后的真实值；真实 apply 执行时从进程环境解析，并按目标 codec 写入 Claude、Codex 或 OpenCode 的目标客户端配置，目标文件的明文由文件权限与客户端安全边界保护 |
| Rules 原生域 | 已完成：Rust dispatcher 已接入 `rules:status` / `rules:plan` / `rules:apply`；固定 Codex/Claude 两目标；规则专用 `sources:get/set`（rules.shared/rules.codex/rules.claude）使用 revision CAS 只写 SQLite；status/plan/dry-run 零写；apply 具备排他锁、审计与 CAS/回滚，外部修改 fail-closed |
| Hooks 原生域 | 已完成：Rust dispatcher 已接入 `hooks:status` / `hooks:plan` / `hooks:apply`；hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）revision CAS 只写 SQLite；固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed |
| 定义编辑原文保存 | 已完成（`5ae995b`）：SourceDefinitionsPanel 禁用文本框自动替换并有单测，Rules/Hooks 原文保存不被 smart quotes 等自动改写 |
| 系统审计 | 已完成：Rust dispatcher 已接入只读 `system:audit` 及其 Tauri offline 桥接；聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态 |
| 密钥状态 | 已完成：只读 `secrets:status` 已接入 Rust dispatcher；仅读取 SQLite 有效策略中已启用 MCP 定义的合法 `${VAR}` 环境变量占位符，只检查 Agent Manager 当前进程环境是否存在对应变量，输出变量名与布尔；不扫描、定位或克隆外部密钥仓库，不持久化、序列化或显示变量值；畸形定义固定脱敏失败 |
| Aliases 原生域 | 已完成：Rust dispatcher 已接入 `aliases:status` / `aliases:plan` / `aliases:apply`；固定 zshrc/zprofile 两目标，只换托管区块、外部修改 fail-closed |
| 设置页高级管理 | 已完成（2026-09-12 裁决）：Aliases、更新中心与 Policy 页可达；Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 已接线（探测宇宙 agents/mcp/skills，缓存落 SQLite `upstream_probes` 表） |
| 独立包真实业务 E2E | 部分完成：2026-09-14 基于 `5ae995b` 的干净 Darwin x86_64 DMG 已通过主审独立验收（只读挂载、仓库外复制、架构与版本、无 Python、源码与 HOME 路径脱敏、从 `/` 隔离环境 5 秒存活，包内构建身份 commit/date 与源码一致；SHA256 `71a386d6ab8e1a7f85ef59c82f379411e9aa1dccaa3c3166309b96b971fce26b`，7632599 bytes）；后续 `a2cc864`/`9f56f00` 仅改工作流，该包仍代表当前产品代码（非仓库 HEAD 构建包）。真实业务 E2E 未完成：宿主 AX/computer-use 对 Agent Manager 与 Finder 均 permission_denied，8 个业务场景未启动，隔离 HOME/data 无 DB 或目标写入，属验收基础设施阻塞，非产品通过或失败 |
| 跨平台发布、签名、更新 | 源码管线已就绪但外部/真实验收未完成：`a2cc864` 的 gui-release.yml 覆盖 macOS arm64+x86_64、Linux x86_64、Windows x86_64（app/dmg、AppImage/deb、NSIS/MSI），含 updater 签名与 latest.json、Apple/Windows 签名及 Apple 公证接线，缺密钥/占位符/脏树/来源异常 fail-closed，临时证书清理，Release 固定 draft；`9f56f00` 已将 Windows 引擎与应用升级为阻断 CI（cargo fmt/test/check、engine release build、NSIS/MSI build，无 continue-on-error）。GitHub Actions secrets=0、variables=0、本机有效 codesigning identities=0，远程 GUI CI 因账户付款失败未实际运行；签名、公证、arm64/Linux/Windows 真实包与更新发布均未验证；未创建 tag、GitHub Release 或对外安装包 |

缺库读取不建库，保存才初始化数据库；设置在 `app_settings` 表。policy 持久化来源为 `sqlite`，写入参数固定 `--layer local`。Claude Code 与 Codex 默认均为官方 `native` 命令，brew 仅按实际检测来源处理。

MCP 支持本地 stdio 与远程 HTTP 两种传输（字段互斥：远程定义含 `url`，仅允许 `https://` 或本机回环 `http://localhost` / `http://127.0.0.1`，可选 `headers` 仅接受 `${VAR}` 占位符，以引用形式写入目标，明文不落盘）。OpenCode 使用 OpenCode JSON codec，必须为对应 Agent 配置 `mcp_config` 目标。

验证入口：`cargo test --manifest-path engine-rs/Cargo.toml`、`cargo test --manifest-path app/src-tauri/Cargo.toml`，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。macOS 打包与独立包验收使用 `pnpm --dir app run build:macos` 与 `./scripts/verify-macos-dmg.sh <path-to-dmg>`。本机验收允许如实报告未签名状态，不代表 Apple 签名或公证通过，也不替代其他平台和未完成域的 E2E。
