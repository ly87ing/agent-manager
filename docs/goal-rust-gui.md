# /goal 任务书：跨平台 GUI 实现（rust 分支）

> 本任务书驱动 `docs/GUI-ARCHITECTURE-PROPOSAL.md`（下称**提案**）的完整落地。
> 提案是规格的唯一事实源；本文件是执行契约。两者冲突时停止并报告，不自行裁决。
>
> **用法**：按 Phase 顺序，把对应「可粘贴 goal 文本」整段作为 `/goal` 目标触发；
> 一个 Phase 验收通过后再触发下一个。每 Phase 完成后更新文末进度表。
>
> **执行分支**：`rust`（专用分支）。**禁止**在 `main` 上直接提交本任务任何改动。

## 通用骨架（所有 Phase 共用）

```
目标:见各 Phase 文本（终态 = 提案 §6.3 对应阶段的全部编号项落地）
质量门槛:达到可长期维护的开源基础设施水准——契约自洽、测试带反例、文档与实现一致
自审:每个 Phase 的交付物至少审查和提升 3 遍，直到达到质量门槛
节奏:每 Phase 验收通过后在 rust 分支 commit + push origin/rust 作为检查点；
     阶段成果落地到文档（更新本文件进度表），不停留在会话里
```

## 全局禁区与边界（所有 Phase 生效，不随 Phase 豁免）

1. **secrets 红线**：任何代码、文档、示例、快照、日志、测试 fixture 中不得出现真实
   凭证；真实敏感信息只允许在外部私有 `secrets/` 仓库；新增 `app/`、`contract/`、
   `agent-profile/` 纳入 `scripts/check_no_secrets.py` 扫描范围。
2. **生成产物**：`agent-rule/rules/dist/` 只能通过 `agent-rule/scripts/build-agent-rules.py`
   重建，永不手改；GUI 中 dist 只读。
3. **Engine 单写者**：GUI 不直接写任何目标文件，一切变更走引擎命令。
4. **运行时契约不稀释**：dry-run / offline / 锁语义由 `manage.sh` 统一解释；
   新增命令必须遵守同一契约（无命令只给 flag 报错退出、原子锁、按次日志）。
5. **不锁版本**：不引入任何版本固定机制；探测 `unknown` 时收敛保守跳过，
   强制重试须显式（对齐 `--force-refresh` 语义）。
6. **既有行为不变**：现有命令（`update` / `init` / `agents:update` / `doctor` /
   `smoke` 及既有 flag）的对外行为与输出不变；新能力只能以新子命令族加入。
7. **工作区有他人进行中的未提交改动**（chrome-devtools 托管模型回调相关，建分支时
   已存在）：不属于本任务，不得提交、修改、回滚或混入任何 commit。
8. **新测试带反例**：每条新契约/新规则至少配一个反例 fixture 证明未放宽。
9. **复用既有机制**（原子写、深合并、探测缓存、`_MANAGED_BY` 标记），不新增平行
   抽象；确需新增时在提案中先补条款。
10. **未验收不推进**：Phase N 未通过验收，不开始 Phase N+1 的实现（允许为下一
    Phase 做只读准备）。

## 已决清单（定案，执行中不得重开讨论）

- 终点架构：Rust 引擎 + Tauri 2 客户端；路径：契约先行 → Python 兼容 → 按切片替换。
- GUI 是引擎的客户端；契约 `contract/engine-v1/` 是前后端唯一接口。
- 元素状态为双轴：收敛轴 `state` + 上游轴 `freshness`（提案 §3.3）。
- status 零网络：上游探测只经显式 `system.refresh-upstream` 写缓存（提案 §2.1.6）。
- skills v2：`skill_paths` = 发现范围；启用矩阵在策略层；`new_skill_default`
  缺省 `disabled`（提案 §7.1）。
- 策略三层栈：`manager.json ⊕ manager.synced.json ⊕ manager.local.json`，
  synced 层为空时行为与现状完全一致（提案 §7.2）。
- 同步后端 v1 只实现 git；后端地址只经 env / 仅本机层提供，不进主仓库。

## Phase 0 — 契约与 Python JSON 化（可粘贴 goal 文本）

```text
目标：在 rust 分支完成 docs/GUI-ARCHITECTURE-PROPOSAL.md §6.3 Phase 0 的全部编号项：
落地 contract/engine-v1/ 的 JSON Schema（envelope + 各域 status/plan）；manage.sh 新增
<domain>:status / <domain>:plan 子命令族并以 envelope 形态输出 --json（元素携带收敛轴
state 与上游轴 freshness block）；落地上游探测缓存与 system:refresh-upstream（status 本身
零网络）；完成 skills Spec/Policy v2 演进（skill_paths 重新定位为发现范围、启用矩阵入策略
层、link_skills.py 支持 per-skill × per-agent 收敛、旧式定义按提案 §7.1 等价语义自动升格、
行为逐比特不变）；策略合并从两层扩为三层（manager.json ⊕ manager.synced.json ⊕
manager.local.json，synced 缺省为空时行为与现状一致）；doctor --json 对齐 system.audit。
Done when：python3 -m unittest discover -s tests -p 'test*.py' 全绿；make smoke 通过；
契约测试用同一份 fixture 校验每个新命令输出符合 contract/engine-v1/ schema 且含反例；
既有命令在 config/manager.smoke.json 下行为与输出不变；python3 scripts/check_no_secrets.py 通过。
Scope：contract/、manage.sh、src/、agent-*/、config/skills.json、config/manager*.json、
tests/、docs/goal-rust-gui.md（进度表）。不创建 app/，不写任何 GUI 代码。
Loop：按提案 §6.3 Phase 0 编号项逐项推进，每项完成后先跑定向测试，再跑全量测试与
make smoke；全部完成后自审 3 遍（契约自洽性、反例覆盖、文档一致）。
Stop：若某项必须改变既有命令对外行为才能满足契约，停止并报告冲突点与对应提案条款；
不自行稀释契约、不修改既有行为、不放宽禁区。验收后在 rust 分支 commit + push origin/rust。
```

## Phase 1 — Tauri 壳 + 只读视图（可粘贴 goal 文本）

```text
目标：在 rust 分支完成提案 §6.3 Phase 1：搭建 app/（Tauri 2 + pnpm + Vite + React 18 +
TypeScript + Tailwind + Radix UI）；Rust 壳实现 sidecar 进程管理与契约 JSON 桥接；实现
Dashboard（含 freshness 聚合卡）、Agents/MCP 只读页（含 freshness 徽章）、Skills 三级
只读视图（集合/发现收件箱/启用矩阵）、漂移 diff 视图、更新中心只读表；数据全部来自
引擎 <domain>:status --json / system.audit，零写路径。
Done when：pnpm tauri dev 启动后各页面渲染真实引擎输出；Dashboard 信息与 doctor --json
等价（逐字段对账）；vitest + msw 组件测试通过（覆盖 freshness 徽章五态与 offline 标注）；
python3 -m unittest discover -s tests -p 'test*.py' 与 make smoke 保持全绿（引擎零回归）；
check_no_secrets 通过。
Scope：app/、contract/（仅勘误性修订）、docs/。不修改引擎任何写路径；不实现任何
apply 按钮（只读阶段）；不引入提案 §6.1 之外的前端依赖。
Loop：页面逐个落地，每页以「与引擎 JSON 输出逐字段一致」为验收；全部完成后自审 3 遍。
Stop：若 sidecar 在本机环境无法运行（缺 pnpm/Rust 工具链等），停止并报告缺失项与安装
方式，不伪造渲染结果。验收后 commit + push origin/rust。
```

## Phase 2 — 写操作、执行中心与配置同步（可粘贴 goal 文本）

```text
目标：在 rust 分支完成提案 §6.3 Phase 2：GUI 接入单元素 apply（含 skill × agent 矩阵
点击切换、更新中心「收敛此项」）、plan 预览确认弹窗（展示动作清单与风险级）、执行中心
（run_id、日志流、锁状态）；快照与回滚；策略编辑器（三层合并视图、override 分层落点、
基线层只读）；新增 agent-profile/sync_profile.py 实现 git backend 的
profile.status/pull/push（push 前 secret 扫描硬门禁 + 自动快照，冲突按 key 报告）。
Done when：GUI 执行一次 system.converge 后，受管目标文件与 make update 产物逐字节一致
（diff 为空）；策略编辑只写 synced/local 层、基线层零改动；两台本地模拟机器经 git 后端
互相同步共享层且冲突可按 key 解决；push 门禁能阻断含疑似明文的同步内容（反例测试）；
全量测试、make smoke、check_no_secrets 全绿。
Scope：app/、agent-profile/、contract/、tests/、config/（仅新增 manager.synced.json 相关
处理）、docs/。secrets 仓库与 manager.local.json 的既有语义不变。
Loop：按「单元素 apply → 执行中心 → 快照回滚 → 策略编辑器 → 配置同步」顺序推进，
每项以对应验收命令为准；自审 3 遍。
Stop：同步冲突语义与提案 §7.2 不符且无法在既有深合并机制上实现时，停止并报告；不为
赶工引入第二套合并逻辑。验收后 commit + push origin/rust。
```

## Phase 3 — 平台扩展与 Rust 切片替换（可粘贴 goal 文本）

```text
目标：在 rust 分支完成提案 §6.3 Phase 3：Linux 回归验证；Windows 过渡方案（sidecar 捆绑
python-build-standalone）或等效方案验证；按 mcp → agents 探测 → skills → rules/hooks 顺序
用 Rust 切片实现同一契约，每片通过既有 contract test 后退役对应 Python 模块（tools/
lifecycle 允许长期保留 Python sidecar，不强制替换）；GitHub Actions 矩阵构建三平台安装包
+ plugin-updater 更新源；配置同步 GUI 页落地。
Done when：每个 Rust 切片与 Python 实现在同一组 contract fixture 下输出一致（逐字节或
schema 等价）；Python 退役模块有对应删除 commit 且全量测试全绿；CI 矩阵构建产出
macOS/Linux 安装包（Windows 按验证结果据实报告）；make smoke 与 check_no_secrets 全绿。
Scope：app/src-tauri/、engine-rs/（如新建）、contract/、.github/、docs/。
Loop：切片逐个替换，每片「contract test 双实现对齐 → 退役 → 全量回归」走完再取下一片；
自审 3 遍。
Stop：Windows 方案若在本机无法验证，据实报告已验证范围与未验证项，不宣称全平台完成。
验收后 commit + push origin/rust。
```

## 进度跟踪

| Phase | 状态 | 验收证据 | commit |
|---|---|---|---|
| Phase 0 契约与 JSON 化 | 未开始 | — | — |
| Phase 1 Tauri 只读视图 | 未开始 | — | — |
| Phase 2 写操作与同步 | 未开始 | — | — |
| Phase 3 平台与 Rust 切片 | 未开始 | — | — |
