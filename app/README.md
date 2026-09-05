# app/ — Agent Manager 桌面客户端（Phase 1 Slice A）

Tauri 2 壳 + React 18 只读前端，数据全部来自引擎契约命令（Contract v1 envelope）。
本切片**零写路径**：唯一主动作是 Dashboard 的「刷新上游状态」（`system:refresh-upstream`，
network 级探测，不写配置）。

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

## Rust sidecar 桥

`src-tauri/src/engine.rs` 暴露 Tauri command `engine_invoke(command, args)`：

- 仓库根解析顺序：env `AGENT_MANAGER_ROOT` > 可执行文件祖先目录探测（含 `manage.sh`）>
  dev 时 `app/` 的父目录。
- 执行 `bash <root>/manage.sh <command> --json <args...>`（manage.sh 自己完成三层策略合并），
  stdout 按 Contract v1 envelope 解析校验；退出码 0/1/2 都可能携带合法 envelope，
  只有 stdout 无法解析时才返回结构化 `EngineError`（含退出码与 stderr 摘要）。
- 命令面白名单：`<domain>:status|plan`（secrets 仅 status）、`system:audit`、
  `system:refresh-upstream`；附加参数仅放行 `--only <element-id>`。
- 默认超时 600s，可用 `AGENT_MANAGER_ENGINE_TIMEOUT_SEC` 覆盖。

真实引擎冒烟（需要本仓库）：

```bash
AGENT_MANAGER_ROOT=/path/to/agent-manager cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture
```

dev-only 可视化验证（纯浏览器，不触引擎）：

```bash
pnpm dev        # 打开 http://localhost:1420/?mock=1      （内置 fixture 全集）
                #       http://localhost:1420/?mock=offline（offline 模式变体）
```

## 前端结构

- `src/lib/contract.ts`：Contract v1 的 TS 镜像（Envelope/Element/Freshness/PlanAction/AuditData…）。
- `src/lib/observed.ts`：各域 spec/policy/observed 的页面级防御性读取类型（契约层为 free-form object）。
- `src/lib/engine.ts`：EngineClient 抽象（生产走 `engine_invoke`，测试注入 fake）+ zod envelope 校验
  + dev-only mock 开关（`?mock=1` / `?mock=offline`，仅 `import.meta.env.DEV` 生效；
  生产构建中 dynamic import 随死代码 treeshake，mock 模块不进产物）。
- `src/lib/queries.ts`：react-query（staleTime 30s、retry 1）与 `useEngineQuery` / `useRefreshUpstream`。
- `src/components/`：StateBadge（收敛七态）、FreshnessBadge（上游五态 + 缓存年龄/stale + git 短 hash）、
  MatrixCell（MCP 注入格 / Skills 启用格词汇表）、DriftExplanationPanel（结构化漂移解释：
  目标路径 + per-agent 单元格状态 + 对应域 plan 动作清单过滤）、ElementTable、ErrorPanel、
  ModeBanner（offline/dry-run 全局横幅）、AppLayout。
- `src/pages/DashboardPage.tsx`：system:audit 总状态卡 + 警告分类 + 按段审计明细 +
  agents/tools/mcp/skills 四域 freshness 聚合卡 + 「刷新上游状态」按钮。
- `src/pages/AgentsPage.tsx`：卡片（display_name/已装版本/provider 分类徽标/policy 开关只读）+
  unmanaged-detected 迁移提示（只读）+ 漂移解释展开。
- `src/pages/McpPage.tsx`：server × agent 注入矩阵 + server 详情（命令/env 占位符解析状态/
  extra_args_by_agent/目标路径/漂移解释）。
- `src/pages/SkillsPage.tsx`：集合卡片 + 发现收件箱 + skill × agent 启用矩阵（三级视图）。
- `src/pages/RulesPage.tsx`：目标清单 + rules:dist 构建新鲜度卡（落后提示）。
- `src/pages/SecretsPage.tsx`：secrets 仓库状态卡 + MCP env 占位符解析矩阵（值永不显示）。
- `src/pages/UpdatesPage.tsx`：更新中心——四域 freshness 总表（behind→unknown→current→ahead）、
  全部刷新 + 行内「重新探测」（--only），offline 整表只读并禁用刷新。
- Hooks / 设置为占位页（Phase 1 后续切片实现）。

UI 中文单语。status 数据只读探测缓存（零网络）；联网探测只由用户显式触发刷新按钮。
