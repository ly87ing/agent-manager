# CLAUDE.md

本仓库协作要求以根目录 [AGENTS.md](AGENTS.md) 为准。当前目标是 Rust 原生后端与 Tauri UI，Python 实现已停止维护并获准清理。

- 产品要求：[GUI 重设计](docs/GUI-REDESIGN.md)。
- 当前实施、执行者切换及验收：[任务书](docs/goal-rust-gui.md)。
- 修改 `agent-rule/` 时同时读取该目录协作说明；修改退役 `tests/` 时读取该目录说明。
- 旧 `manage.sh` 管线、三层 JSON、自动收敛和 Python 打包记录不是新版要求；不得据此恢复已废弃的行为。
- 不凭旧版包验收或工作区候选声明原生版本完成。用户配置、会话及凭据不在代码清理范围。
