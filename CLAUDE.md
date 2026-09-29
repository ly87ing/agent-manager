# CLAUDE.md

本仓库协作要求以根目录 @AGENTS.md 为准——这一行是 import，它的正文随本文件一并加载，不用再单独打开。当前目标是 Rust 原生后端与 Tauri UI，Python 实现已停止维护并获准清理。

- 产品定位与分期：[多账号并行设计](docs/MULTI-ACCOUNT-DESIGN.md)；数据语义与写入保护：[GUI 重设计](docs/GUI-REDESIGN.md)。
- 当前实施、执行者切换及验收：[任务书](docs/goal-rust-gui.md)。
- 规则正文在 agent-skills 仓库 `rules/` 维护（见其 `rules/README.md`）；修改退役 `tests/` 时读取该目录说明。
- 旧 `manage.sh` 管线、三层 JSON、自动收敛和 Python 打包记录不是新版要求；不得据此恢复已废弃的行为。
- 不凭旧版包验收或工作区候选声明原生版本完成。用户配置、会话及凭据不在代码清理范围。
