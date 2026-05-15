# CLAUDE.md

This file keeps repo-level memory for Claude Code in this repository.

## 常用命令

- `python3 -m unittest discover -s tests -p 'test*.py' -v`
- `make smoke`
- `make sync`
- `make doctor`

## 高频事实

- `manage.sh` 是唯一顶层编排入口，统一解释 `--dry-run`、`--offline`、锁和日志。
- `config/manager.json` 是仓库基线，`config/manager.local.json` 是本机覆盖。
- `agent-rule/rules/src/` 是可编辑规则源码，`agent-rule/rules/dist/` 是生成产物。
- `config/rules-targets.list` 定义 built rule artifact 到各 agent 目标路径的映射。
- `codex` 纳入 Agent lifecycle 管理，默认 provider 为 npm，升级使用 `codex update`；同时接收 MCP、Rules、Skills 分发。
- `tests/` 里的测试以 on-disk contract/integration 为主，迭代时优先跑最窄的 unittest 目标。
- 不再维护 terminal dotfiles 管理；不要让 agent-manager 改写 `~/.zshrc`、`~/.tmux.conf` 或 iTerm2 配置。

## Scoped Memory

- 编辑 `agent-rule/` 时，先读 `agent-rule/CLAUDE.md`。
- 编辑 `tests/` 时，先读 `tests/CLAUDE.md`。
