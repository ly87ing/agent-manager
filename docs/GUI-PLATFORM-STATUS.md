# GUI 平台状态

本页以当前分支的真实 Rust/Tauri 实现为基准；只有已接线并经过对应验证的能力才标记为完成。

| 范围 | 状态 |
| --- | --- |
| SQLite 核心 | 已完成：`native-manager.db`、settings、policy、runs 和读写边界 |
| CLI / Tauri 共用库 | 已完成：两者调用同一 `engine-rs` |
| Tauri bridge / React 基础 | 已完成：原生桥接、设置与 policy 页面、主要资源页面布局和直接 apply 交互 |
| 精品化体验（2026-09-17） | 已完成：设计令牌与浅色/深色/跟随系统主题（同步原生标题栏）；系统托盘驻留、窗口位置尺寸记忆、单实例唤回；页面与弹窗动效、骨架屏、组件化空态；助手/技能/MCP/操作记录搜索、⌘,/⌘F 快捷键、设置入口更新提示角标。验收见 [交付回执](goal-rust-gui.md)；2026-09-21 已在隔离数据目录与 DEV mock 下实点同源 Tauri 原生托盘，验证接入项唤回窗口、目标直达、显式确认及状态刷新。Dock Reopen、关窗驻留与安装包托盘仍未做真机 GUI 验收 |
| Python 清理 | 已完成：根仓库不再依赖 Python 或 `manage.sh` |
| Agent 原生域 | 已完成：定义、实际来源、状态、apply、跨进程锁、结果核验和操作记录 |
| Skills 原生域 | 已完成：已接入 dispatcher；内置默认只读、写入进 SQLite；集合与单技能支持更新/启停/移除；内容快照与安全软连接；status 只读零写与 apply 排他锁/审计 |
| Runs 操作记录与清理 | 已完成：列表与详情读取 SQLite `operation_logs`；新增操作记录时按设置（`log_retention_days`，0 为不清理）删除过期记录；提供 `runs:clear` 二次确认手动清空（清空自身零审计且不删库改配置） |
| 本地结构化诊断日志与清理 | 已退役（2026-09-19 裁决）：JSONL 诊断日志、`logs:clear` 与 envelope `log_file` 一并移除；错误消息出库前仍统一脱敏，排查依赖 `operation_logs` |
| MCP 原生域 | 2026-09-22 改为「实际条目为主」，待主审验收：`mcp:status` 直接列出各助手配置文件里的 stdio 与远程条目（脱敏、原样保留未建模字段、给出条目摘要），`mcp:apply` 逐条 add/edit/enable/disable/remove/copy/forget，写前按摘要 CAS、外部修改报「已在外部修改」；Codex/OpenCode 用官方 `enabled` 停用，Claude 停用移出条目并存快照（疑似凭据转存外部密钥文件）；旧服务库与分配表在首次写入时迁移，迁移不改配置文件；status 只读零写，apply 具备锁、审计、回读与条件回滚 |
| Rules 原生域 | 已完成（2026-09-22 改为实际文件为准）：`rules:status` 输出主 Codex/Claude 与各实例规则文件的实际全文与写入基准；`rules:apply` 按 `--base` 直接编辑，或应用模板/内置推荐/复制另一目标（覆盖前回填备份）；模板存 `rule_templates`；status 只读零写；apply 具备排他锁、审计与 CAS/回滚，外部修改时拒绝并提示刷新 |
| Hooks 原生域 | 已完成：Rust dispatcher 已接入 `hooks:status` / 真实 `hooks:apply`；hooks 专用 `sources:get/set`（hooks.claude/hooks.codex）revision CAS 只写 SQLite；固定 Claude `~/.claude/settings.json` signature-set 合并与 Codex `~/.codex/hooks.json` whole-file 两目标，外部修改 fail-closed |
| 定义编辑原文保存 | 已完成（`5ae995b`）：SourceDefinitionsPanel 禁用文本框自动替换并有单测，Rules/Hooks 原文保存不被 smart quotes 等自动改写 |
| 系统审计 | 已完成：Rust dispatcher 已接入只读 `system:audit`；聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态 |
| 密钥状态 | 已完成：`secrets:status` 只读输出变量名、来源与布尔状态；用户显式选择权限为 `600` 的外部 `.env` 文件，数据库只存路径，`secrets:set` 与 MCP 停用时的密钥转存只写文件，配置备份不含路径或取值；历史数据库值需显式确认迁移，写入成功后才删旧表；变量行来自各助手实际 MCP 条目与停用快照，字面量不列出、不回显 |
| Aliases 原生域 | 已完成：Rust dispatcher 已接入 `aliases:status` / 真实 `aliases:apply`；固定 zshrc/zprofile 两目标，只换托管区块、外部修改 fail-closed |
| 设置页配置工具 | 已完成：保留客户端诊断、Aliases 与 Secrets 入口，更新中心并入总览、Policy 原文编辑器撤销；Tools 域、快照（`snapshots:*`）与 `system:converge` 已退役，相关页面、Tauri 白名单与契约 schema 已移除；`system:refresh-upstream` 已接线（探测宇宙 agents/mcp/skills，缓存落 SQLite `upstream_probes` 表） |
| 独立包真实业务 E2E | 主要场景已完成（2026-09-15 主审独立验收，绑定验收提交 `5a110be3`）：Darwin x86_64 DMG verify 脚本 exit 0、包内构建身份 commit/date 与源码精确一致、未签名，SHA256 `076d16b098ede8915802c0bc6dceec42dc06e0dcadddd137e7865681c7e90a68`，7632606 bytes；真实 GUI E2E（旧版 plan→confirm→apply 交互，现已失效，不证明本轮直接 apply 新交互）覆盖 system:audit 缺库零写、Rules 取消零写/应用、Hooks 双目标、Aliases 双目标、MCP stdio+远程 HTTP 的 plan/confirm/apply、Secrets 仅显示变量名与存在性、Runs、外部修改 plan fail-closed；MCP 定义经共用 Rust CLI 写入隔离 SQLite 测试夹具，MCP 新增服务表单因 macOS 自动化输入限制未验收（不以测试夹具结果替代表单通过）；（旧版 dry-run 证据）stdio dry-run 数据库与目标文件哈希不变、日志不落库，远程 header 以环境变量引用写入 Codex 目标、合成值不出现在目标/诊断/操作日志。历史（2026-09-14）：基于 `5ae995b` 的 DMG 曾通过独立包验收（SHA256 `71a386d6ab8e1a7f85ef59c82f379411e9aa1dccaa3c3166309b96b971fce26b`，7632599 bytes，非 HEAD 构建包），当时 GUI 业务 E2E 因宿主 AX/computer-use permission_denied 未启动 |
| 跨平台发布、签名、更新 | 源码管线已就绪但外部/真实验收未完成：`14e6790` 重构后的 gui-release.yml 采用草稿先行单写者编排——前置 create-release job 统一校验 tag/checkout/版本一致性并创建唯一 draft Release，macOS arm64+x86_64、Linux x86_64、Windows x86_64 四腿矩阵（app/dmg、AppImage/deb、NSIS/MSI）以 releaseId 模式只向该草稿上传产物，收尾 updater-json job 在全矩阵完成后单写 latest.json（任一平台产物或签名缺失即失败，不上传残缺 latest.json）；支持 prerelease tag（由 tag 后缀推导）；每腿生成并上传 SHA256 checksums；engine-rs 与应用版本一致性门禁；verify 脚本在已签名分支补充公证票据如实检查；含 updater 签名、Apple/Windows 签名及 Apple 公证接线，缺密钥/占位符/脏树/来源异常 fail-closed，临时证书清理，Release 固定 draft；`9f56f00` 已将 Windows 引擎与应用升级为阻断 CI（cargo fmt/test/check、engine release build、NSIS/MSI build，无 continue-on-error）。2026-09-15 复测外部状态：GitHub Actions secrets=0、variables=0、本机有效 codesigning identities=0、GitHub Releases=0；GUI CI run 34936884572（该 run 绑定提交 `5a110be3`）六个 jobs 均 steps=[]，因账户付款失败或 spending limit 在启动前失败；arm64 DMG 已交叉构建但仅静态验收（SHA256 `a9540a2690b9d2ede16ef627f310e277ca49130e608c518b876aeaed0bbfd3f8`，7298163 bytes，thin arm64，adhoc linker-signed，未在 arm64 设备运行）；Developer ID 签名、公证、Linux/Windows 真实包、各平台真机运行与安装、updater 真实签名下载安装回读均未验证；未创建 tag、GitHub Release 或对外安装包 |

本轮发布就绪打磨（`d46b85a`、`47702ed`、`14e6790`）：补齐 MIT LICENSE 与公开发布元数据、启用显式 Webview CSP、更新应用图标、Bundle ID 定为 `io.github.ly87ing.agent-manager`；前端增加顶层错误边界、自定义助手安装/升级确认与 MCP 空态引导，并清理退役代码；发布管线重构内容见上表"跨平台发布、签名、更新"行。

缺库读取不建库，保存才初始化数据库；设置在 `app_settings` 表。policy 持久化来源为 `sqlite`，写入参数固定 `--layer local`。Claude Code 与 Codex 默认均为官方 `native` 命令，brew 仅按实际检测来源处理。

公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝；status/query 只读零写；六域写操作仅通过真实 apply 完成，apply 内部保留 preflight/写前校验、精确 scope、锁、外部修改检测、CAS/条件回滚与审计。

MCP 支持本地 stdio 与远程 HTTP 两种传输；条目以各助手配置文件为准，新增或修改远程地址时仅允许 `https://` 或本机回环 `http://localhost` / `http://127.0.0.1`，headers 只接受 `${VAR}` 引用或保留原值，以客户端原生引用形式写入。界面按 `mcp:status` 的 `targets` 列出全部配置了 MCP 的助手（含实例与 OpenCode）。

旧 DMG/旧 GUI E2E 不证明本轮新交互。本轮源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收已完成；最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify（提交、SHA256 与验证结果见 [交付回执](goal-rust-gui.md)）。

验证入口：`cargo test --manifest-path engine-rs/Cargo.toml`、`cargo test --manifest-path app/src-tauri/Cargo.toml`，以及 `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`。macOS 打包与独立包验收使用 `pnpm --dir app run build:macos` 与 `./scripts/verify-macos-dmg.sh <path-to-dmg>`。本机验收允许如实报告未签名状态，不代表 Apple 签名或公证通过，也不替代其他平台和未完成域的 E2E。
