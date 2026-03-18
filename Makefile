.PHONY: all ui help doctor detect status cleanup unlock backups-list restore logs rules-sync skills-link mcp-sync mcp-prepare tools-sync dry-run clean-logs offline

# 默认目标：显示帮助，不执行任何同步操作
.DEFAULT_GOAL := help

# 显式执行全量同步（包括安装/更新）
all:
	@./manage.sh all

# 离线模式 (禁止远程拉取/安装，仅使用本地已存在资源)
offline:
	@./manage.sh all --offline

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

# 预览模式 (仅输出计划动作，跳过远端副作用)
dry-run:
	@./manage.sh all --dry-run

help:
	@./manage.sh help
