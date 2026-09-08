# app/ — Agent Manager 桌面客户端

Tauri 2 壳 + React 18 前端，数据全部来自引擎契约命令（Contract v1 envelope）。
写路径统一编排（Phase 2c）：**plan 预览确认 → 域级 apply / system:converge（引擎单写者，
非 dry-run 前自动快照）**；skills 矩阵点击 = policy.set（落默认策略层）+ skills:apply 预览确认。
GUI 发起的动作带 `gui-` 前缀 run_id（经 AGENT_MANAGER_RUN_ID 透传），执行中心按 run_id 追踪日志。

## 命令

### 在 `app/` 目录运行（前端开发与桌面壳构建）

```bash
pnpm install
pnpm dev          # Vite 开发服务器（1420 端口；纯浏览器下引擎调用会失败，需经 Tauri 运行）
pnpm typecheck    # tsc --noEmit
pnpm test:unit run
pnpm build        # tsc --noEmit && vite build
pnpm tauri dev    # 桌面壳（需要 Rust 工具链）
RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=/YOUR_BUILD_HOME --remap-path-prefix=$PWD=/YOUR_SOURCE_ROOT" pnpm tauri build # 发行打包（含路径重映射，防止构建机路径泄漏）
cd src-tauri && cargo check && cargo test
```

## 独立运行（B 阶段）

发行态从 Tauri 的包资源目录 `resources/engine` 读取程序种子、内置 Python 和 Rust 引擎，
首次启动将程序种子初始化到稳定的用户数据目录 `engine/`。包内资源只读；用户数据目录保存
运行时配置、缓存、日志和快照。发行态不依赖源码仓库、系统 Python 或 Homebrew。

开发态可通过 `AGENT_MANAGER_ROOT=YOUR_AGENT_MANAGER_ROOT` 显式指定源码根；这只用于开发和
测试，不能作为发行包的资源回退路径。

### 在仓库根目录运行（资源构建与集成验收测试）

从仓库根构建独立资源（自动按真实 host 选择固定目标，下载官方
`python-build-standalone` 归档并校验 SHA256，同时构建 release Rust 引擎）：

```bash
python3 scripts/build-standalone-resources.py
```

构建依赖（Rust、Node、pnpm、Linux Tauri 系统库）只用于构建，不会被当作发行包运行时；
业务运行时依赖（外部 Agent、Node/git、远程仓库和用户凭据）仍由状态页明确显示，应用不会
隐式安装或同步它们。构建缓存和生成资源位于已忽略的 `app/.standalone-build/` 与
`app/src-tauri/resources/`。

资源侧单项契约测试（使用临时目录和 mock 下载，不启动真实 Agent）：

```bash
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/agent-manager-test.XXXXXX")"
HOME="$TEST_HOME" PATH="$(dirname "$(command -v python3)"):/usr/bin:/bin" \
  python3 -m unittest tests.test_standalone_resources -v
```

独立发行包资源与生产桥接隔离集成验收（无桌面环境/Linux/macOS 均可在仓库根复跑；调用包内内置 Python/Rust/manage.sh 与生产桥接）：

```bash
cargo test --manifest-path app/src-tauri/Cargo.toml -- --ignored real_packaged_resources --test-threads=1 --nocapture
```

> **注意**：该验收直接检验 `resources/engine` 内置运行时、状态/规划/执行（status/plan/runs/snapshots）、同域 A/B 漂移单项生效（selective apply）、非法范围阻断、dryrun 零写入、离线门禁、日志可读性、快照恢复、首启与升级幂等保留用户状态、全程零 pyc 生成与只读资源不可变性。测试在纯命令行/终端运行，不依赖桌面显示服务（DISPLAY/Wayland），验证发行包资源与生产桥接路径，但不等同于 Linux 桌面图形渲染运行；未宣称 Linux 桌面图形端端到端已验收，未发布真实签名升级包。

真实引擎冒烟（开发态，需要源码根）：

```bash
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/agent-manager-engine-test.XXXXXX")"
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
  HOME="$TEST_HOME" PATH="$(dirname "$(command -v cargo)"):$(dirname "$(command -v python3)"):/usr/bin:/bin" \
  AGENT_MANAGER_ROOT=YOUR_AGENT_MANAGER_ROOT \
  cargo test --manifest-path app/src-tauri/Cargo.toml real_manage_sh_mcp_status_returns_ok -- --ignored --nocapture
```

该命令只运行指定的真实引擎冒烟用例；临时 HOME 和显式 PATH 用于隔离用户配置与外部命令发现。

dev-only 可视化验证（纯浏览器，不触引擎）：

```bash
pnpm dev        # 打开 http://localhost:1420/?mock=1       （内置 fixture 全集）
                #       http://localhost:1420/?mock=offline（offline 模式变体）
```

## Rust sidecar 桥（src-tauri/src/engine.rs）

暴露 Tauri command `engine_invoke(command, args, run_id?)`：

- 运行态解析与隔离：
  - **发行态（Release / `not(debug_assertions)`）**：从 Tauri 应用资源目录（`resource_dir() / "resources/engine"`）读取程序清单与只读资源，检查清单结构、文件存在性与内置执行体权限，再逐文件原子替换用户数据目录（`engine/`）中的程序；绑定包内 Python（`runtime/python/bin/python3`）与 Rust 引擎（`bin/agent-manager-engine`），不依赖源码仓库根、系统 Python 或 Homebrew。
  - **开发态（Development / `debug_assertions`）**：`resolve_root()` 解析源码根，顺序为 env `AGENT_MANAGER_ROOT` > 可执行文件祖先目录探测（含 `manage.sh`）> `app/src-tauri/` 的上两级目录；使用开发环境 Python 与 Rust 引擎。
- 执行 `bash <root>/manage.sh <command> --json <args...>`（manage.sh 自己完成三层策略合并），
  stdout 按 Contract v1 envelope 解析校验；退出码 0/1/2 都可能携带合法 envelope，
  只有 stdout 无法解析时才返回结构化 `EngineError`（含退出码与 stderr 摘要）。
- 命令面白名单：`<domain>:status|plan|apply`（secrets 仅 status）、
  `system:audit|refresh-upstream|converge`、`snapshots:list|restore`、`runs:list|get`、
  `policy:get|set`、`sources:get|set`、`profile:status|pull|push`。
- 参数命令感知白名单：`--only <元素id>`（refresh-upstream）/ `--only <顶层键>`（profile:pull）、
  `--dry-run`、`--allow-uninstall`（仅 agents:apply）、`--run <id>`、`--lines <n>`、
  `--layer synced|local`、`--key <dot.path>`、`--value <json>`、`--accept-remote`（仅 profile:pull）、
  `--force`（仅 profile:push）；policy:delete 仅 --layer/--key。
- run_id 经 `AGENT_MANAGER_RUN_ID` 透传（manage.sh 的 `RUN_ID="${AGENT_MANAGER_RUN_ID:-…}"`
  优先采用外部值），按次日志机制不变。
- 默认超时 600s，可用 `AGENT_MANAGER_ENGINE_TIMEOUT_SEC` 覆盖。

## 页面（src/pages/）

- Dashboard / Agents / MCP / Skills / Rules / Secrets / 更新中心：状态页（Skills 三级视图、
  MCP 注入矩阵、更新中心 freshness 总表）；写动作经 PlanPreviewModal 编排
  （plan 动作清单 + 风险级驱动确认：install-remove 二次确认输入元素 id、network 知情勾选、
  write-local/exec-local 普通确认 + dry-run 开关）。
- Hooks / Tools / Aliases：DomainApplyPage（元素表 + 收敛此域）。
- 执行中心（RunsPage）：runs:list 5s 轮询；会话内执行记录卡（run_id/状态机/步骤摘要/锁状态）；
  日志尾部 runs:get（运行中 2s 轮询）。
- 策略编辑器（PolicyPage）：policy:get 三层视图（baseline 灰只读 / synced 蓝 / local 橙）+
  任意键编辑（落点 synced/local）+ 默认落点层选择（skills 矩阵快捷写入用）。
- 备份回滚（SnapshotsPage）：snapshots:list + restore 二次确认（输入 run_id）。
- 配置同步（SyncPage）：profile:status 卡 + 逐项差异表（diff：仅本机改/仅远端改/两边都改/一致，
  只显示类型与 12 位内容指纹）+ 四个方向按钮（拉取合并 / 以远端覆盖本地（二次确认）/ 推送 /
  强制推送（仅分叉时可用，二次确认））+ 键级「以远端为准」（pull --accept-remote --only）；
  offline 全部禁用并标注。
- 元素启停：Agents 卡片 / MCP 矩阵行 / Tools 行 / Skills 集合卡均有开关（policy.set 写默认策略层，
  toast 给「立即同步」动作）；Agent、Tools、MCP 服务与 Skills 集合支持新增/编辑/移除（覆盖层语义）。Agent/Tools 使用 JSON 定义表单，Skills 集合支持编辑来源、发现范围、目标助手和单独更新。
- Rules / Hooks / Aliases 的「管理定义」入口直接编辑源码或来源 JSON，支持增删内容；保存带读取版本校验与快照，之后预览域级同步。规则生成文件不直接编辑。来源保存属于配置写入，预览中的演练开关只控制后续同步。
- 设置：展示并保存 `offline`、`defaultDryRun`、`upstreamTtlSec`，以及应用/契约版本、资源目录、数据目录、日志目录和更新配置状态。设置持久化到稳定用户数据目录的 `settings.json`，通过独立的 `settings_get` / `settings_save` 接口读取和写入，重启后由真实桥接消费。

更新采用手动检查与确认流程：未配置真实 updater 公钥或签名发布材料时显示“未配置”，不发起检查；检查到新版本后由用户明确确认下载和安装，取消不会下载，安装完成后提示重启。当前未进行签名发布。

## 前端结构

- `src/lib/contract.ts`：Contract v1 的 TS 镜像（Envelope/Element/Freshness/PlanAction/
  ApplyResult/Converge/Runs/Snapshots/Policy/Profile…）。
- `src/lib/observed.ts`：各域 spec/policy/observed 的页面级防御性读取类型（契约层为 free-form object）。
- `src/lib/engine.ts`：EngineClient 抽象（生产走 `engine_invoke`，测试注入 fake）+ zod envelope 校验
  + dev-only mock 开关（仅 `import.meta.env.DEV` 生效；生产构建 dynamic import 随死代码 treeshake）。
- `src/lib/queries.ts`：react-query（staleTime 30s、retry 1）+ `useEngineQuery` / `useRefreshUpstream` /
  `useOfflineMode`。
- `src/lib/execution.tsx`：写路径执行记录（run_id 生成与透传、状态机、toast、失效刷新）。
- `src/lib/usePlanPreview.tsx`：「先 plan 后 apply」编排 hook。
- `src/lib/ui-copy.ts`：界面文案唯一事实源（工程术语 → 用户语言对照；tooltip 保留原值对照）。
- `src/lib/useElementToggle.tsx`：元素启停开关编排（policy.set enabled 键 → toast「立即同步」→ plan 预览 → apply）。
- `src/lib/definitionOverlay.ts` + `useDefinitionOps.tsx`：定义覆盖层（mcp.definitions / skills.repositories）
  的新增·编辑·移除（覆盖层条目 policy:delete；基线条目 tombstone {"_delete": true}）。
- `src/components/`：StateBadge、FreshnessBadge、MatrixCell、DriftExplanationPanel、ElementTable、
  ErrorPanel、ModeBanner、AppLayout、PlanPreviewModal、ConfirmModal、PageState、Switch、
  McpServerFormDialog、SkillCollectionFormDialog。
- `src/test/mock-client.ts`：dev-only 有状态 mock 引擎（policy.set 会真实改写内存层、
  apply/converge 产生 run 与快照记录），fixture 覆盖 freshness 五态与收敛七态。

UI 中文单语。status 数据只读探测缓存（零网络）；联网探测只由用户显式触发刷新按钮。
