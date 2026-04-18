# 探测失败时保守跳过远端刷新设计

## 目标

让受管资源在“远端更新探测失败”时默认更省流量：

- install-based tool 的版本探测返回 `unknown` 时，默认跳过升级
- repo-backed tool / skill / MCP 仓库的远端探测返回 `unknown` 时，默认跳过 `git pull` / `git fetch`
- 保留一个显式 `--force-refresh` 开关，在需要时继续执行原先的强刷路径

## 设计结论

采用最小本地改动：

1. 在 [`agent-tool/sync_tools.py`](/Users/ly/agent-manager/agent-tool/sync_tools.py) 中增加 `force_refresh` 开关，并将 `unknown` 的默认行为从“继续升级/拉取”改为“warning 后跳过”
2. 在 [`agent-skill/link_skills.py`](/Users/ly/agent-manager/agent-skill/link_skills.py) 与 [`agent-mcp/sync_mcp.py`](/Users/ly/agent-manager/agent-mcp/sync_mcp.py) 中对 `clone_or_update_repo` 增加同名开关，默认跳过 `unknown` 探测回退刷新的网络动作
3. 在 [`manage.sh`](/Users/ly/agent-manager/manage.sh) 中增加全局 `--force-refresh`，并透传到 tools / skills / mcp 同步入口
4. 更新帮助文案与架构说明，明确默认行为、显式强刷入口和失败模式

## 默认行为

- 默认：探测失败时记录 warning，并保留本地已就绪状态，不发起额外网络刷新
- 显式 `--force-refresh`：恢复旧的保守防漏更路径，即探测失败后继续升级或拉取

## 非目标

- 不修改 agent lifecycle 的 `brew outdated` 探测策略
- 不改变 `chrome-devtools` 依赖 `npx @latest` 的运行模式
- 不引入新的配置项或本地持久化策略

## 回滚路径

如果新策略导致用户感觉“漏更”风险过高，可以：

1. 临时在命令上增加 `--force-refresh`
2. 直接回退本次代码改动，恢复探测失败后继续刷新的默认行为
