# Legacy Unit Test Seeding Workflow

## 目标

本流程用于没有单元测试或单元测试很少的老项目。目标不是让 AI 直接追覆盖率数字，而是在不改变现有生产行为的前提下，为核心业务逻辑建立可重构的测试安全网。

最终目标由客户要求决定：

```text
TARGET_MEANINGFUL_COVERAGE = 客户约定的覆盖率目标
```

如果客户要求的是全仓覆盖率，也必须优先通过核心业务逻辑测试贡献覆盖率，不允许用 DTO、getter/setter、简单 CRUD 或无意义断言刷数字。

## 成功标准

一次测试建设任务完成时，必须同时满足：

- 核心业务逻辑被测试覆盖，而不是只提高报表数字。
- 新增测试锁定当前行为，能保护后续重构。
- 生产代码行为没有变化。
- 测试可重复运行，不依赖真实外部服务、真实网络或不稳定时间。
- 测试包含明确断言，不接受只判断 `not null` 或只验证 mock 被调用的假测试。
- 覆盖率报告可生成，并能说明覆盖了哪些核心逻辑、还缺哪些核心逻辑。
- 疑似错误行为被记录，但没有在测试任务中顺手修复。

## 基本原则

生产代码默认只读。添加测试期间不修改业务逻辑、不重构生产代码、不改变 public API、不改数据库 schema、不改变运行时行为。

覆盖率目标必须参数化。文档、prompt、skill 和 CI 中都不要写死固定百分比，统一引用客户约定的目标值。

先锁定当前行为，再判断业务正确性。Characterization test 的职责是记录系统当前如何工作，不代表当前行为一定正确。

每批只处理少量高价值模块。建议每批选择 1 到 3 个核心模块，完成测试、验证、覆盖说明和风险记录后再进入下一批。

CI gate 分阶段启用。初期只生成 report，不直接因覆盖率阻断合并；等核心测试稳定后，再逐步启用 coverage 不下降、新增代码必须有测试、客户目标 gate。

## 不做什么

- 不为了 coverage 测试 DTO、getter/setter、简单 mapper 或框架 glue code。
- 不 mock 掉被测方法本身。
- 不写没有业务断言的测试。
- 不为了让测试通过而降低断言、删除测试、跳过测试或放宽失败条件。
- 不在测试建设任务中顺手修 bug。
- 不一次性铺开全仓测试，避免 AI 在低价值文件上消耗时间。

## Phase 0：冻结当前基线

目标是确认项目现状，避免 AI 一边补测试一边误改系统。

执行内容：

1. 识别项目语言、构建工具、测试框架、启动命令、测试命令和 coverage 命令。
2. 查找 README、Makefile、package/build 文件、CI 配置和现有测试目录。
3. 运行最小安全验证命令；如果命令可能修改外部环境，先记录原因并等待确认。
4. 记录当前失败项，只记录不修复业务代码。
5. 生成 `docs/testing/current-baseline.md`。

`current-baseline.md` 至少包含：

```markdown
# Current Baseline

## Project
- Language:
- Build command:
- Unit test command:
- Coverage command:

## Current Result
- Command:
- Result:
- Known failures:
- Notes:

## Constraint
- Production code was not modified during baseline discovery.
```

## Phase 1：建立风险地图

目标是让 AI 按业务风险选测试目标，而不是按容易覆盖的文件选目标。

优先级最高：

- 金额、计费、订单、支付、退款、折扣、结算逻辑。
- 权限、鉴权、认证、角色、租户隔离逻辑。
- 状态机、生命周期、workflow、审批流、任务流转逻辑。
- 复杂分支、复杂校验、复杂数据转换。
- 最近频繁修改、bug 较多、线上事故相关模块。
- 客户重点验收流程和核心入口。

暂不优先：

- DTO、纯数据对象、getter/setter。
- 纯 CRUD。
- 简单 mapper。
- 薄 controller。
- 框架配置和没有业务含义的 glue code。

生成 `docs/testing/risk-map.md`，格式建议：

```markdown
# Risk Map

## High Priority

### ModuleOrClass.method
- Reason:
- Current entry point:
- Business risk:
- Suggested test type:
- Suggested cases:
- External dependencies:
- Priority:

## Medium Priority

### ModuleOrClass.method
- Reason:
- Suggested cases:
```

## Phase 2：建立测试基础设施

目标是让测试可稳定运行，但不改变生产行为。

允许做：

- 使用项目已有测试框架或团队标准测试框架。
- 增加测试 helper、fixture、mock factory。
- 增加本地 coverage report 命令。
- 增加测试数据构造工具。
- 增加隔离外部依赖的 mock 或 fake。

需要人工确认后才能做：

- 添加新的测试依赖。
- 修改 CI。
- 修改生产代码以暴露测试 seam。
- 修改构建脚本或运行脚本。
- 修改数据库 schema、public API 或运行时配置。

禁止做：

- 为了测试方便重构业务代码。
- 为了测试方便改变业务逻辑。
- 引入大型依赖但没有明确收益。
- 使用真实网络、真实外部服务或不可控测试数据。

## Phase 3：编写 Characterization Tests

目标是锁定当前行为，为后续重构提供安全网。

测试方式：

```text
固定输入 -> 调用真实业务逻辑 -> 断言当前输出或状态变化
```

每个核心目标至少考虑：

- 正常路径。
- 边界值。
- 空值、非法值、异常输入。
- 关键分支。
- 依赖失败。
- 状态变化。
- 错误返回或异常行为。

写测试时必须遵守：

- 测真实逻辑，不测 AI 猜出来的理想逻辑。
- Mock 外部依赖，不 mock 被测方法本身。
- 对当前行为可疑的地方加说明，并记录到 `docs/testing/suspicious-behavior.md`。
- 不在同一任务里修复可疑行为。

`suspicious-behavior.md` 格式建议：

```markdown
# Suspicious Behavior

## ModuleOrClass.method
- Observed behavior:
- Why suspicious:
- Test that locks it:
- Suggested product decision:
- Status: recorded only, not fixed
```

## Phase 4：扩展有效覆盖率

目标是向客户覆盖率目标推进，但只计算有意义的覆盖。

有效覆盖包括：

- service/domain 业务规则。
- 状态流转。
- 校验规则。
- 复杂分支。
- 错误处理。
- 依赖失败行为。
- 有业务含义的数据转换。

无效覆盖包括：

- 只构造对象。
- 只断言非空。
- 只验证 mock 被调用但不验证业务结果。
- 测试纯 DTO、getter/setter 或简单 CRUD。
- snapshot 内容没有业务审核价值。

每批结束后更新 `docs/testing/coverage-map.md`：

```markdown
# Coverage Map

## Target
- Customer coverage target:
- Current meaningful coverage:
- Current overall coverage:
- Measurement command:

## Covered

### Module
- Function:
- Test file:
- Cases:
- Risk covered:
- Notes:

## Remaining Gaps

### Module
- Missing cases:
- Reason:
- Priority:
```

## Phase 5：CI 保护策略

目标是逐步把测试安全网变成团队约束，避免老项目一开始被过高 gate 阻断。

推荐顺序：

1. 第一阶段：CI 只生成 unit test 和 coverage report，不因覆盖率失败阻断合并。
2. 第二阶段：新增测试必须通过，已有测试不能被删除、跳过或放宽。
3. 第三阶段：核心模块 coverage 不下降。
4. 第四阶段：新增核心逻辑必须带测试。
5. 第五阶段：客户约定覆盖率目标作为 gate。

## Claude Code 和 Codex 的协作方式

Claude Code 更适合前置分析：

- 梳理项目结构。
- 建立风险地图。
- 设计测试策略。
- 识别测试基础设施缺口。
- 总结可疑现有行为。

Codex 更适合执行闭环：

- 批量添加测试文件。
- 运行测试命令。
- 修复测试编译错误。
- 生成 coverage 结果。
- 更新 coverage-map。

两者都必须遵守同一套仓库规则：

- `AGENTS.md` 放项目级通用规则。
- `CLAUDE.md` 通过 `@AGENTS.md` 或 symlink 复用通用规则。
- `legacy-unit-test` 的长期维护源放在 `agent-skills` 仓。
- `agent-manager` 通过 `config/skills.json` 和 `./manage.sh skills:link` 暴露给 Claude Code、Codex 和 Antigravity CLI。

## Skill 和 Goal 的使用方式

推荐把 skill 和 `/goal` 分开理解：

```text
Skill = 固化怎么做
/goal = 驱动本批做到什么可验证终点
CI / test / coverage / 文档 = 证明真的做到
人工审批门 = 防止老项目被误改
```

仅有 skill 和 `/goal` 不等于完成目标。`/goal` 只能持续推进会话，它不能替代测试命令、coverage 报告、文档记录和人工审批门。

### Source-of-truth

正式 skill 维护在：

```text
agent-skill/repositories/agent-skills/legacy-unit-test/SKILL.md
```

不要在业务仓里同时维护另一份同名 skill。业务仓只需要通过受管同步或复制安装后的版本使用它。

### Codex 使用方式

使用方式：

```text
/skills
```

选择或显式调用 `legacy-unit-test` 后，再设置一个批次目标：

```text
/goal Use legacy-unit-test for the first batch: cover 1 to 3 High Priority modules, do not modify production code, add characterization tests with real assertions, run the smallest relevant tests and coverage command, update docs/testing/coverage-map.md, record suspicious behavior only, and stop for approval before dependencies, CI, build scripts, or production-code seams.
```

如果 Codex 中没有 `/goal`，需要先启用 goals 功能，或改用普通 prompt 分批执行。

### Claude Code 使用方式

Claude Code 使用同步后的 skill：

```text
~/.claude/skills/legacy-unit-test/SKILL.md
```

使用方式：

```text
/legacy-unit-test
```

或让 Claude 根据任务自动触发该 skill。然后设置一个可验证的 `/goal`：

```text
/goal 使用 legacy-unit-test 完成第一批老项目单元测试安全网：只选择风险地图中 High Priority 的 1 到 3 个核心模块；不修改 production code；新增 characterization tests 锁定当前行为；新增测试必须有真实断言；运行最小相关测试命令和 coverage 命令；更新 docs/testing/coverage-map.md；如发现疑似 bug，只记录到 docs/testing/suspicious-behavior.md，不修复；如果需要添加依赖、修改 CI、修改构建脚本或修改生产代码，立即停止并请求人工确认；最终输出修改文件、覆盖逻辑、命令结果、coverage 结果和下一批目标。
```

### Goal 编写原则

`/goal` 必须是批次级目标，不要写成全仓一次性目标。

好的 `/goal` 需要包含：

- 可验证终点，例如测试命令退出码、coverage 结果、文档更新。
- 明确范围，例如 1 到 3 个高风险模块。
- 禁止事项，例如不改生产代码、不修 bug、不刷低价值覆盖。
- 停止条件，例如需要依赖、CI、构建脚本或生产代码 seam 时先请求确认。
- 证据要求，例如最终输出修改文件、命令结果和覆盖说明。

不建议这样写：

```text
/goal 把项目单元测试覆盖率做到客户要求
```

这个目标太粗，AI 容易选择低价值文件刷 coverage，也难以及时停下来请求审批。

建议这样写：

```text
/goal 使用 legacy-unit-test 完成本批核心逻辑 characterization tests：范围限制为 1 到 3 个 High Priority 模块；不修改 production code；新增测试必须有真实断言；运行最小相关测试和 coverage；更新 coverage-map；疑似 bug 只记录不修复；如果需要改变依赖、CI、构建脚本或生产代码，停止并请求确认。
```

### 使用限制

`/goal` 的判断依赖会话中已经展示的证据。它不会替代 agent 自己去运行命令、读取文件或核对 coverage。因此，skill 和 goal 都必须要求 agent 在最终报告中展示：

- 修改文件列表。
- 测试命令和 exit code。
- coverage 命令和结果。
- coverage-map 更新内容。
- suspicious-behavior 记录。
- production code 是否被修改。

## 每批交付物

每批测试建设必须交付：

- 新增或修改的测试文件。
- 更新后的 `docs/testing/coverage-map.md`。
- 如有疑似 bug，更新 `docs/testing/suspicious-behavior.md`。
- 本批覆盖的核心逻辑清单。
- 本批未覆盖的核心逻辑清单。
- 执行过的命令和结果。
- 是否修改 production code；默认应为否。
- 如果覆盖率目标尚未达到，列出下一批最高优先级目标。

## 推荐执行 Prompt

```text
请使用 legacy-unit-test workflow 为当前仓库建立单元测试安全网。

目标：
1. 达到客户约定的核心逻辑有效覆盖率目标，而不是追固定百分比。
2. 不修改任何生产代码行为。
3. 不做业务重构。
4. 不为了 coverage 写无意义测试。
5. 优先覆盖核心业务逻辑。
6. 使用 characterization tests 锁定当前行为。

请按以下步骤执行：

Phase 0:
- 识别项目语言、测试框架、构建命令、测试命令、coverage 命令。
- 不修改生产代码。
- 生成 docs/testing/current-baseline.md。

Phase 1:
- 扫描代码，找出高风险核心逻辑。
- 按业务风险排序，而不是按容易测试排序。
- 生成 docs/testing/risk-map.md。

Phase 2:
- 建立最小测试基础设施。
- 可以添加测试 helper、fixture、mock factory。
- 添加依赖、修改 CI、修改构建脚本或修改生产代码前，必须先说明原因并等待确认。

Phase 3:
- 选择最高优先级的 1 到 3 个模块。
- 为它们添加 characterization unit tests。
- 覆盖正常路径、边界值、异常路径和关键分支。
- 测试必须有真实断言。

Phase 4:
- 运行最小相关测试命令。
- 运行 coverage 命令。
- 更新 docs/testing/coverage-map.md。
- 如发现疑似错误行为，只记录到 docs/testing/suspicious-behavior.md，不修复。

限制：
- 禁止修改 production code，除非提前说明并获得人工确认。
- 禁止修 bug。
- 禁止改业务逻辑。
- 禁止只为了 coverage 测 getter/setter/DTO。
- 禁止删除、跳过或放宽测试。

完成后请给出：
1. 修改文件列表
2. 新增测试说明
3. 覆盖的核心逻辑
4. 执行过的命令
5. coverage 结果
6. 疑似现有行为问题
7. 下一批建议覆盖目标
```

## 风险控制结论

这套方式可以降低老项目补测试的风险，但前提是严格执行三个约束：

1. 覆盖率目标由客户要求决定，不由 AI 写死。
2. 生产代码默认只读，测试建设不混入重构和修 bug。
3. 每批只覆盖少量高价值模块，并用 coverage-map 和 suspicious-behavior 记录事实。

满足这些约束后，AI 的作用不是随机补单测，而是持续执行一个可审查、可回滚、可验收的 legacy test seeding 流程。
