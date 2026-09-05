.PHONY: help init update agents-update agents\:update doctor smoke

# 默认目标：显示帮助，不执行任何同步操作
.DEFAULT_GOAL := help

help:
	@./manage.sh help

# 新机器初始化或全量收敛（安装缺失 Agent + 升级已装 Agent + 全量分发）
init:
	@./manage.sh init

# 日常最常用：把所有受管组件收敛到本机（不安装、也不升级 Agent 软件）
update:
	@./manage.sh update

# 升级/修复已装 Agent 到上游最新（agents.json 不锁版本）
agents-update agents\:update:
	@./manage.sh agents:update

# 系统健康深度诊断
doctor:
	@./manage.sh doctor

# 跨平台 smoke 验证（安全预演路径）
smoke:
	@./scripts/ci-smoke.sh
