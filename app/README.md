# app/ — Agent Manager 桌面客户端

Tauri 2 壳 + React 18 前端，数据全部来自引擎契约命令（Contract v1 envelope）。
写路径统一编排（Phase 2c）：**plan 预览确认 → 域级 apply / system:converge（引擎单写者，
非 dry-run 前自动快照）**；skills 矩阵点击 = policy.set（落默认策略层）+ skills:apply 预览确认。
GUI 发起的动作带 `gui-` 前缀 run_id（经 AGENT_MANAGER_RUN_ID 透传），执行中心按 run_id 追踪日志。

## 命令

```bash
pnpm install
pnpm dev          # Vite 开发服务器（1420 端口；纯浏览器下引擎调用会失败，需经 Tauri 运行）
pnpm typecheck    # tsc --noEmit
pnpm test:unit run
pnpm build        # tsc --noEmit && vite build
pnpm tauri dev    # 桌面壳（需要 Rust 工具链）
cd src-tauri && cargo check && cargo test
```

真实引擎冒烟（需要本仓库）：

```bash
AGENT_MANAGER_ROOT=/path/to/agent-manager cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture
```

dev-only 可视化验证（纯浏览器，不触引擎）：

```bash
pnpm dev        # 打开 http://localhost:1420/?mock=1       （内置 fixture 全集）
                #       http://localhost:1420/?mock=offline（offline 模式变体）
```

## Rust sidecar 桥（src-tauri/src/engine.rs）

暴露 Tauri command `engine_invoke(command, args, run_id?)`：

- 仓库根解析顺序：env `AGENT_MANAGER_ROOT` > 可执行文件祖先目录探测（含 `manage.sh`）>
  dev 时 `app/` 的父目录。
- 执行 `bash <root>/manage.sh <command> --json <args...>`（manage.sh 自己完成三层策略合并），
  stdout 按 Contract v1 envelope 解析校验；退出码 0/1/2 都可能携带合法 envelope，
  只有 stdout 无法解析时才返回结构化 `EngineError`（含退出码与 stderr 摘要）。
- 命令面白名单：`<domain>:status|plan|apply`（secrets 仅 status）、
  `system:audit|refresh-upstream|converge`、`snapshots:list|restore`、`runs:list|get`、
  `policy:get|set`、`profile:status|pull|push`。
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
  toast 给「立即同步」动作）；MCP 服务与 Skills 集合支持新增/编辑/移除（覆盖层语义）。
- 设置：占位页。

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
