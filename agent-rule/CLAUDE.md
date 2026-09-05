# CLAUDE.md

## Scope

- Maintain the shared core and agent adapters under `agent-rule/rules/src/`.
- Treat `agent-rule/rules/dist/` as generated artifacts; do not hand-edit them directly.

## Workflow

- After changing rules, run `make update`（构建 `agent-rule/scripts/build-agent-rules.py` 与分发随管线完成）。
- When adjusting target paths or filenames, also check `config/rules-targets.list`.

## Guardrails

- Keep shared rules portable; leave agent-specific detail in each adapter.
- For repo-level commands or general background, return to the root `CLAUDE.md`.

## 沉淀落位:该进常驻规则还是 skill/memory

依据 Anthropic 官方 + 社区共识(精简常驻层、渐进披露):常驻规则(`core.md` 分发出的 CLAUDE.md/AGENTS.md)每条都对**每一轮**上下文征税,只装两类东西——**普适不变量** + **skill 触发指针**;判据是"删了它 agent 会不会犯错"。

- **是"事实/不变量"→ 常驻;是"流程/步骤/条件性"→ 下沉**。条件性行为习惯(如"承重结论先核一手来源")不是每轮都要的不变量,不塞 `core.md`——归对应 skill(按需加载)或 auto-memory(按相关召回)。
- **绝不把一个 skill 的多路分类压成规则里的二分法**——那是有损压缩,等于在 always-on 层立一条会被 skill 纠正的简化规则,是负分不是中性。路由行只保留 `` 遇到 X → use the `<skill>` skill `` 的指针,分类正文留给 skill。
- 触发词要点名**当前漏掉的高频具体场景**,既不搞 catch-all(稀释触发信号)也别太窄(漏触发)。

## delete-test 量化设施已下线，保留实测结论与纪律

判据"删了它 agent 会不会犯错"缺两个主语:**哪个 runtime 上的哪一代模型**、**截至何时**。两者都在动(Anthropic 已删掉 Claude Code 出厂系统提示 80%+,被覆盖的面在缩小),所以一次性证明会过期,而字符预算门禁只防新增、永远不会为「存量变冗余」报警。

- `make rules:audit` 与 `tests/run_rule_behaviour_evals.py` 已随 2026-09 的命令精简下线（连同 `scripts/evals/` 离线评测设施），以下仅保留当时实测沉淀的结论与纪律；如需复核请从 git 历史恢复工具。
- 实测该行的 lift 时曾用 `with`=完整 core.md、`without`=删掉该行的 core.md 由判官打分;`without` 已过 = delete-test 正在触发。用例写在 `agent-rule/rules/evals/core.json`,`anchor` 必须唯一命中 core.md 一行(命中 0 或多行直接报错,防止静默测错行);**改写行文须同步 anchor**。
- **两臂默认都装着 skill 目录,这是修 bug 不是加功能**:规则要去的真机上,5 个 skill 的 description 永远在系统提示里。把规则单独拎到空环境里测,量的是一个不存在的世界——裸模型下有 lift 的行,挨着一个已经认领同一片地的 skill 时可能就是冗余;路由表四行正是这个问题的主场(它们的全部工作就是指向一个**已经装好**的 skill),实测结果见下面「别用 hook 替代路由表」一节。2026-07-27 之前记录的所有 lift 都出自裸模型臂,与装 skill 臂的数字不可比。
- **别把 Claude 上的 no-lift 直接读成「可以删」**:core.md 由 Claude Code 与 Codex 共享,而当时的 runner 只测 Claude Code。一条在 Claude 上冗余的行,在 Codex 上仍可能承重。**只有当替代它的机制本身跨端存在**(例如两端都安装的 skill description),Claude 上的结论才可外推。
- **隔离是必须的,不是可选项**:规则已分发到 `~/.claude/CLAUDE.md`,直接跑 `claude -p` 两臂都读得到,测出来恒为 0;`--system-prompt` 挡不住 CLAUDE.md,`--bare` 不读 OAuth 登录不了——两条路都实测否决过,别重走。临时 HOME 里拷进来的 `.claude.json` 会带着整台机器的 MCP 列表,两臂都需 `--strict-mcp-config`,否则每次调用都重新拉起 MCP;代价是「该优先用哪个工具」这类规则测不了——臂里根本没有 MCP 工具。
- 读数纪律与已否定的候选**不在这里重复一份**:唯一出处是 `agent-skill/repositories/agent-skills/README.md` 的 "rules for reading trigger-eval results" 与 "Already-tested dead ends" 两节(判官模型、N≥9、比 Wilson 区间、哪些 description 候选与拆分实验已实测否决)。规则层与 skill 层共用同一套读数规则,改了那里即可,本节只留规则层特有的部分。

## 编辑 core.md 的隐形护栏(改前先看,否则门禁莫名报红)

- **预算硬门禁** `tests/test_rule_budgets.py`:`round(len(core.md)/4)` 有硬上限(**数字以该测试的常量为准,别在这儿抄第二份**)。core 长期贴着上限跑,任何净增字符都可能破门——**改前先跑该测试看实测余量**;路由行改写须**字符中性或负增**,想加内容先压缩现有措辞腾位。
- **抬上限的判据是 delete-test,不是「我加的是 always-on 边界」**——后者恰恰是 bd139d5 对自己的定性:它新增常驻条目并抬 420→460,三个 commit 后 6dd67bb 判定该行(core.md 第 11 行)是低频条件性 when-X-do-Y、过不了 delete-test,遂删行并**对称回退** 460→420,明写"恢复「净增即破门」的强制函数"。规矩:先用 delete-test(删了它 agent 会不会犯错)证明该行配占常驻槽位再抬,且**抬升与该行同生共死**——行下线时上限一并回退;绝不为让门禁变绿而抬。
- **短语断言** `tests/test_sync_rules.py` 逐字断言四条 `` use the `change-discipline` skill ``/`` `artifact-hygiene` ``/`` `frontend-verification` ``/`` `reader-facing-writing` ``,`test_rule_layout.py` 按 ``re.findall(r"use the `([a-z0-9-]+)` skill")`` 提取——**skill 改名要同步测试**;但短语左侧的触发关键词不被断言,可自由改写。
- **反向断言** `test_rule_layout.py` 禁止分发规则出现探索/工具督促类子句(`codebase-retrieval`、`augment MCP`、`graphify-out/GRAPH_REPORT.md`、`For codebase exploration:`)。

## 规则与 skill 里一律不放注释(用户 2026-07-27 拍板,别再拿"这家免费"来复议)

- **门禁**:`tests/test_rule_layout.test_rules_carry_no_comment_content_anywhere` 覆盖 `core.md`、三份 adapter 与三份 dist 产物;skill 侧对应 `agent-skills/tests/test_skill_contracts.test_skill_content_carries_no_comments`(SKILL.md + references,代码围栏内豁免)。给维护者看的话写进仓库文档,那是运行时不加载的地方。
- **"某家会剥离所以免费"不是理由,这条已经试过并被推翻**:同一份文件发给两家。Claude Code 注入 CLAUDE.md 前确实会剥掉块级 HTML 注释(实测 CLI 2.1.220,单行与跨行都剥,`~/.claude/rules/*.md` 也一样),但 Codex 没有同等行为、每一轮都要计费,而且即便在 Claude 上,只要有 agent 用 Read 打开该文件,注释就照样进上下文。为一家的豁免维护一套例外,不值。
- 这条同时封掉了"用注释给分发产物自报家门"的做法:`~/.claude/CLAUDE.md` 被工具整体接管、覆盖写不留 `.bak` 这件事,只能靠仓库文档和分发目标本身的选择解决,不能靠往产物里塞注释。

## 别用 hook 替代路由表,别加常驻规则去"推"工具

- **路由表 2026-07-27 第一次被实测,两条行结论不同**(sonnet,每行 2 用例 × 9 次 = N=18,两臂都装着 skill):`route-change-discipline` with 18/18 / without 15/18(+0.17,区间重叠);`route-artifact-hygiene` with 18/18 / **without 18/18**——裸臂一次没错过,delete-test 按仓库自己的判据是**触发了**的。
- **但 delete-test 触发不等于该删,这是对既有判据的修正**:留着一行路由的代价是每会话约 28 token,删错的代价是 artifact-hygiene 那类边界(产物落位、密钥进日志)在边缘措辞上不再被认领。**代价不对称时,证据要求也应该不对称**——两个用例、一次会话的 null 不足以换掉一条按设计承重的行。想真正结掉这个问题,需要的是更边缘的措辞用例(不是更多次重复)、以及另外两条路由行(`frontend-verification`/`reader-facing-writing`)也各自测一遍;在那之前四行都留着,四条逐字断言也别动。
- 那张常驻 skill 路由表是**唯一跨两端**生效的强制机制:hooks 是 Claude Code harness 专属,够不到 Codex——把守门 skill 搬进 hook、砍掉 core 路由表会让路由在 Codex 上失效。
- MCP 工具(serena 等)自动暴露、相关时 agent 自会调用;**别加常驻规则或 hook 去督促工具使用**(重蹈已删除的 graphify 上下文膨胀覆辙,`test_rule_layout` 反向断言正是为此)。用得不够,唯一 graphify-safe 的补法是按需 skill,绝不是常驻规则。
- 路由行按**动词/动作**分流(Validate vs Write)而非共享名词,避免 `HTML`/`reports` 这类名词被两行同时认领而削弱触发确定性。
