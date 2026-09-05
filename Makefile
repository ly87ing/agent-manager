.PHONY: init apply bump update plan agents-apply agents\:apply agents-status agents\:status sys-upgrade sys\:upgrade smoke ui help help-all help\:all doctor status detect fetch-models eval-rules sys-clean sys\:clean state-clean state\:clean sys-unlock sys\:unlock sys-logs sys\:logs sys-clean-logs sys\:clean-logs state-list state\:list state-restore state\:restore rules-build rules\:build rules-apply rules\:apply rules-audit rules\:audit skills-apply skills\:apply skills-bump skills\:bump skill-disable skill\:disable skill-enable skill\:enable skill-list skill\:list mcp-apply mcp\:apply tools-apply tools\:apply browser-apply browser\:apply claude-hooks-apply claude\:hooks\:apply claude-statusline-apply claude\:statusline\:apply claude-statusline-wire claude\:statusline\:wire claude-shell-aliases-apply claude\:shell-aliases\:apply claude-uninstall claude\:uninstall claude-clean claude\:clean codex-hooks-apply codex\:hooks\:apply secrets-check secrets\:check instructions-check instructions\:check

# 默认目标：显示帮助，不执行任何同步操作
.DEFAULT_GOAL := help

# 新机器初始化或全量收敛
init:
	@./manage.sh init

# apply：把锁定态分发/收敛到本机（rules/skills/mcp/tools/browser/alias）；不刷版本锁、不装 Agent
apply:
	@./manage.sh apply

# 纯粹刷新版本锁与 skill commit（更新元数据到锁定文件），不执行本地环境应用
bump:
	@./manage.sh bump

# update：日常最常用。刷新元数据后再分发/收敛到本机（= bump + apply），不安装缺失 Agent
update:
	@./manage.sh update

# 预演一次完整 init
plan:
	@./manage.sh plan

# 仅对齐已安装 Agent 的生命周期，不安装缺失 Agent
agents-apply agents\:apply:
	@./manage.sh agents:apply

# 查看已安装 Agent 的生命周期状态
agents-status agents\:status:
	@./manage.sh agents:status

# 仅更新 agent-manager 仓库自身逻辑
sys-upgrade sys\:upgrade:
	@./manage.sh sys:upgrade

# 跨平台 smoke 验证（安全预演路径）
smoke:
	@./scripts/ci-smoke.sh

# 主仓库明文敏感信息扫描
secrets-check:
	@python3 ./scripts/check_no_secrets.py

secrets\:check:
	@python3 ./scripts/check_no_secrets.py

# 启动交互式管理界面
ui:
	@./manage.sh ui

# 系统健康深度诊断
doctor:
	@./manage.sh doctor

# 快速查看状态 (同 doctor)
status: doctor

# 全量大扫除 (清理备份、日志、残留)
sys-clean sys\:clean:
	@./manage.sh sys:clean

# 删除受管 Agent home 下超过 30 天的 .bak.* 文件
state-clean state\:clean:
	@find ~/.claude ~/.codex -maxdepth 2 -type f -name '*.bak.*' -mtime +30 -print -delete 2>/dev/null || true

# 强制解除资源锁
sys-unlock sys\:unlock:
	@./manage.sh sys:unlock

# 历史配置备份清单

state-list state\:list:
	@./manage.sh state:list

# 交互式备份恢复
state-restore state\:restore:
	@./manage.sh state:restore

# 环境探测
detect:
	@./manage.sh detect

# 探测并获取支持的模型列表 (用法: make fetch-models url=<API地址> key=<API_KEY>)
fetch-models:
	@./manage.sh fetch-models $(url) $(key)

# 查看最近同步日志
sys-logs sys\:logs:
	@./manage.sh sys:logs

# 清理日志
sys-clean-logs sys\:clean-logs:
	@./manage.sh sys:clean-logs

# 独立同步任务
rules-build:
	@./manage.sh rules:build

rules\:build:
	@./manage.sh rules:build

rules-apply rules\:apply:
	@./manage.sh rules:apply

# 常驻规则体检：每行的引入日期与 behaviour eval 覆盖情况（只读报告，不改任何文件）
rules-audit rules\:audit:
	@python3 ./scripts/audit_rule_provenance.py

skills-apply skills\:apply:
	@./manage.sh skills:apply

skills-bump skills\:bump:
	@./manage.sh skills:bump

skill-disable skill\:disable:
	@./manage.sh skill:disable $(SKILL)
	@./manage.sh skills:apply

skill-enable skill\:enable:
	@./manage.sh skill:enable $(SKILL)
	@./manage.sh skills:apply

skill-list skill\:list:
	@./manage.sh skill:list

mcp-apply mcp\:apply:
	@./manage.sh mcp:apply

tools-apply tools\:apply:
	@./manage.sh tools:apply

browser-apply browser\:apply:
	@./manage.sh browser:apply

claude-hooks-apply claude\:hooks\:apply:
	@./manage.sh claude:hooks:apply

claude-statusline-apply claude\:statusline\:apply:
	@./manage.sh claude:statusline:apply

claude-statusline-wire:
	@./manage.sh claude:statusline:wire

claude\:statusline\:wire:
	@./manage.sh claude:statusline:wire

claude-shell-aliases-apply claude\:shell-aliases\:apply:
	@./manage.sh claude:shell-aliases:apply

# 完整卸载 Claude Code 全局痕迹（默认预览；ARGS=--yes 才真删，可加 --backup-memory）
claude-uninstall claude\:uninstall:
	@./manage.sh claude:uninstall $(ARGS)

# 清理会话/历史/缓存等可删除数据，保留配置·记忆·技能（默认预览；ARGS=--yes 才删）
claude-clean claude\:clean:
	@./manage.sh claude:clean $(ARGS)

codex-hooks-apply codex\:hooks\:apply:
	@./manage.sh codex:hooks:apply

# 离线规则效果评测 (读取本地 fixtures 并输出稳定 JSON 摘要)
eval-rules:
	@python3 ./scripts/evals/run_rule_effectiveness_eval.py

# 指令质量回归门 (共享指令 / skills exposure / managed hooks)
instructions-check:
	@python3 -m unittest tests.test_project_contracts tests.test_rule_budgets tests.test_rule_layout tests.test_skill_exposure_contracts tests.test_sync_rules tests.test_sync_claude_hooks tests.test_sync_codex_hooks tests.test_rule_effectiveness_eval tests.test_skill_usage_metrics tests.test_skill_reference_audit -v
	@python3 ./scripts/evals/run_rule_effectiveness_eval.py --baseline ./scripts/evals/fixtures/rule_effectiveness_baseline.json

instructions\:check: instructions-check

help:
	@./manage.sh help

# 完整命令清单（日常 help 只显示日常入口 + 进阶原语）
help-all help\:all:
	@./manage.sh help:all
