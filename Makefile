.PHONY: all bootstrap sync update plan self-update smoke ui help doctor detect fetch-models status cleanup unlock backups-list restore logs rules-sync skills-link mcp-sync mcp-prepare tools-sync terminal-sync dry-run clean-logs offline

# 默认目标：显示帮助，不执行任何同步操作
.DEFAULT_GOAL := help

# 全量执行（安装 + 更新 + 同步）
all:
	@./manage.sh all

# 新机器初始化或全量收敛
bootstrap:
	@./manage.sh bootstrap

# 仅同步当前配置，不联网、不安装
sync:
	@./manage.sh sync

# 更新远端资源并收敛本地环境，但不安装缺失 Agent
update:
	@./manage.sh update

# 预演一次完整 bootstrap
plan:
	@./manage.sh plan

# 仅更新 agent-manager 仓库自身逻辑
self-update:
	@./manage.sh self-update

# 跨平台 smoke 验证（安全预演路径）
smoke:
	@./scripts/ci-smoke.sh

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
cleanup:
	@./manage.sh cleanup

# 强制解除资源锁
unlock:
	@./manage.sh unlock

# 历史配置备份清单

backups-list:
	@./manage.sh backups:list

# 交互式备份恢复
restore:
	@./manage.sh backups:restore

# 环境探测
detect:
	@./manage.sh detect

# 探测并获取支持的模型列表 (用法: make fetch-models url=<API地址> key=<API_KEY>)
fetch-models:
	@./manage.sh fetch-models $(url) $(key)

# 查看最近同步日志
logs:
	@./manage.sh logs

# 清理日志
clean-logs:
	@./manage.sh clean-logs

# 独立同步任务
rules-sync:
	@./manage.sh rules:sync

skills-link:
	@./manage.sh skills:link

mcp-sync:
	@./manage.sh mcp:sync

mcp-prepare:
	@./manage.sh mcp:prepare

tools-sync:
	@./manage.sh tools:sync

terminal-sync:
	@./manage.sh terminal:sync

# 预览模式 (仅输出计划动作，跳过远端副作用)
dry-run:
	@./manage.sh plan

help:
	@./manage.sh help
