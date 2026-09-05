# CLAUDE.md

This file keeps repo-level memory for Claude Code in this repository.

## Common commands

- `python3 -m unittest discover -s tests -p 'test*.py' -v`
- `make smoke`
- `make update`
- `make doctor`

## Key facts

- `manage.sh` is the single top-level orchestration entry point; it centrally interprets `--dry-run`, `--offline`, the lock, and logging.
- `manage.sh` 只有 5 个命令(命令→管线映射见 `manage.sh` 的 `run_update_pipeline`/`run_agents_pipeline`/`run_init_pipeline`):`update`=日常主入口,把所有受管组件**分发/收敛到本机**(secrets 拉取 + shell alias + tools + browser + rules 构建分发 + skills 链接 + mcp 注入 + Claude/Codex hooks 同步),不装不升 Agent 软件;`init`=`update` + Agent 生命周期(装缺失 + 升级已装,默认 `--wait-async-actions`);`agents:update` 只跑 Agent 生命周期(`--no-install`),把已装 Agent 升级到上游最新;`doctor` 深度体检(`--json` 可机读);`help` 显示帮助。**全仓库不锁版本**:npm 命令不带版本号、git 依赖跟踪分支头、skill 仓库跟随默认分支,`update` 即把本机收敛到上游当前状态。预演用 `./manage.sh init --dry-run`。以上刻意**非交互**,让 CI/bootstrap 不卡 stdin。
- `config/manager.json` is the repo baseline; `config/manager.local.json` is the per-machine override.
- `agent-rule/rules/src/` holds the editable rule sources; `agent-rule/rules/dist/` holds the generated artifacts.
- `config/rules-targets.list` maps each built rule artifact to its per-agent target path.
- `codex` is under Agent lifecycle management, defaults to the npm provider, and is upgraded with `codex update`; it also receives MCP, Rules, and Skills distribution.
- Tests under `tests/` are mainly on-disk contract/integration tests; when iterating, run the narrowest unittest target first.
- Terminal dotfiles management is no longer maintained; do not let agent-manager rewrite `~/.tmux.conf` or iTerm2 config. **例外**:shell alias 的受管区块(marker 圈定、幂等、只动区块内)由 `sync_shell_aliases.sh` 同步到 `~/.zshrc`(interactive shell 读)与 `~/.zprofile`(login shell 读);写 `.zprofile` 是刻意为之——让非交互 login shell(如 UU 远程 term)也能加载别名。除这个 alias 区块外,不改任何终端 dotfile。

## Scoped Memory

- When editing `agent-rule/`, read `agent-rule/CLAUDE.md` first.
- When editing `tests/`, read `tests/CLAUDE.md` first.
