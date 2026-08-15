# Skill 提示词审计报告（2026-08-04）

**结论**：agent-manager 5 个 skill 与 xiaoyu-dev 8 个 skill 整体质量良好（禁止项带理由、安全门规范、几乎无压力语言堆砌），不需重写。主要病灶是**同一规则多处全文复述**与**事故叙事固化**；另发现并已修复一处运维问题（~/.xiaoyu-dev 安装库落后源库 3 个提交，运行版缺 Behavior/Test Focus 门禁）。本次已执行高优先级三项（见「已执行」），其余列入待办。

审计方式：两个只读子代理分别通读两仓库全部 SKILL.md 与 references，依据 Anthropic prompt-audit 方法（区分"业务契约/安全门→保留"与"为老模型写的脚手架→删改"），目标模型 Claude Opus 5 / Fable 5。触发频率取自本机 2026-07-27 ~ 08-04 全部 Claude Code 会话记录。

## 触发频率（决定优化优先级）

9 天内 Skill 调用：reader-facing-writing **44**、change-discipline 10、xiaoyu-code-submit 10、ones 6、artifact-hygiene 5、gitlab 4、ones-manhour-fill 4、frontend-verification 3、remote-ops 3、safe-merge-review 1、bug-handler 1、**xiaoyu-bug-loop 0**。

- 频率 × 正文体量 = 单位收益：reader-facing-writing（44 × 16KB）居首。
- 所有 skill 的 frontmatter description 常驻每次请求（自定义 12 个合计约 4.4k tokens），与触发与否无关。

## 关键校准：description 不能一刀切压缩

agent-manager 仓库自带触发评测体系（agent-skills/README 记录 1024 字符宿主截断契约、front-load 规则、已实测否决的压缩候选）。**其 5 条长 description（816–1014 字符）是逐案例调优的局部最优，不做盲目压缩**；唯一例外是 safe-merge-review 有一条经评测的候选改写（可省约 100 字符，须过评测后落地）。xiaoyu-dev 无评测体系，其 description 属触发短语枚举，可安全改写（bug-loop、code-submit 改写稿见下）。

## agent-manager / agent-skills 审计发现

| Skill | 判断 | 主要发现与改法 |
|---|---|---|
| artifact-hygiene（9.1KB） | 健康，小修 | L17 同一原则连说三遍→压一句+理由；L21 vs L49 复用测试与 Hard Boundaries 重叠→归置规则单一出处。保留：密钥不入日志/降权回收、filter-repo 先备份 |
| change-discipline（9.3KB+2 refs） | 健康，小修 | L25–27 三条 escalation 规则→26/27 合并；「A real case:…」×6→每规则留一例。description 经评测为局部最优，勿改 |
| frontend-verification（7.7KB+2 refs） | 健康 | SKILL.md:34 与 visuals-and-decks:42「OS-level full screen…glare」近逐字两份→阈值留本 skill；playwright-essentials 通用知识（locator 优先级、web-first 断言）砍 ~50%，只留 storageState 保密、mock 边界、flaky 分类。playwright-toolchain 的 INSTALLATION_COMPLETE/30s 静默超时/exit 137 是经验性失败模式，保留 |
| reader-facing-writing（16KB，贴预算上限） | 需中修 | 「demote don't delete」4+ 处→单一出处；指向 reference 的三段复述其目录→砍半留触发条件+关键词；L40/44/45 超长 bullet 拆短。**已执行，见下** |
| safe-merge-review（11.5KB+2 refs） | 需中修，重复最重 | 同一字段表三份（SKILL.md:125–141 / checklist§7 / build_merge_evidence.py）→单源到脚本；checklist§6 与 workflow§7 完整性验证整节重复→删 §6 指向 §7；AI-provenance 禁令近逐字三份→skill 内留一份（core.md 份是 always-on 不变量，保留）；workflow§1–3.3 的 merge-base/left-right 释义属模型已知→砍 ~40% 保命令删解释。保留：§3.4 平行基线按内容判定、§7.5 squash 三法+cherry-pick-noop 协议 |
| agent-rule/ | 健康勿动 | core.md 每行经预算门禁+delete-test 背书；hooks 仅注入 1–2 句。CLAUDE.md(9.6KB) 中 MCP 延迟叙事等过程描写可砍半（约 −1.5KB），commit id/日期等取证锚点保留 |

预计可削减：按需加载正文约 18–19KB（~25%）；常驻 description 不削。

## xiaoyu-dev / skills 审计发现

| Skill | 判断 | 主要发现与改法 |
|---|---|---|
| xiaoyu-bug-loop（SKILL 43KB + refs ~131KB） | 骨架优秀，复述严重 | 相对迭代解析在 G10/Phase 0/D11 三处全文重复约 6KB→唯一保留 D11；空范围门出现 4 次→收敛 1 处；「Loop controls」~3.5KB 调度/模型选型建议属运维文档→移 README；一次性测量数据（"nine months apart"迭代名）→删。mode-fix：约束 7/9 嵌入整段案例复盘、约束 11 与 E2 双写、「Experience distilled」保姆段→删或压 2 行。mode-verify：§1/§3 大量转述 G3/G5→改引用。known-pitfalls **判定为活契约非纪念碑**（附件 ~80 字上限、transition uuid 随 workflow 变化、name_match 跨团队不可靠、--since/--until 丢 commit 均保留）；Z2/Z7 自认重复→合并；D11 的 111 迭代测量表→压为原则+一行示例。预估：SKILL −35%、mode-fix −30%、known-pitfalls −25%、mode-verify −20% |
| xiaoyu-code-submit（15.5KB+refs） | 全套里最适配新模型 | description 压力开头（"Mandatory…no exception"）→平实化；§9 Final self-check 13 项与 SKILL.md Stop Conditions 近 1:1 重复且对 Fable 5 诱发过度验证→删（逐项核实均有原生出处）。保留：秘密扫描、AI provenance 剥离、`[#id]`+40 字符服务端 hook、码点/UTF-16 计数差异。**已执行，见下** |
| xiaoyu-security-review（20KB+refs ~105KB） | 一处大头 | references/ones-defect-evidence.md **78KB 判定为历史证据堆积**（833 条一行式 bug 标题）：活价值仅头部分布/趋势表、各节导语、§② 教训段→移出 skill 至知识库，包内留 ~8KB 摘要。「只报本次变更新引入」三处复述→收敛。保留：置信度 ≥8 阈值、14 条硬排除、TR>硬排除仲裁、远端取数门禁 |
| xiaoyu-monitoring-gen（23KB） | 双写重 | 17 条铁律与 Phase 正文双写（选择器三分叉等 2–3 遍）；Phase 4 手写 4 步验证循环→改「validate.py 无 FAIL 才算完成」。可削 35–45% |
| xiaoyu-generate-api-detail（23KB） | 双写最重 | 步骤 5 确认关口四处重复（含 🔴 强调）；v1/v2 迁移史与带日期示例分支名属易腐。可削 40–50% |
| xiaoyu-compatibility-review（13KB） | 三者最佳 | 命令基本带理由，阶段五是真方法论；仅阶段一逐命令细节可下沉 reference。可作重写范式。可削 10–15% |

### description 改写稿（待评测/已用）

bug-loop（~890→~520 字符，未落地，建议配套触发验证）：

> Batch-process one developer's ONES bugs. Full mode analyzes New/In Progress bugs and comments + transits them; code changes, commits and Resolved happen only on explicit "自动修复". Verify mode proves deployed Resolved fixes with UI evidence matching each bug's exact scenario, then routes passing bugs to their filing QA — no matching evidence, no routing. Scope defaults to all the current user's actionable bugs across sprints; another named person is read-only unless writes are authorized. Single-bug root cause → bug-handler; ONES-only ops → ones.

code-submit：已落地（见下），在审计稿基础上保留了 "including small 'just commit this' changes" 触发线索。

## 已执行（2026-08-04）

1. **同步安装库**：~/.xiaoyu-dev `git pull --ff-only` 追平 origin（3936f25），4 处漂移文件对齐，`diff -rq` 零差异。注意：本次源库新改动需 commit+push 后再 pull 一次才会到达安装库。
2. **reader-facing-writing 去重**（agent-skills 仓库）：9 处编辑，16060→15150 字节。demote 规则收敛到 L37 单一出处；prose-style/revision-pass/visuals 三个指针段砍半（触发条件保留在 SKILL.md，遵守 README 2026-07-29 实测警示：reference-only 规则不保证生效）；文档集/命名实体/组织逻辑三条长 bullet 删内部复述。离线 23 项契约测试通过；行为评测未跑（改动未触碰 description 与规则本体，触发评测不受影响）。
3. **xiaoyu-code-submit**（xiaoyu-dev 仓库）：description 843→约 600 字符去压力化；删除 §9 Final self-check（13 项逐一核实在 SKILL.md/其余章节均有原生出处）；同步清理目录行与 SKILL.md:69 引用。该仓库的检查目标全部通过。

## 第二批执行（2026-08-04，原待办 1–5 全部完成）

三个并行编辑代理执行、合并态复验后按 skill 粒度提交推送（agent-skills e7db98e..46b9d71 四提交；xiaoyu-dev 16dd2ba..39b49e1 四提交；~/.xiaoyu-dev 已同步）：

- **safe-merge-review**：字段表单源化（脚本为骨架权威、SKILL.md 为语义唯一出处）、checklist§6/§7 删除改指向、AI-provenance 收敛、workflow 纯概念释义删净（节内字节 −20%，未达字面 40% 因余量均为保留清单内容）
- **bug-loop**：description 换稿落地；SKILL.md 43.1→36.1KB（−16%），相对迭代解析收敛至 D11 单源、空范围门收敛至 G10、Loop controls 移仓库 README；mode-fix −2.2KB（案例复盘压缩、Experience distilled 压 2 行）；known-pitfalls Z2/Z7 合并、D11 测量表压为原则；mode-verify 转述改引用
- **security-review**：78KB 证据底稿 git mv 至 docs/ 保全历史，包内留 13.6KB 摘要（分布/趋势表+代表案例+教训段）；description 340→165 字符；"只报新引入"收敛
- **generate-api-detail**：确认关口收敛至步骤 5 单源、🔴 删净、迁移史压一句、日期示例改占位符（−0.9KB）
- **monitoring-gen**：铁律与 Phase 双写收敛、Phase 4 循环替换为 validate.py 无 FAIL 单一判据（保留"不得为全绿删检查项"安全门）（−1.0KB）
- **playwright-essentials** −33%（三块经验性内容整体保留故未达 50%）；**change-discipline** L26/27 合并+案例每规则一例；**artifact-hygiene** 三连述压缩；**visuals-and-decks** 与 frontend-verification 投影规则收敛

验证：agent-skills 23 项契约测试、xiaoyu-dev 仓库检查均在合并终态全绿；安全门/硬约束抽查（⛔×5、G10、Z7 无残留、monitoring 安全门）通过。

## 剩余可选项

- agent-rule/CLAUDE.md 过程叙事砍半（−1.5KB）
- bug-loop 如长期不用可加 `.default-off` 免除常驻描述（触发频率见上，9 天 0 次）
- safe-merge-review description 的评测候选改写（须先跑触发评测）
- 行为评测复跑（run_behaviour_evals.py 与 xiaoyu 的 output-quality 评测，需调模型）确认压缩无行为回退

执行约定：agent-skills 仓库改动过其评测（tests/ + run_trigger_evals）；xiaoyu-dev 一次只改一个 skill，git 保留旧文案便于回退。保留清单（勿删）：ONES 状态机与 ⛔ 安全门、G1–G10、秘密扫描/AI provenance 剥离、known-pitfalls 环境契约、agent-rule/core.md 全部、frontend-verification 的经验性失败模式。
