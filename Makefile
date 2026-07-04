.PHONY: all init bootstrap install apply sync bump update plan agents-apply agents\:apply agents-sync agents\:sync sys-upgrade sys\:upgrade self-update smoke ui help doctor status detect fetch-models offline dry-run eval-rules cleanup sys-clean sys\:clean state-clean state\:clean backups-gc backups\:gc unlock sys-unlock sys\:unlock logs sys-logs sys\:logs clean-logs sys-clean-logs sys\:clean-logs backups-list backups\:list state-list state\:list restore backups\:restore state-restore state\:restore rules-build rules\:build rules-apply rules\:apply rules-sync rules\:sync skills-apply skills\:apply skills-link skills\:link skills-bump skills\:bump skills\:upgrade mcp-apply mcp\:apply mcp-sync mcp\:sync mcp-build mcp\:build mcp-prepare mcp\:prepare tools-apply tools\:apply tools-sync tools\:sync browser-apply browser\:apply browser-us-sync browser\:sync claude-hooks-apply claude\:hooks\:apply claude-hooks-sync claude\:hooks\:sync claude-statusline-apply claude\:statusline\:apply claude-statusline-install claude\:statusline\:install claude-statusline-wire claude\:statusline\:wire claude-shell-aliases-apply claude\:shell-aliases\:apply claude-shell-aliases-sync claude\:shell-aliases\:sync codex-hooks-apply codex\:hooks\:apply codex-hooks-sync codex\:hooks\:sync secrets-check secrets\:check instructions-check instructions\:check

# 默认目标：显示帮助，不执行任何同步操作
.DEFAULT_GOAL := help

# 全量执行（安装 + 更新 + 同步）
all:
	@./manage.sh all

# 新机器初始化或全量收敛
init bootstrap:
	@./manage.sh init

# 兼容安装入口：等价于 bootstrap
install: init

# 同步当前配置并收敛 skills / MCP 资源；不处理 Agent 安装
apply sync:
	@./manage.sh apply

# 纯粹刷新版本锁与 skill commit（更新元数据到锁定文件），不执行本地环境应用
bump update:
	@./manage.sh bump

# 预演一次完整 bootstrap
plan:
	@./manage.sh plan

# 仅更新 agent-manager 仓库自身逻辑
agents-apply agents\:apply agents-sync agents\:sync:
	@./manage.sh agents:apply

sys-upgrade sys\:upgrade self-update:
	@./manage.sh sys:upgrade

# 跨平台 smoke 验证（安全预演路径）
smoke:
	@./scripts/ci-smoke.sh

# 主仓库明文敏感信息扫描
secrets-check:
	@python3 ./scripts/check_no_secrets.py

secrets\:check:
	@python3 ./scripts/check_no_secrets.py

# 兼容旧入口：离线本地同步
offline:
	@./manage.sh offline

# 启动交互式管理界面
ui:
	@./manage.sh ui

# 系统健康深度诊断
doctor:
	@./manage.sh doctor

# 快速查看状态 (同 doctor)
status: doctor

# 全量大扫除 (清理备份、日志、残留)
sys-clean sys\:clean cleanup:
	@./manage.sh sys:clean

# 删除受管 Agent home 下超过 30 天的 .bak.* 文件
state-clean state\:clean backups-gc:
	@find ~/.claude ~/.codex ~/.gemini -maxdepth 2 -type f -name '*.bak.*' -mtime +30 -print -delete 2>/dev/null || true

backups\:gc:
	@$(MAKE) backups-gc

# 强制解除资源锁
sys-unlock sys\:unlock unlock:
	@./manage.sh sys:unlock

# 历史配置备份清单

state-list state\:list backups-list backups\:list:
	@./manage.sh state:list

# 交互式备份恢复
state-restore state\:restore restore backups\:restore:
	@./manage.sh state:restore

# 环境探测
detect:
	@./manage.sh detect

# 探测并获取支持的模型列表 (用法: make fetch-models url=<API地址> key=<API_KEY>)
fetch-models:
	@./manage.sh fetch-models $(url) $(key)

# 查看最近同步日志
sys-logs sys\:logs logs:
	@./manage.sh sys:logs

# 清理日志
sys-clean-logs sys\:clean-logs clean-logs:
	@./manage.sh sys:clean-logs

# 独立同步任务
rules-build:
	@./manage.sh rules:build

rules\:build:
	@./manage.sh rules:build

rules-apply rules\:apply rules-sync rules\:sync:
	@./manage.sh rules:apply

skills-apply skills\:apply skills-link skills\:link:
	@./manage.sh skills:apply

skills-bump skills\:bump skills\:upgrade:
	@./manage.sh skills:bump

mcp-apply mcp\:apply mcp-sync mcp\:sync:
	@./manage.sh mcp:apply

mcp-build mcp\:build mcp-prepare mcp\:prepare:
	@./manage.sh mcp:build

tools-apply tools\:apply tools-sync tools\:sync:
	@./manage.sh tools:apply

browser-apply browser\:apply browser-us-sync browser\:sync:
	@./manage.sh browser:apply

claude-hooks-apply claude\:hooks\:apply claude-hooks-sync claude\:hooks\:sync:
	@./manage.sh claude:hooks:apply

claude-statusline-apply claude\:statusline\:apply claude-statusline-install claude\:statusline\:install:
	@./manage.sh claude:statusline:apply

claude-statusline-wire:
	@./manage.sh claude:statusline:wire

claude\:statusline\:wire:
	@./manage.sh claude:statusline:wire

claude-shell-aliases-apply claude\:shell-aliases\:apply claude-shell-aliases-sync claude\:shell-aliases\:sync:
	@./manage.sh claude:shell-aliases:apply

codex-hooks-apply codex\:hooks\:apply codex-hooks-sync codex\:hooks\:sync:
	@./manage.sh codex:hooks:apply

# 预览模式 (仅输出计划动作，跳过远端副作用)
dry-run:
	@./manage.sh plan

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
