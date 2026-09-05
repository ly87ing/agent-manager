# 跨平台可视化架构设计提案 (GUI Architecture Proposal)

> **状态：设计提案，尚未实现。** 本文档描述目标架构与实施路线，不描述当前系统行为。
> 当前已实现行为以 `docs/ARCHITECTURE.md` 与 `README.md` 为准；本文档是
> `ARCHITECTURE.md`「未来改进方向 · 图形化界面」一条的展开与修正（终点从"基于 Web"
> 修正为"引擎契约 + 任意前端"，Tauri 桌面应用是第一个客户端）。
>
> 参考实现：[`farion1231/cc-switch`](https://github.com/farion1231/cc-switch)
> （Tauri 2 + React + Rust command 层，已验证同问题域的跨平台桌面形态）。

## 目录

1. [零基结论](#1-零基结论)
2. [引擎契约 Engine Contract v1](#2-引擎契约-engine-contract-v1)
3. [统一元素模型与状态机](#3-统一元素模型与状态机)
4. [功能推演（功能地图）](#4-功能推演功能地图)
5. [元素控制矩阵](#5-元素控制矩阵)
6. [实现方案](#6-实现方案)
7. [灵活性专题：Skill 细粒度与配置同步](#7-灵活性专题skill-细粒度与配置同步)
8. [红线与不变量](#8-红线与不变量)

---

## 1. 零基结论

### 1.1 持久资产与脚手架

仓库中真正持久的资产是 **`config/*.json` 声明式定义**（Spec 层）：每个 agent 的配置
路径/格式/键名、安装源命令、MCP server 定义、skill 仓库映射、rules 分发清单。这些
是语言中立的纯数据，任何实现都可直接消费。各同步引擎（Python + shell）只是 Spec 的
执行器 —— 执行器可以替换，Spec 必须原样保留。

### 1.2 三层架构

```
┌────────────────────────────────────────────────────────┐
│ Frontend 层（瘦客户端，可多个并存）                      │
│   Tauri 桌面 GUI · CLI · CI 脚本 · 未来 TUI/Web         │
├────────────────────────────────────────────────────────┤
│ Engine 层（唯一写者，单一二进制目标形态）                 │
│   <domain>.status / plan / apply + system.audit        │
│   全部 stdin/stdout JSON，dry-run 内建于每个动作         │
│   实现：Python（现状）→ Rust（按切片渐进替换）            │
├────────────────────────────────────────────────────────┤
│ Spec 层（已有，语言中立，100% 保留）                      │
│   config/agents.json · mcp.json · skills.json          │
│   tools.json · rules-targets.list · *-hooks.json       │
│   manager.json ⊕ manager.synced.json ⊕ manager.local.json │
└────────────────────────────────────────────────────────┘
```

核心认知：**GUI 是引擎的一个客户端，不是系统本身**。参照 docker CLI + Docker
Desktop、`gh` + GitHub Web 的形态 —— 谁把 GUI 当系统做，谁就被 UI 框架绑架后端
决策。

### 1.3 终点与路径

- **终点**：Rust 引擎（单二进制、无 Python 运行时依赖）+ Tauri 2 客户端。
- **路径**：契约先行 → Python 先学会说契约（GUI 立即可开工，对后端语言无感）→
  Rust 按用户价值切片替换，Python 对应模块逐片退役。
- **反对一步重写**：现有 Python 引擎与 `tests/` contract test 是"安全性"的载体
  （原子写、secrets 边界、漂移审计），big-bang rewrite 的回归风险集中在最输不起
  的地方。

### 1.4 灵活性设计原则

来自实际使用的硬需求（详见 §7）抽象为四条贯穿全文的原则：

1. **粒度三级**：集合（collection）→ 条目（item）→ 每 agent，每一级都有独立
   策略；任何"一组东西"都不能只有全有或全无的开关。
2. **默认保守**：新出现的元素默认不接管、不启用；显式选择才生效。上游集合新增
   条目绝不自动扩大本机安装面。
3. **个性化全部走策略层**：Spec 描述"世界是什么"，Policy 表达"我要什么"；策略
   分层（仓库基线 ⊕ 个人共享 ⊕ 仅本机），每层归属与同步语义明确。
4. **后端可插拔**：存储/同步后端是接口不是假设；v1 实现 git 后端，其余后端经
   同一接口后续接入。

---

## 2. 引擎契约 Engine Contract v1

契约是整个项目最重要的产物：**一份语言中立的 JSON Schema**，Python 和 Rust 实现
都受同一份 contract test 检验，GUI/CLI/CI 都只依赖契约不依赖实现。

### 2.1 设计原则

1. **每个写动作都有 plan**：`plan` 输出动作列表但不执行；`apply` 执行。不存在
   "只有 apply" 的命令。
2. **一切输出走 envelope**：stdout 永远是结构化 JSON；人类可读文本只去 stderr。
3. **dry-run / offline 是 mode，不是子命令分支**：与 `manage.sh` 现有运行时契约对齐。
4. **锁与日志对客户端可见**：`lock.held`、`run_id`、日志路径都在 envelope 里，GUI
   才能正确展示"同步中"状态。
5. **secrets 永不出现明文**：任何 payload 中 secrets 只允许以占位符名与
   `resolved: bool` 形式存在。
6. **status 默认零网络**：状态读取只消费本地探测与缓存；上游新鲜度的网络探测
   由显式 `system.refresh-upstream` 触发（§3.3），GUI 轮询 status 不产生网络流量。

### 2.2 Envelope Schema（所有命令共用）

```json
{
  "contract": "agent-manager/engine/v1",
  "run_id": "20260905T103000Z-a1b2c3",
  "command": "mcp.plan",
  "mode": { "dry_run": true, "offline": false },
  "lock": { "held": true, "pid": 12345 },
  "log_file": "/tmp/agent_manager_sync_<user>_<runid>.log",
  "ok": true,
  "data": {},
  "warnings": [
    { "code": "MCP_ENV_UNRESOLVED", "element": "mcp:grok-search", "message": "..." }
  ],
  "errors": []
}
```

退出码语义：`0` = ok（warnings 允许）；`1` = errors 非空；`2` = 契约/用法错误。
warnings 沿用 doctor 的细分惯例：`可选工具未就绪` 与 `托管产物尚未生成` 两类提示
不与真实漂移混同。

### 2.3 命令面（Command Surface）

| 命令 | 语义 | 对应当前入口 |
|---|---|---|
| `agents.status / plan / apply` | agent 生命周期探测与收敛 | `make agents:update` / `init` 的 lifecycle 段 |
| `mcp.status / plan / apply` | MCP 注入状态与收敛 | `update` 的 MCP 段 |
| `skills.status / plan / apply` | skill 仓库同步与链接，**粒度到单 skill × 单 agent**（§7.1） | `update` 的 skills 段 |
| `rules.status / plan / apply` | rules 构建 + 分发 | `update` 的 rules 段 |
| `hooks.status / plan / apply` | Claude/Codex hooks 渲染 | `update` 的 hooks 段 |
| `tools.status / plan / apply` | 工具链生命周期 | `update` 的 tools 段 |
| `aliases.status / plan / apply` | shell alias 受管区块 | `update` 的 alias 段 |
| `secrets.status` | 仅存在性/占位符解析状态（只读） | secrets 初始化段的探测部分 |
| `profile.status / pull / push` | 个性化配置同步（§7.2） | 新增，无对应当前入口 |
| `policy.get / policy.set` | 三层策略合并视图与分层写入（§7.2） | 目前手工编辑 |
| `system.refresh-upstream` | 上游版本探测并写探测缓存（§3.3） | 新增；语义对齐现有 `--force-refresh` 的探测部分 |
| `system.audit` | 全系统体检 | `make doctor`（`--json` 已有雏形） |
| `system.converge` | 全管线收敛 = 各域 plan/apply 的编排 | `make update` |
| `runs.list / runs.get` | 执行历史与按次日志读取 | `/tmp/agent_manager_sync_*` |

### 2.4 plan 动作条目 Schema

```json
{
  "op": "write-file | run-cmd | symlink | unlink | install | uninstall | git-pull | render",
  "element": "mcp:grok-search",
  "target": "~/.claude.json",
  "risk": "write-local",
  "before_digest": "sha256:... | null",
  "after_digest": "sha256:...",
  "backup_path": "<snapshot>/~/.claude.json",
  "cmd": null,
  "note": "注入 mcpServers.grok-search（env 8 个占位符已解析 6 个）"
}
```

`risk` 五级：`read`（纯探测）→ `write-local`（写家目录配置，可备份回滚）→
`exec-local`（本地命令：构建、链接）→ `network`（拉取/下载，offline 下禁止）→
`install-remove`（安装/卸载软件，最高确认级）。

---

## 3. 统一元素模型与状态机

所有受管对象（agent / MCP server / skill / rules 目标 / hooks / tool / alias）
统一抽象为**元素**。每个元素携带**两条独立的状态轴**：收敛轴（本地 vs 期望）
与上游轴（本地 vs 上游最新，§3.3）：

```json
{
  "id": "skill:agent-skills/safe-merge-review",
  "domain": "skills",
  "spec": { "…": "来自 config/*.json 定义层" },
  "policy": { "enabled": true, "…": "三层策略合并结果（§7.2）" },
  "observed": { "…": "运行时探测到的物理状态" },
  "state": "managed-ok",
  "freshness": {
    "state": "behind",
    "current": "a1b2c3d",
    "upstream": "e4f5g6h",
    "checked_at": "2026-09-05T10:20:00Z",
    "probe": "git-ls-remote",
    "cache_age_sec": 1820
  },
  "actions": [
    { "id": "apply", "risk": "write-local", "needs_lock": true, "offline_ok": true }
  ]
}
```

元素 id 形式：`agents:claude-code`、`mcp:grok-search`、
`skill:<collection>/<skill-name>`（skill 为一等元素，见 §7.1）、
`rules:claude_global`、`tools:playwright-cli`。

### 3.1 收敛轴状态枚举（spec × policy × observed）

| state | 含义 | 典型动作 |
|---|---|---|
| `managed-ok` | 物理状态与期望一致 | 无 / 重新收敛 |
| `drifted` | 内容或结构与期望偏离 | `apply` 收敛、查看 diff |
| `missing` | policy 启用但物理缺失 | `apply` 物化 |
| `not-materialized` | 托管产物尚未生成（提示级，非漂移） | 首次 `apply` |
| `unmanaged-detected` | 探测到但未被托管（如 brew 装的 claude） | 提示迁移 / 忽略 |
| `unsupported` | 探测到来源但定义层未声明 | 显式报错，不静默 |
| `disabled` | policy 在该 agent/范围上未启用 | 启用（显式动作） |

### 3.2 不变量

- **Engine 是唯一写者**：GUI 永不直接写任何目标文件（包括 `config/*.json` 与
  三层策略文件），一切经 `policy.set` / `apply` 走引擎，继承原子写 + 锁 + 日志。
- **生成产物只读**：`agent-rule/rules/dist/` 在 GUI 中只读展示；编辑入口只能落在
  `rules/src/`（提供"打开源文件"跳转，不做 dist 在线编辑）。
- **单元素动作 = 全局收敛的特例**：`system.converge` 就是所有启用元素的批量
  plan/apply；GUI 上单元素按钮与全局按钮复用同一管线，只是过滤条件不同。

### 3.3 上游轴：新鲜度模型（freshness）

**动机**：每个受管组件"是否是最新版本"、每份配置/skill/MCP"是否与上游最新同步"，
必须在 UI 上一眼可见。这与收敛轴正交 —— 一个组件可以"与期望完全一致但落后上游"
（还没升级），也可以"已是最新但配置漂移"。

**freshness.state 枚举**：

| state | 含义 | UI 表现 |
|---|---|---|
| `current` | 本地 = 上游最新 | 绿色"已是最新" |
| `behind` | 本地落后上游 | 黄色"可更新 vX → vY" + 一键收敛入口 |
| `ahead` | 本地领先上游（本地修改/预发布） | 蓝色提示，可查看差异 |
| `unknown` | 探测失败/未探测 | 灰色"未知" + 上次检查时间 + 重试 |
| `not-applicable` | 该元素无上游概念 | 不显示 freshness 徽章 |

**探测源矩阵**（每类组件的"最新"如何判定）：

| 组件类型 | current（本地） | upstream（上游） | 备注 |
|---|---|---|---|
| agents / tools（npm 包） | `<bin> --version` / `npm ls -g` | `npm view <pkg> version` | 不锁版本，升级 = 重跑升级命令 |
| MCP（npx 包，如 chrome-devtools） | npx 缓存中已解析版本 | `npm view` | 启动时 npx 解析最新，缓存可能滞后 |
| MCP（uvx `git+…@branch`，如 grok-search / serena） | uvx 缓存的 commit | `git ls-remote <repo> <branch>` | 跟随分支头 |
| skill 集合仓库 | 本地缓存 HEAD commit | `git ls-remote` 默认分支头 | 复用 `src/git_remote_probe.py` |
| rules / hooks / aliases / 注入配置 | — | — | `not-applicable`；它们的"最新"= 收敛轴（src→dist 构建新鲜度、目标漂移） |
| detect-only agent（如 kimi-code） | bin 探测版本 | 无受管升级路径 | freshness 仅作观测，不提供收敛动作 |

**缓存与刷新纪律**：

- 上游探测结果写入探测缓存（带 `checked_at`）；所有 `*.status` 默认只读缓存，
  **零网络**（契约原则 2.1.6）。
- 缓存超过 TTL（默认 24h，可配）在 UI 标 `stale`，不自动联网 —— 联网探测只由
  显式动作触发：`system.refresh-upstream`（全部/单元素）或 GUI 的刷新按钮。
- **`unknown` 是一等状态**：探测失败 → `unknown` + warning，收敛时保守跳过下载
  （对齐现有语义：探测返回 unknown 时记录 warning 并跳过保守回退下载）；强制
  重试是显式动作，对齐现有 `--force-refresh` 的语义边界。
- **offline 模式**：不探测、不标红，展示 last-known 值 + `checked_at` + "offline，
  未检查"标注。

---

## 4. 功能推演（功能地图）

按用户场景组织。优先级：**P0** = 验证架构所需最小集（第一版只做这些）；**P1** =
日常价值主体；**P2** = 长尾与增强。

### 4.1 Dashboard（P0）

- 全元素健康总览：按 domain 分组的 `system.audit` 可视化（收敛轴状态徽章 +
  warning 分类）。
- **上游新鲜度聚合卡**：`behind N / current M / unknown K`，上次全量检查时间，
  「刷新上游状态」按钮（= `system.refresh-upstream`）。
- 「一键收敛」按钮 = `system.converge`（先 plan 预览动作清单，确认后 apply）。
- 上次运行摘要：`runs.list` 最近一条（时间、ok/warnings、日志入口）。

### 4.2 Agents 页（P0 只读 / P1 写）

- 卡片列表：display_name、**已装版本 + freshness 徽章**（`已是最新` /
  `可更新 2.1.0 → 2.1.3` / `未知·重试`）、provider 徽标（npm/native/
  brew-detect-only）、收敛轴 state 徽章、policy 开关（enabled / auto_install /
  auto_update）。
- provider 迁移引导：`unmanaged-detected`（brew）→ npm 的迁移提示与执行。
- 操作：安装缺失、升级到上游最新（= 对 `behind` 元素的一键收敛）、卸载
  （`install-remove` 级确认）。

### 4.3 MCP 页（P0 只读 / P1 写）

- **server × agent 注入矩阵**：行 = MCP server，列 = agent（claude-code / codex /
  kimi-code），单元格 = 已注入/未注入/漂移，点击切换 = 单元素 apply。
- 每个 server 行的 freshness 徽章：npx 包显示 registry 版本对比；uvx git 源显示
  本地缓存 commit vs 分支头（短 hash）。
- server 详情：command/args 展示、env 占位符解析状态（`GROK_API_KEY ✓resolved`，
  永不显示值）、`startup_timeout_sec`、per-agent `extra_args`。
- 目标配置文件在线查看（CodeMirror 只读渲染 JSON/TOML），漂移时展示 diff ——
  注入配置"是否最新"由收敛轴漂移视图表达。
- 新增 server 表单（react-hook-form + zod 校验）→ 写 `config/mcp.json` 走
  `policy.set` 等价物。

### 4.4 Skills 页（P1，三级视图，模型见 §7.1）

- **集合（collection）卡片**：url、跟随分支、**缓存新鲜度（本地 HEAD vs 上游
  分支头，`behind` 时显示 commit 差数）**、发现范围、`new_skill_default` 姿态、
  来源信任标识（自研/第三方）。
- **发现收件箱（discovery inbox）**：上游集合新出现的 skill 以 `disabled` 状态
  进入收件箱，展示名称/来源/SKILL.md 摘要，显式启用才进入分发 ——
  上游新增绝不自动扩大安装面。
- **skill × agent 启用矩阵**：行 = 单 skill，列 = agent，单元格 =
  已链接/未启用/死链；点击切换 = 单元素 apply（只动托管链接）。skill 内容随
  集合仓库 pull 更新（软链指向缓存）， freshness 体现在集合卡片上。

### 4.5 Rules 页（P1）

- `rules-targets.list` 目标清单：每目标收敛轴 state（含目录收敛漂移、陈旧残留）。
- dist 新鲜度：`src/` mtime > `dist/` mtime 时提示"构建落后"（属收敛轴，rules
  无上游轴）。
- dist 只读预览 + "打开 src" 跳转（系统编辑器）。

### 4.6 Hooks 页（P2）

- `claude-hooks.json` / `codex-hooks.json` 渲染结果预览与目标文件 diff。

### 4.7 Secrets 页（P1，只读）

- secrets 仓库存在性、最近拉取时间。
- 各 MCP env 占位符解析状态矩阵（server × 变量名 → resolved ✓/✗）。
- **永不出现值**；提供"打开 secrets 仓库"跳转而不是内置编辑器。

### 4.8 执行中心（P1）

- 每次 apply 的任务视图：run_id、实时日志流（tail log_file）、进度（plan 动作
  N/M）、结束摘要。
- 锁状态展示：锁持有者 PID、等待提示。

### 4.9 策略编辑器（P1，三层模型见 §7.2）

- 三层合并视图：`manager.json` 基线值灰显、`manager.synced.json` 共享层蓝标、
  `manager.local.json` 仅本机层橙标；每条 override 清晰标注来源层。
- 编辑动作必须选择**落点**：「共享（随同步走）」写 synced 层，「仅本机」写
  local 层；基线层只读（保持可审计，改动走正常 git 流程）。
- zod schema 校验 + 稳定 agent/skill ID 下拉（禁止手敲 ID）。

### 4.10 备份与回滚（P1）

- 每次 apply 自动快照写入快照目录；快照列表按 run_id 组织。
- 回滚 = 逆向 apply（write-file 用备份恢复；install-remove 不回滚，只提示）。

### 4.11 配置同步页（P2，模型见 §7.2）

- 后端状态卡：backend 类型/地址（脱敏显示）、`clean / ahead / behind / diverged /
  uninitialized`、last sync 时间。
- `pull` / `push` 操作；冲突时按 key 展示双方值与层归属，逐个选择保留方。
- 机器注册表（P2 远期）：哪些机器在共享这份个性化配置。

### 4.12 更新中心（P1，freshness 的统一视图）

- **全组件上游新鲜度总表**：agents / MCP servers / tools / skill 集合按
  `behind → unknown → current` 排序，每行显示 current → upstream、
  `checked_at`、缓存年龄、探测源。
- 行内操作：「收敛此项」（= 单元素 apply，不锁版本语义下即升级到上游最新）、
  「重新探测」（单元素 refresh）；`unknown` 行的强制重试是显式动作。
- 顶部：「全部刷新上游状态」+「收敛全部落后项」（= 过滤后的
  `system.converge`，先 plan 预览）。
- offline 模式：隐藏刷新与收敛按钮，整表切换为 last-known 只读视图并标注。

### 4.13 设置（P2）

- 默认 offline 模式、默认 dry-run 确认、上游探测缓存 TTL、日志目录、契约版本显示。
- 应用自更新（`tauri-plugin-updater`）。

### 4.14 远期（不在本提案范围承诺）

- Profiles（开发/测试/生产多环境策略组）。
- 远程机器管理（SSH/Tailscale 目标，复用现有远程运维文档的边界）。
- TUI / Web 客户端（同一契约的第二、三前端）。

---

## 5. 元素控制矩阵

每个域的"可执行动作 × 风险级 × 确认策略 × offline 可用性"总表。GUI 按此表渲染
按钮形态（风险级决定确认弹窗文案与二次确认）；引擎按此表执行（offline 下拒绝
`network` 动作并显式报错）。

| 域 | 动作 | risk | 确认 | 备份 | offline |
|---|---|---|---|---|---|
| agents | 探测/版本刷新 | read | 无 | — | ✓ |
| agents | 升级已装 agent | network | 单次确认 | — | ✗ |
| agents | 安装缺失 agent | install-remove | 单次确认 | — | ✗ |
| agents | 卸载 / provider 迁移 | install-remove | 二次确认 | 配置快照 | ✗ |
| mcp | 注入/移除 server（单 agent） | write-local | 无（可撤销） | 目标文件快照 | ✓ |
| mcp | 新增/编辑 server 定义 | write-local | 无 | config 快照 | ✓ |
| skills | clone/pull 集合仓库 | network | 无 | — | ✗ |
| skills | 启用/停用单 skill（单 agent） | exec-local | 无 | — | ✓ |
| skills | 调整集合发现范围 / `new_skill_default` | write-local | 无 | config 快照 | ✓ |
| skills | 死链清理 | exec-local | 列表确认 | — | ✓ |
| rules | 构建（src→dist）+ 分发 | write-local | 无 | 目标文件快照 | ✓ |
| hooks | 渲染 hooks 到仓级配置 | write-local | 无 | 目标文件快照 | ✓ |
| tools | 安装/升级 + runtime setup | install-remove | 单次确认 | — | ✗ |
| aliases | 受管区块写入 zshrc/zprofile | write-local | 首次确认 | 区块外不动 | ✓ |
| secrets | 拉取 secrets 仓库 | network | 无 | — | ✗ |
| profile | status（远端比对） | read | 无 | — | ✗* |
| profile | pull（远端 → 本地共享层） | network | 有冲突时逐 key 确认 | 本地层快照 | ✗ |
| profile | push（本地共享层 → 远端） | network | 推送门禁通过后确认 | 远端层快照 | ✗ |
| 更新中心 | 上游版本探测/刷新（全部或单元素） | network | 无 | 探测缓存 | ✗ |
| 更新中心 | 收敛落后项（单个或全部） | 取目标动作级 | plan 预览后确认 | 全量快照 | ✗ |
| 全局 | `system.converge` | 取各动作最高级 | plan 预览后确认 | 全量快照 | 按动作降级 |

\* profile.status 的本地部分 offline 可用，远端比对部分需网络。

控制原则：

1. **先 plan 后 apply**：任何确认弹窗里展示的是 plan 动作清单（目标、风险、
   before/after digest），不是一句"确定吗？"。
2. **风险级驱动 UI**：`read` 自动轮询；`write-local` 静默可撤销；`network` 普通
   确认；`install-remove` 显式二次确认。
3. **offline 是全局 mode**：GUI 切换后所有 `network` 按钮禁用并标注原因，而不是
   点了才报错。
4. **drift 永远可解释**：任何 `drifted` 徽章都可展开成 before/after diff。
5. **status 零网络**：后台轮询只读本地与缓存；任何联网探测都是用户显式触发
   （刷新按钮 / refresh 命令），`unknown` 不用红色告警表达，避免制造焦虑。

---

## 6. 实现方案

### 6.1 技术栈（对齐 cc-switch 已验证组合）

- 壳：**Tauri 2**（系统 WebView，三平台安装包，`plugin-updater` 自更新、
  `plugin-dialog` / `plugin-log` / `plugin-process`）。
- 前端：React 18 + TypeScript + Vite + Tailwind + Radix UI；数据层
  `@tanstack/react-query`（轮询 status）；表单 `react-hook-form` + `zod`；
  配置查看 `CodeMirror`（JSON/TOML lint）+ `smol-toml`；反馈 `sonner`。
- 引擎过渡形态：**Tauri sidecar** 调 Python（`manage.sh`/同步器），Rust 侧只做
  进程管理、JSON 桥接、快照目录管理；终态逐切片替换为原生 Rust command。

### 6.2 仓库布局（增量，不移动现有代码）

```
agent-manager/
├── contract/                  # Engine Contract v1 的 JSON Schema（契约单一事实源）
│   └── engine-v1/             # envelope.schema.json / agents.schema.json / ...
├── app/                       # Tauri 桌面应用
│   ├── src/                   # React 前端（页面按第 4 节功能地图组织）
│   └── src-tauri/             # Rust 壳：sidecar 管理、命令桥接、快照/回滚
├── agent-profile/             # 个性化配置同步引擎（§7.2，Phase 2 新增）
├── src/ · agent-*/            # 现有 Python 引擎，逐步学会说契约（不搬迁）
└── tests/                     # 现有 contract test 惯例延伸到契约测试
```

### 6.3 分阶段路线

**Phase 0 — 契约与 Python JSON 化**（纯后端，无 GUI 依赖）

1. 落地 `contract/engine-v1/*.schema.json` 与 envelope。
2. `manage.sh` 增加 `<domain>:status` / `<domain>:plan` 子命令族，`--json` 输出
   对齐 envelope；`doctor --json` 对齐为 `system.audit`。
3. status schema 含双轴（收敛 `state` + 上游 `freshness` block，§3.3）；落地
   上游探测缓存（`system.refresh-upstream` 写缓存，status 只读缓存）。
4. **Skills Spec/Policy v2 演进**（§7.1）：`skills.json` 的 `skill_paths` 重新定位
   为发现范围，启用矩阵进入策略层；`link_skills.py` 支持 per-skill × per-agent
   收敛，含向后兼容迁移。
5. **策略三层栈落地**（§7.2 前半）：运行时合并从两层扩为
   `manager.json ⊕ manager.synced.json ⊕ manager.local.json`（synced 层缺省为空，
   纯本地行为不变）。
6. contract test：同一份 fixture 校验每个命令输出符合 schema（沿用
   `tests/test_project_contracts.py` 惯例）。
7. **验收**：`make smoke` 扩展为跑契约冒烟；CI 红绿可见。

**Phase 1 — Tauri 壳 + 只读视图**（macOS）

1. `app/` 脚手架（pnpm + Vite + React + tauri dev）。
2. sidecar 调 `<domain>:status --json` / `system.audit`；实现 Dashboard
   （含 freshness 聚合卡）、Agents/MCP 只读页（含 freshness 徽章）、Skills 三级
   只读视图（集合/收件箱/矩阵）、漂移 diff 视图、更新中心只读表。
3. **验收**：GUI 只读信息 = `make doctor` + 探测缓存的并集，零写路径。

**Phase 2 — 写操作、执行中心与配置同步**

1. 单元素 apply（含 skill × agent 矩阵点击切换、更新中心「收敛此项」）、plan
   预览确认弹窗、执行中心（日志流 + 锁状态）。
2. 快照与回滚；策略编辑器（分层落点：共享层 vs 仅本机层）。
3. `agent-profile/sync_profile.py`：git backend 的 `profile.status/pull/push`，
   推送门禁（secret 扫描 + 快照）与冲突报告。
4. **验收**：GUI 完成一次 `system.converge` ≡ `make update`（产物逐字节一致）；
   两台机器经 git 后端互相同步共享层，冲突可解。

**Phase 3 — 平台扩展与 Rust 切片替换**

1. Linux 回归；Windows 过渡方案：sidecar 捆绑 `python-build-standalone`。
2. Rust 引擎按用户价值切片生长，每片实现同一契约后 Python 对应模块退役：
   **mcp**（纯 JSON/TOML 文件手术，smol-toml）→ **agents 探测**（版本/PATH/缓存）
   → **skills**（git2 + symlink，含启用矩阵与 freshness 探测）→ **rules/hooks**
   （模板渲染）→ **tools/lifecycle**（重进程编排，允许长期保留 Python sidecar）。
3. 发布管线：GitHub Actions 矩阵构建三平台安装包 + `plugin-updater` 更新源。
4. 配置同步 GUI 页与第二后端（gist/s3/webdav 之一，按实际需求驱动）。

### 6.4 测试策略

- **契约测试**（核心）：`contract/engine-v1/` schema + fixtures，同时打 Python 与
  Rust 实现 —— 这是"换引擎不换前端"的保障，沿用现有 unittest + contract-test 惯例。
- **GUI**：vitest + msw（mock envelope）；关键交互（plan 预览 → 确认 → apply →
  执行中心；收件箱启用；同步冲突按 key 解决；freshness 徽章与刷新纪律）组件级测试。
- **端到端**：`make smoke` 的 dry-run/preview 契约保持不变；新增 GUI 冒烟
  （tauri 驱动可选）。

---

## 7. 灵活性专题：Skill 细粒度与配置同步

### 7.1 Skill 集合的细粒度控制

**问题**：自研或第三方 skill 集合通常包含大量 skill，但并非每个都需要；不同
agent 大概率配置不同子集；集合内单个 skill 需要独立 enable/disable。

**现状局限**：`skills.json` 的 `skill_paths` 是仓库级白名单、`agents` 是仓库级
列表，策略层只有 `skills.<repo>.enabled` 仓库级开关 —— 三个维度都只有全有或全无。

**目标模型（v2，向后兼容）**：

- **Spec 层**（`config/skills.json`）：collection 定义 = url + **发现范围** +
  缓存位置。`skill_paths` 重新定位为"在仓库内哪些子路径参与 skill 发现"
  （不再是启用白名单）；skill 身份 = 集合内目录的 `SKILL.md` frontmatter `name`；
  provenance（来源仓库、分支、最近 fetch）随发现结果产出。
- **Policy 层**（三层策略栈）：per-collection 与 per-skill × per-agent 的启用
  策略：

```json
"skills": {
  "agent-skills": {
    "enabled": true,
    "new_skill_default": "disabled",
    "skills": {
      "safe-merge-review":   { "agents": { "claude-code": true, "codex": true, "kimi-code": false } },
      "change-discipline":   { "agents": null },
      "some-new-upstream-skill": { }
    }
  }
}
```

  - `new_skill_default: "disabled"`（默认，延续"不因上游新增自动扩大安装面"的
    既有安全姿态）：集合里新发现的 skill 进收件箱，不显式启用就不分发；
    对完全可信的自研集合可显式设为 `"enabled"`。
  - 单 skill 的 `agents`：`null` = 跟随 collection 的默认目标集；显式 map =
    每 agent 独立开关；缺省 `{}` = 按 `new_skill_default` 处理。
- **收敛语义**：每个 agent 的有效 skill 集 = 发现(集合) ∩ policy 矩阵；
  `link_skills` 按 agent 分别计算链接集；停用 = 只解除带托管标记的链接，
  不碰用户手工放置的目录。
- **元素粒度**：`skill:<collection>/<name>` 成为一等元素，收敛轴状态含
  `linked / missing-link / dead-link / disabled(per agent) / drifted`；上游轴
  freshness 挂在 collection 元素上（仓库级），单 skill 内容随仓库 pull 更新。
- **兼容迁移**：现行 `skill_paths` + `agents` 语义等价于"发现范围 = skill_paths
  且范围内 skill 对 collection 的 agents 默认启用"；v2 引擎读取旧式定义时按该
  等价语义自动升格，保证现有行为逐比特不变，迁移在 spec 文件中显式化后完成。

**信任边界**：启用一个第三方 skill = 把指令注入 agent 上下文。GUI 的启用动作必须
展示来源仓库与 SKILL.md 摘要，且启用是逐条显式动作，不提供"全选启用第三方集合"
的一键操作。

### 7.2 个性化配置的本地与远端同步

**问题**：个性化配置默认存本地；但存在同步到 GitHub（或其他仓库/存储）以便多
机器复用的真实需求；需要本地与远端之间的获取与同步能力。

**策略三层栈**（把现有两层扩为三层，每层归属与同步语义明确）：

| 层 | 文件 | 归属 | 同步语义 |
|---|---|---|---|
| 基线层 | `config/manager.json` | 主仓库，已提交 | 走正常 git 流程，可审计 |
| 共享层 | `config/manager.synced.json` | 个人，**同步引擎管理**（不手工编辑） | 经同步后端在多机间同步；不进主仓库 |
| 仅本机层 | `config/manager.local.json` | 单台机器（现状保留） | 永不离开本机 |

合并顺序：基线 ⊕ 共享 ⊕ 仅本机（本机层永远最高优先）。synced 层缺省为空时，
行为与现状两层完全一致。

**后端可插拔**：

- 同步引擎只面向 `sync-backend` 接口工作：`status() / fetch() / publish()`。
- v1 只实现 **git backend**（GitHub 私有仓库或任意 git remote，复用 secrets 仓库
  同款 clone/pull 模式）；gist / s3 / webdav 等后续经同一接口接入，按实际需求驱动。
- 后端地址**永不提交主仓库**：经环境变量（如 `PROFILE_SYNC_REPO_URL`）或
  `manager.local.json` 提供，对齐 `SECRETS_REPO_URL` 的既有模式。

**git 后端的同步仓库布局**：

```
profile.json          # 共享层内容的单一事实源（= manager.synced.json）
meta.json             # schema 版本、机器注册表（machine-id → 最近同步时间）
machines/<id>.json    # （P2 远期）按机器的共享层细分
```

**冲突语义**：深合并不相交键（与现有 local override 合并同一套深合并逻辑）；
同一键两侧都改 → 产生冲突报告，保留本地值，由 GUI 按 key 逐个选择保留方；
pull/push 之前都先自动快照本层。

**门禁**：push 前必须过 secret 扫描（复用 `scripts/check_no_secrets.py` 机制对
同步内容做扫描），命中即阻断 —— 同步仓库一旦误设为公开，代价必须可控。同步
内容永不包含 secrets 明文、secrets 仓库地址、机器特有路径。

**命令与模式**：`profile.status`（ahead/behind/diverged/uninitialized）、
`profile.pull`、`profile.push`；三者中 pull/push 为 `network` 级动作，
offline 下显式拒绝。

---

## 8. 红线与不变量

1. **Secrets 红线**：GUI 任何界面、日志、快照中永不出现 secrets 明文；只展示
   占位符名与 `resolved: bool`。延续 `secrets/` 外部私有仓库边界与
   `scripts/check_no_secrets.py` 扫描（新增 `app/`、`contract/`、`agent-profile/`
   目录纳入扫描范围）。
2. **生成产物边界**：`agent-rule/rules/dist/` 在 GUI 只读；规则编辑入口只能是
   `rules/src/`。新增 `contract/` 的 schema 是手写源，不是生成物。
3. **Engine 单写者**：GUI（包括策略编辑器）不直接写任何文件；一切变更走引擎，
   继承原子写、锁、按次日志、`_MANAGED_BY` 审计标记。
4. **运行时契约不稀释**：dry-run / offline / 锁语义由引擎统一解释，GUI 只是
   mode 的传递者，不自行实现"伪 dry-run"。
5. **不锁版本立场不变**："跟随上游最新"的策略不受 GUI 影响；freshness 与版本
   展示是观测信息，不引入版本固定 UI；探测 `unknown` 时收敛保守跳过，强制重试
   须显式动作（对齐 `--force-refresh` 语义）。
6. **同步红线**：配置同步的内容永不包含 secrets 明文、secrets 仓库地址与机器
   特有路径；push 前 secret 扫描是硬门禁；同步后端地址只经 env 或仅本机层提供，
   不提交主仓库。
7. **Skill 信任红线**：集合中新发现的 skill 默认不启用（`new_skill_default`
   缺省 `disabled`）；启用动作逐条显式执行且展示来源与 SKILL.md 摘要；不提供
   第三方集合的一键全量启用。
