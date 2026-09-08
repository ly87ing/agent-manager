# GUI 平台支持现状（Phase 3.5 据实报告）

> 本文是 `docs/goal-rust-gui.md` Phase 3「平台扩展」的诚实报告：
> 只宣称已验证的范围，未验证项明示。冲突时以提案 §6.3 Phase 3 为准。

## 平台支持矩阵

| 平台 | 构建验证 | 引擎契约验证 | 运行时验证 | 状态 |
|---|---|---|---|---|
| macOS（本机 Darwin x86_64，2026-09-08） | 已在 canonical 临时路径构建并启动 `.app`，资源脚本自动获取固定 SHA256 Python；Rust/Python 资源自测通过 | 总览、MCP 预览、演练可用；包内资源选择性 apply、日志、restore 通过；设置持久化与真实 Aliases 演练/apply/恢复通过 | 隔离 HOME/PATH 且禁止读取源码根；重启后 overlay/mtime 保留 | **standalone 已完成本地候选验收范围（未发布）** |
| Linux (ubuntu-latest) | 已配置 CI typecheck + vitest + Tauri 构建；本轮未运行 Linux 构建 | 已配置 engine-rs CI 回归；本轮未运行 | **未在真实 Linux 桌面环境运行验证** | 构建/引擎证据不等于桌面运行验收 |
| Windows (windows-latest) | CI 实验性 job（`continue-on-error: true`）：仅 typecheck + vitest；不产出安装器 | 未跑 parity | **不支持独立 Bash 运行链，未进行桌面运行验收**；过渡方案需捆绑 `python-build-standalone`，尚未实施 | **实验性，不宣称可用** |

## CI 工作流结构

- `.github/workflows/ci.yml`（既有，未改动）：Python 3.9/3.10/3.11 × ubuntu/macos 矩阵，全量 unittest + `make smoke`。
- `.github/workflows/gui-ci.yml`（新增，push 到 main/rust、PR、手动触发）：
  - `engine-rs`（ubuntu + macos）：rust toolchain、`cargo test`、`cargo build --release`、python parity（`tests.test_engine_rs_parity` + `tests.test_engine_cli`）。
  - `app`（ubuntu + macos）：pnpm install、typecheck、vitest、`pnpm tauri build --bundles` 按平台分派（macos→app、ubuntu→deb）；ubuntu 前置安装 tauri 官方 Linux 系统依赖（webkit2gtk-4.1 等）。
  - `app-windows`（windows，`continue-on-error: true`，实验性）：仅 typecheck + vitest，不产出安装器。
  - 缓存：rustup/cargo（`~/.rustup`、`~/.cargo/registry`、`~/.cargo/git`、target 目录）与 pnpm store，均走 `actions/cache@v4`。
- `.github/workflows/gui-release.yml`（新增，push 到已有 `v*` tag 或手动输入明确 tag）：先校验 `vX.Y.Z`、精确 tag 解析及 checkout 提交一致，再运行 tauri-action 矩阵（macos → app/dmg，ubuntu → appimage/deb），`projectPath: app`，Node 24，产物上传到指定 tag 的 release。Windows 尚未具备独立运行能力，因此不进入正式 release 矩阵。

本机当前为 Darwin x86_64（2026-09-08）；本轮 standalone 资源已完成构建脚本与定向契约测试，并通过严格隔离 3 项包测试（source deny、系统 Python deny、Homebrew deny、网络 deny、最小 PATH）。真实 `.app` 已在独立 HOME/data 与禁止源码读取的环境下启动，验证设置持久化（离线=true、默认演练=true、TTL=1177秒）及 Aliases 演练、apply、日志与恢复。最终本地回归：Python 751 项、Rust 引擎 42 项、Tauri 37 项及额外 4 项显式集成检查、前端 137 项通过；parity/CLI 112 项、类型检查、生产构建、smoke 和敏感信息扫描通过。当前属于本地候选未发布范围，当前未进行签名发布，未在远程 CI 运行本轮矩阵，不作全量发布可用性宣称。

## 自更新（tauri-plugin-updater）接线说明

当前状态：**设置持久化和手动更新确认流程已接好；未配置真实公钥或签名发布材料时不会发起更新检查，当前未进行签名发布**。
`app/src-tauri/tauri.conf.json` 的 `plugins.updater` 段：

- `endpoints`：`https://github.com/ly87ing/agent-manager/releases/latest/download/latest.json`
- `pubkey`：`YOUR_UPDATER_PUBKEY`（占位符；pubkey 仅在实际检查更新时才被解析，占位符不影响应用启动与构建）

Rust 侧已注册 `tauri-plugin-updater`（`Cargo.toml` + `main.rs`），前端已加
`@tauri-apps/plugin-updater` 依赖，capabilities 已放行 `updater:default`。

启用真实自更新的步骤（密钥永不进主仓库）：

1. 生成签名密钥对（一次性，本机执行）：
   `pnpm tauri signer generate -w ~/.tauri/agent-manager-updater.key`
   生成的私钥与密码只存外部私有 `secrets/` 仓库，**不写入本仓库任何文件**。
2. 把输出的公钥替换 `app/src-tauri/tauri.conf.json` 中 `YOUR_UPDATER_PUBKEY`。
3. 在 GitHub 仓库 Settings → Secrets 添加：
   - `TAURI_SIGNING_PRIVATE_KEY`：私钥文件内容；
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：私钥密码。
4. `gui-release.yml` 在 secret 存在时（步骤 `if` 门控）自动把
   `bundle.createUpdaterArtifacts` 置为 true，tauri-action 完成签名并生成
   `latest.json` 上传 release；secret 不存在时跳过签名，只产未签名安装器。
5. 密钥轮换：重新生成密钥对 → 更新 secrets 与 `pubkey` → 发新版本
   （旧版本客户端用旧公钥验签，新包须用旧私钥签名才能被旧客户端接受，
   轮换即意味着旧客户端停更）。

## 配置同步 GUI

配置同步页已于 Phase 2c 落地：`app/src/pages/SyncPage.tsx`（git backend 的
profile.status/pull/push，push 前 secret 扫描硬门禁 + 自动快照，冲突按 key
报告；双机同步与冲突解决验收见 `docs/goal-rust-gui.md` Phase 2 进度行）。
