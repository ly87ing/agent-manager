# Cursor 默认安装策略调整设计

## 目标

让仓库默认策略不再把 `Cursor IDE` 作为自动安装对象处理，同时保留：

- `cursor` 这个受管 Agent 定义
- 显式开启 `auto_install` 后的安装能力
- 已安装场景下的受管更新能力

## 当前判断

仓库内没有单独受管的 `cursor-cli` 条目。当前策略中的 `cursor` 代表 `Cursor IDE`，其命令行入口同样使用 `cursor` 二进制。因此本次调整按“默认不自动安装 Cursor IDE，以及随安装带出的 CLI 入口”处理。

## 设计结论

采用最小改动方案：

1. 将 [`config/manager.json`](/Users/ly/agent-manager/config/manager.json) 中 `agents.cursor.auto_install` 从 `true` 调整为 `false`
2. 在 [`README.md`](/Users/ly/agent-manager/README.md) 中补充默认自动安装策略说明，避免文档把“受管支持”误读为“默认自动安装”
3. 增加一条 contract test，固定默认策略语义

## 非目标

- 不移除 `cursor` 的受管定义
- 不修改 [`config/agents.json`](/Users/ly/agent-manager/config/agents.json) 中的安装源定义
- 不更改 `doctor` / `detect` / `sync` 对已安装 Cursor 的审计与更新逻辑
- 不新增单独的 `cursor-cli` 配置对象

## 验证方式

1. 先添加测试，确认当前基线下因 `cursor.auto_install == true` 而失败
2. 修改策略与 README
3. 运行目标测试与相关模块测试，确认默认策略和文档契约一致
