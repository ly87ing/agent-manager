# OpenCode 默认安装策略调整设计

## 目标

让仓库默认策略不再把 `OpenCode` 作为自动安装对象处理，同时保留：

- `opencode` 这个受管 Agent 定义
- 显式开启 `auto_install` 后的安装能力
- 已安装场景下的受管更新能力

## 设计结论

继续采用上一轮同型的最小改动方案：

1. 将 [`config/manager.json`](/Users/ly/agent-manager/config/manager.json) 中 `agents.opencode.auto_install` 从 `true` 调整为 `false`
2. 在 [`README.md`](/Users/ly/agent-manager/README.md) 中同步更新默认自动安装集合，避免文档和默认策略脱节
3. 增加一条 contract test，固定 `OpenCode` 默认不自动安装的语义

## 非目标

- 不移除 `opencode` 的受管定义
- 不修改 [`config/agents.json`](/Users/ly/agent-manager/config/agents.json) 中的安装源定义
- 不更改 `doctor` / `detect` / `sync` 对已安装 OpenCode 的审计与更新逻辑

## 验证方式

1. 先添加测试，确认当前基线下因 `opencode.auto_install == true` 而失败
2. 修改策略与 README
3. 运行目标测试与相关模块测试，确认默认策略和文档契约一致
