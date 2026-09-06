# GUI 平台支持现状（Phase 3.5 据实报告）

> 本文是 `docs/goal-rust-gui.md` Phase 3「平台扩展」的诚实报告：
> 只宣称已验证的范围，未验证项明示。冲突时以提案 §6.3 Phase 3 为准。

## 平台支持矩阵

| 平台 | 构建验证 | 引擎契约验证 | 运行时验证 | 状态 |
|---|---|---|---|---|
| macOS (本机 arm64) | `pnpm tauri build --bundles app` 产出 .app（实测） | engine-rs `cargo test` + `tests.test_engine_rs_parity` / `tests.test_engine_cli` python 双跑 parity（实测） | GUI `system.converge` ≡ `make update` 产物逐字节 diff 为空（Phase 2 真实机验收） | **全量实测通过** |
| Linux (ubuntu-latest) | CI 构建：typecheck + vitest + `pnpm tauri build --bundles deb`（run 34008157126 绿）；安装器 appimage/deb 由 `gui-release.yml` 构建 | CI 回归：engine-rs `cargo test` / `cargo build --release` + python parity 双跑（同 run 绿） | **未在真实 Linux 桌面环境运行验证** | CI 回归通过，运行时未实测 |
| Windows (windows-latest) | CI 实验性 job（`continue-on-error: true`，按 goal 任务书 Phase 3「据实报告」条款）：typecheck + vitest + `pnpm tauri build --bundles nsis`（run 34008157126 绿，含 nsis 安装器产出） | 未在 CI 跑 parity（python sidecar 路径未验证） | **未实测**：引擎 sidecar 进程编排、路径处理在 Windows 上的运行时行为均未验证；既定过渡方案为 sidecar 捆绑 python-build-standalone（提案 §6.3 Phase 3 第 1 条），尚未实施 | **实验性，不宣称可用**（构建与单测 CI 绿、含 nsis 安装器产出；运行时未实测） |

## CI 工作流结构

- `.github/workflows/ci.yml`（既有，未改动）：Python 3.9/3.10/3.11 × ubuntu/macos 矩阵，全量 unittest + `make smoke`。
- `.github/workflows/gui-ci.yml`（新增，push 到 main/rust、PR、手动触发）：
  - `engine-rs`（ubuntu + macos）：rust toolchain、`cargo test`、`cargo build --release`、python parity（`tests.test_engine_rs_parity` + `tests.test_engine_cli`）。
  - `app`（ubuntu + macos）：pnpm install、typecheck、vitest、`pnpm tauri build --bundles` 按平台分派（macos→app、ubuntu→deb）；ubuntu 前置安装 tauri 官方 Linux 系统依赖（webkit2gtk-4.1 等）。
  - `app-windows`（windows，`continue-on-error: true`，实验性）：同上，`--bundles nsis`。
  - 缓存：rustup/cargo（`~/.rustup`、`~/.cargo/registry`、`~/.cargo/git`、target 目录）与 pnpm store，均走 `actions/cache@v4`。
- `.github/workflows/gui-release.yml`（新增，tag `v*` 触发）：tauri-action 矩阵（macos → app/dmg，ubuntu → appimage/deb，windows → nsis/msi），产全量安装器并上传到该 tag 的 release。

## 自更新（tauri-plugin-updater）接线说明

当前状态：**骨架已接好，密钥为占位符，不会真正执行更新**。
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
