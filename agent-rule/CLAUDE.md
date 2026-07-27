# CLAUDE.md

## Scope

- Maintain the shared core and agent adapters under `agent-rule/rules/src/`.
- Treat `agent-rule/rules/dist/` as generated artifacts; do not hand-edit them directly.

## Workflow

- After changing rules, run `make rules-build` first.
- Run `make rules:apply` when distribution is needed.
- When adjusting target paths or filenames, also check `config/rules-targets.list`.

## Guardrails

- Keep shared rules portable; leave agent-specific detail in each adapter.
- For repo-level commands or general background, return to the root `CLAUDE.md`.

## 沉淀落位:该进常驻规则还是 skill/memory

依据 Anthropic 官方 + 社区共识(精简常驻层、渐进披露):常驻规则(`core.md` 分发出的 CLAUDE.md/AGENTS.md)每条都对**每一轮**上下文征税,只装两类东西——**普适不变量** + **skill 触发指针**;判据是"删了它 agent 会不会犯错"。

- **是"事实/不变量"→ 常驻;是"流程/步骤/条件性"→ 下沉**。条件性行为习惯(如"承重结论先核一手来源")不是每轮都要的不变量,不塞 `core.md`——归对应 skill(按需加载)或 auto-memory(按相关召回)。
- **绝不把一个 skill 的多路分类压成规则里的二分法**——那是有损压缩,等于在 always-on 层立一条会被 skill 纠正的简化规则,是负分不是中性。路由行只保留 `` 遇到 X → use the `<skill>` skill `` 的指针,分类正文留给 skill。
- 触发词要点名**当前漏掉的高频具体场景**,既不搞 catch-all(稀释触发信号)也别太窄(漏触发)。

## delete-test 现在可以量,别再只靠判断

判据"删了它 agent 会不会犯错"缺两个主语:**哪个 runtime 上的哪一代模型**、**截至何时**。两者都在动(Anthropic 已删掉 Claude Code 出厂系统提示 80%+,被覆盖的面在缩小),所以一次性证明会过期,而字符预算门禁只防新增、永远不会为「存量变冗余」报警。

- `make rules:audit` — 只读报告,列出 core.md 每行的引入日期与 behaviour eval 覆盖情况。**不是门禁**(过期是日历函数,写进 unittest 就是会自己变红的坏测试)。
- `python3 tests/run_rule_behaviour_evals.py --rule <id> --runs-per-case 3` — 实测该行的 lift:`with`=完整 core.md,`without`=删掉该行的 core.md,判官打分。`without` 已过 = delete-test 正在触发。用例写在 `agent-rule/rules/evals/core.json`,`anchor` 必须唯一命中 core.md 一行(命中 0 或多行直接报错,防止静默测错行);**改写行文须同步 anchor**。
- **隔离是必须的,不是可选项**:规则已分发到 `~/.claude/CLAUDE.md`,直接跑 `claude -p` 两臂都读得到,测出来恒为 0。runner 用临时 HOME(拷 `~/.claude.json` + 软链 `~/Library/Keychains`,否则报 Not logged in)。`--system-prompt` 挡不住 CLAUDE.md,`--bare` 不读 OAuth 登录不了——两条路都实测否决过,别重走。
- 与 `make eval-rules`(`scripts/evals/run_rule_effectiveness_eval.py`)分工:那个是离线确定性覆盖矩阵、进 CI;这个真调模型、慢且有噪声,**手动跑**,别塞进 CI。
- 读数遵循既有纪律:不用最便宜的模型定罪,N≥9,比 Wilson 区间而非点估计。

## 编辑 core.md 的隐形护栏(改前先看,否则门禁莫名报红)

- **预算硬门禁** `tests/test_rule_budgets.py`:`round(len(core.md)/4)` 有硬上限(**数字以该测试的常量为准,别在这儿抄第二份**)。core 长期贴着上限跑,任何净增字符都可能破门——**改前先跑该测试看实测余量**;路由行改写须**字符中性或负增**,想加内容先压缩现有措辞腾位。
- **抬上限的判据是 delete-test,不是「我加的是 always-on 边界」**——后者恰恰是 bd139d5 对自己的定性:它新增常驻条目并抬 420→460,三个 commit 后 6dd67bb 判定该行(core.md 第 11 行)是低频条件性 when-X-do-Y、过不了 delete-test,遂删行并**对称回退** 460→420,明写"恢复「净增即破门」的强制函数"。规矩:先用 delete-test(删了它 agent 会不会犯错)证明该行配占常驻槽位再抬,且**抬升与该行同生共死**——行下线时上限一并回退;绝不为让门禁变绿而抬。
- **短语断言** `tests/test_sync_rules.py` 逐字断言四条 `` use the `change-discipline` skill ``/`` `artifact-hygiene` ``/`` `frontend-verification` ``/`` `reader-facing-writing` ``,`test_rule_layout.py` 按 ``re.findall(r"use the `([a-z0-9-]+)` skill")`` 提取——**skill 改名要同步测试**;但短语左侧的触发关键词不被断言,可自由改写。
- **反向断言** `test_rule_layout.py` 禁止分发规则出现探索/工具督促类子句(`codebase-retrieval`、`augment MCP`、`graphify-out/GRAPH_REPORT.md`、`For codebase exploration:`)。

## 别用 hook 替代路由表,别加常驻规则去"推"工具

- 那张常驻 skill 路由表是**唯一跨活跃端**生效的强制机制:hooks 是 Claude Code harness 专属,够不到同样每天在用的 Antigravity(codex 休眠)——把守门 skill 搬进 hook、砍掉 core 路由表会让路由在 Antigravity 上失效。
- MCP 工具(serena 等)自动暴露、相关时 agent 自会调用;**别加常驻规则或 hook 去督促工具使用**(重蹈已删除的 graphify 上下文膨胀覆辙,`test_rule_layout` 反向断言正是为此)。用得不够,唯一 graphify-safe 的补法是按需 skill,绝不是常驻规则。
- 路由行按**动词/动作**分流(Validate vs Write)而非共享名词,避免 `HTML`/`reports` 这类名词被两行同时认领而削弱触发确定性。
