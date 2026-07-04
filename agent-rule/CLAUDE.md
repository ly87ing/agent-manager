# CLAUDE.md

## Scope

- Maintain the shared core and agent adapters under `agent-rule/rules/src/`.
- Treat `agent-rule/rules/dist/` as generated artifacts; do not hand-edit them directly.

## Workflow

- After changing rules, run `make rules-build` first.
- Run `make rules:sync` when distribution is needed.
- When adjusting target paths or filenames, also check `config/rules-targets.list`.

## Guardrails

- Keep shared rules portable; leave agent-specific detail in each adapter.
- For repo-level commands or general background, return to the root `CLAUDE.md`.

## 沉淀落位:该进常驻规则还是 skill/memory

依据 Anthropic 官方 + 社区共识(精简常驻层、渐进披露):常驻规则(`core.md` 分发出的 CLAUDE.md/AGENTS.md)每条都对**每一轮**上下文征税,只装两类东西——**普适不变量** + **skill 触发指针**;判据是"删了它 agent 会不会犯错"。

- **是"事实/不变量"→ 常驻;是"流程/步骤/条件性"→ 下沉**。条件性行为习惯(如"承重结论先核一手来源")不是每轮都要的不变量,不塞 `core.md`——归对应 skill(按需加载)或 auto-memory(按相关召回)。
- **绝不把一个 skill 的多路分类压成规则里的二分法**——那是有损压缩,等于在 always-on 层立一条会被 skill 纠正的简化规则,是负分不是中性。路由行只保留 `` 遇到 X → use the `<skill>` skill `` 的指针,分类正文留给 skill。
- 触发词要点名**当前漏掉的高频具体场景**,既不搞 catch-all(稀释触发信号)也别太窄(漏触发)。

## 编辑 core.md 的隐形护栏(改前先看,否则门禁莫名报红)

- **预算硬门禁** `tests/test_rule_budgets.py`:`round(len(core.md)/4) ≤ 460`。长期贴上限跑(2026-07 时为 452,余量仅 ~8 token),任何净增字符都会破门——路由行改写须**字符中性或负增**,想加内容先压缩现有措辞腾位。
- **短语断言** `tests/test_sync_rules.py` 逐字断言四条 `` use the `change-discipline` skill ``/`` `artifact-hygiene` ``/`` `frontend-verification` ``/`` `reader-facing-writing` ``,`test_rule_layout.py` 按 ``re.findall(r"use the `([a-z0-9-]+)` skill")`` 提取——**skill 改名要同步测试**;但短语左侧的触发关键词不被断言,可自由改写。
- **反向断言** `test_rule_layout.py` 禁止分发规则出现探索/工具督促类子句(`codebase-retrieval`、`augment MCP`、`graphify-out/GRAPH_REPORT.md`、`For codebase exploration:`)。

## 别用 hook 替代路由表,别加常驻规则去"推"工具

- 那张常驻 skill 路由表是**唯一跨活跃端**生效的强制机制:hooks 是 Claude Code harness 专属,够不到同样每天在用的 Antigravity(codex 休眠)——把守门 skill 搬进 hook、砍掉 core 路由表会让路由在 Antigravity 上失效。
- MCP 工具(serena 等)自动暴露、相关时 agent 自会调用;**别加常驻规则或 hook 去督促工具使用**(重蹈已删除的 graphify 上下文膨胀覆辙,`test_rule_layout` 反向断言正是为此)。用得不够,唯一 graphify-safe 的补法是按需 skill,绝不是常驻规则。
- 路由行按**动词/动作**分流(Validate vs Write)而非共享名词,避免 `HTML`/`reports` 这类名词被两行同时认领而削弱触发确定性。
