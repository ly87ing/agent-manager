# contract/ — 引擎契约（Engine Contract）

契约是整个项目最重要的产物：**一份语言中立的 JSON Schema**，Python 与 Rust 实现
都受同一份契约测试检验，GUI / CLI / CI 只依赖契约、不依赖实现
（见 `docs/GUI-ARCHITECTURE-PROPOSAL.md` §2 与 `docs/goal-rust-gui.md` 已决清单）。

## 目录布局与版本化

```
contract/
  engine-v1/            # Engine Contract v1，一个版本一个目录
    envelope.schema.json
    element.schema.json
    plan-action.schema.json
    status.schema.json
    plan.schema.json
    apply-result.schema.json
    converge.schema.json
    runs.schema.json
    snapshots.schema.json
    policy.schema.json
    profile.schema.json
```

- 版本号锚点有二：目录名（`engine-v1`）与 envelope 的 `contract` 常量
  （`"agent-manager/engine/v1"`）。两者必须同步演进。
- **冻结规则**：在第一个引擎命令（Phase 0 的 `<domain>:status`）落地之前，v1 允许
  就地修订；落地之后 v1 冻结，除勘误（不改语义的 description 修订）外一律通过
  新增 `engine-v2/` 目录演进，新旧目录并存，客户端按 `contract` 常量识别版本。

  **勘误（2026-09-07，P4a）**：冻结针对既有字段的语义；**向后兼容的 additive
  可选字段允许**——即只新增不出现在 `required` 里的字段、且 `additionalProperties`
  形状不放宽。首例：`profile.schema.json` 新增可选 `diff` 数组（profile:status
  的 per 顶层键分叉视图；旧客户端忽略该键即可，义务不变）。
- envelope 的 `data` 在信封层只要求 object；具体形状由各域 schema 细化：
  `status.schema.json` / `plan.schema.json` 校验的是 **envelope.data** 而非整个信封。

## Schema 清单

| 文件 | 校验对象 | 提案出处 |
|---|---|---|
| `envelope.schema.json` | 所有命令 stdout 的统一信封（contract/run_id/command/mode/lock/log_file/ok/data/warnings/errors） | §2.2 |
| `element.schema.json` | 统一元素模型：id/domain/spec/policy/observed/state + freshness + actions | §3（§3.1 七态、§3.3 五态） |
| `plan-action.schema.json` | plan 动作条目：op/element/target/risk/摘要字段 | §2.4 |
| `status.schema.json` | `<domain>.status` 的 data：`{domain, elements[]}` | §2.2/§2.3 |
| `plan.schema.json` | `<domain>.plan` 的 data：`{domain, actions[], summary}` | §2.2/§2.4 |
| `apply-result.schema.json` | `<domain>.apply` 的 data：`{domain, dry_run, steps[], snapshot, skipped[]}`（复用既有同步器，写前快照） | §2.3/§4.10 |
| `converge.schema.json` | `system.converge` 的 data：`{pipeline, dry_run, steps[], snapshot, skipped[]}`（update 管线等价） | §2.3 |
| `runs.schema.json` | `runs.list` / `runs.get` 的 data：`{runs[], tail, lines}`（get 时 runs 为单元素数组） | §2.3/§4.8 |
| `snapshots.schema.json` | `snapshots.list` / `snapshots.restore` 的 data：`{snapshots[] \| restore}`（restore 前先再快照当前态） | §4.10 |
| `policy.schema.json` | `policy.get` / `policy.set` 的 data：三层原文 + effective + 层归属 / 写入回执（baseline 只读） | §7.2/§4.9 |
| `profile.schema.json` | `profile.status/pull/push` 的 data：后端状态/ahead/behind/冲突/门禁回执 | §7.2/§4.11 |

## 关键字白名单（子集）

schema 文件只允许使用以下 JSON Schema 关键字，由 `tests/contract_validator.py`
的 `SCHEMA_KEYWORDS` 定义并在校验前强制执行（越白名单直接 `SchemaError`）：

`type`、`properties`、`required`、`additionalProperties`、`items`、`enum`、
`$defs`、`$ref`、`description`

明确禁用：`oneOf` / `anyOf` / `allOf` / `not` / `pattern` / `format` / `minimum` /
`minLength` / `$id` 等一切白名单外关键字。取舍：白名单小到可以让
`tests/contract_validator.py`（纯 stdlib，CI 3.9–3.11 无 jsonschema）与未来 Rust
侧的等价校验器都保持数百行以内，从而降低双实现漂移风险。

补充约定：

- `$ref` 只允许**文件内本地引用**（`#/...` JSON Pointer，从文档根解析）；
  禁止跨文件引用。含 `$ref` 的节点不得携带其他校验关键字（`description` 与
  定义容器 `$defs` 除外）。
- `additionalProperties` 本仓库 schema 一律用 `false`（形状显式、漂移可见）。
- 可缺值的字段一律 **required 但 nullable**（`"type": ["...", "null"]`），
  保证 payload 形状统一、客户端无需做键存在性判断。适用：`lock.pid`、
  `log_file`、diagnostic 的 `element`、freshness 的五个值字段、plan 动作条目的
  `target` / `before_digest` / `after_digest` / `backup_path` / `cmd` / `note`。

## 跨文件复用的处理：内联拷贝 + 测试强制等价

`status.schema.json` 需要元素模型、`plan.schema.json` 需要 plan 动作条目，而跨文件
`$ref` 被禁止。方案是**直接内联 `$defs` 拷贝**，不引入拼装脚本：

- 单一事实源仍是 `element.schema.json` / `plan-action.schema.json`；
- 拷贝侧在 description 中注明来源；
- `tests/test_engine_contract.py` 的
  `test_status_schema_inlines_element_schema_verbatim` /
  `test_plan_schema_inlines_plan_action_schema_verbatim`
  逐字段断言拷贝与源相等——改源不改拷贝会让契约测试变红。

不选脚本拼装的理由：仓库对生成产物有严格边界（见根 AGENTS.md），脚本拼装会引入
"源文件 + 生成文件"双层结构；内联拷贝加等价测试以零构建步骤达到同等一致性。

## Fixtures 纪律

fixtures 位于 `tests/fixtures/engine-v1/`，命名：

- 正例：`<schema>-<case>.valid.json`（每 schema 至少 2 个：最小 + 完整）；
- 反例：`<schema>-<违反点>.invalid.json`（每 schema 至少 2 个），按违反点命名，
  且必须在 `tests/test_engine_contract.py` 的 `EXPECTED_INVALID_PATHS` 登记预期
  失败路径（如 `$.elements[0].freshness.state`）；测试断言反例恰好挂在
  该路径上。

fixtures 只允许占位符与示例值，禁止任何真实凭证/主机/域名（`contract/` 与
`tests/fixtures/` 均在 `scripts/check_no_secrets.py` 扫描范围内）。

## 新增 / 演进 schema 的流程

1. 在版本目录新增（或冻结后在新版本目录演进）`*.schema.json`，只用白名单关键字；
2. 配 fixtures：≥2 正例 + ≥2 反例，反例按违反点命名并登记预期路径；
3. 在 `tests/test_engine_contract.py` 的 `SCHEMA_FILES` / `EXPECTED_INVALID_PATHS`
   注册；若 schema 间有复用，按「内联拷贝 + 等价测试」处理；
4. 更新本 README 的 Schema 清单；
5. 验收：`python3 -m unittest tests.test_engine_contract -v` 全绿，再跑
   `python3 -m unittest discover -s tests -p 'test*.py'` 与
   `python3 scripts/check_no_secrets.py`。

## 双实现对齐约定（Python / Rust）

- Python 引擎（`manage.sh` / `src/`）的每个新命令输出，必须用
  `tests/contract_validator.py` 以同一份 fixtures + schema 校验；
- Rust 切片（Phase 3，按提案 §6.3 顺序替换）须实现等价的白名单校验器，消费**同一份**
  `tests/fixtures/engine-v1/` 与 `contract/engine-v1/`；每个切片以「同 fixture 下
  输出 schema 等价」为退役 Python 模块的前置条件；
- 两侧共享的只有 schema 与 fixtures 两类文件；校验器各自实现，互不依赖。
