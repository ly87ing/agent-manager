# Skill 编写标准

适用于 `agent-skills` 仓库(ly87ing/agent-skills)所有受管 skill。硬性形态契约(命名、frontmatter、
英文正文、`agents/` 四件套、≤500 行、evals)已由该仓库 `tests/test_skill_contracts.py` 编码,此处不重复;
本文只收录测试无法编码的编写判断标准。

依据:Anthropic Agent Skills 官方最佳实践(docs.claude.com → Agent Skills best practices)、
OpenAI Codex Skills 官方文档(developers.openai.com/codex/skills)。两家一致的核心:
description 决定触发(写清能力 + 何时用 + 何时不用);SKILL.md 是目录页,细节下沉 `references/`、
确定性逻辑下沉 `scripts/`(执行而非读入,只有输出进上下文);指令自包含、含验证步骤与正反例;
核心文件不放时效性事实(易变事实放 references 并标注核验日期)。

## 本仓库增补的两条治理标准

1. **补充而非替代**。领域定制 skill 只讲本地知识会把模型的通用工程判断挤掉——这是可复现的评测失败模式
   (安全左移 skill 初版只讲本公司案例,效果反而不如无 skill 对照组)。正确形态 = 通用判断 + 本地证据叠加,
   SKILL.md 必须显式写明"本 skill 补充什么、不替代什么"。
2. **本地证据优先于通用模板**。清单、权重、注意力分配按自家历史数据的真实分布校准,不照搬业界通用清单
   (如 OWASP 通用比例);每条证据给可回查的原单编号,对抗审查专抓"转述层与证据库不一致"。
   本地事故换来的知识(TR)与业内通用默认值冲突时,本地知识优先——通用默认是无信息先验,
   TR 是团队用真实事故买来的后验。

## 变更流程

dev 仓库(`~/Documents/github/agent-skills`)修改 → 增补 eval 用例 → 跑仓库契约测试 →
按仓库惯例 commit(不带 AI 署名)→ 经用户同意 push → agent-manager `make bump` + apply →
抽验各 agent 实际加载。
