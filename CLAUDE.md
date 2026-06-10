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
- Claude Code statusline 由 `scripts/claude/statusline.sh`（纯 bash、无 jq/git/网络/transcript IO；5h/7d 用量后跟「重置倒计时」，仅当 `resets_at` 存在时 fork 一次 `date +%s` 取 now，两窗口共用、纯 bash 算时长，其余零 fork）维护，`make claude:statusline:install` 装到 `~/.claude/statusline.sh`、`make claude:statusline:wire` 写入 settings.json（保留既有 theme/effortLevel，不强设 theme）。
- `tests/` 里的测试以 on-disk contract/integration 为主，迭代时优先跑最窄的 unittest 目标。
- 不再维护 terminal dotfiles 管理；不要让 agent-manager 改写 `~/.zshrc`、`~/.tmux.conf` 或 iTerm2 配置。

## Scoped Memory

- 编辑 `agent-rule/` 时，先读 `agent-rule/CLAUDE.md`。
- 编辑 `tests/` 时，先读 `tests/CLAUDE.md`。
