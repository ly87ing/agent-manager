# CLAUDE.md

This file keeps repo-level memory for Claude Code in this repository.

## Common commands

- `python3 -m unittest discover -s tests -p 'test*.py' -v`
- `make smoke`
- `make update`
- `make apply`
- `make doctor`

## Key facts

- `manage.sh` is the single top-level orchestration entry point; it centrally interprets `--dry-run`, `--offline`, the lock, and logging.
- `manage.sh` 命令按 GitOps 语义拆成两个正交原语 `apply` 与 `bump`,`update` 叠加二者、`init` 再叠加装 agent(命令→管线映射见 `manage.sh` 的 `run_apply_pipeline`/`run_bump_pipeline`/`run_update_pipeline`/`run_agents_pipeline`/`run_init_pipeline`):`apply`=把当前锁定态**分发/收敛到本机**(secrets 拉取 + shell alias + tools + browser + rules 构建分发 + skills 链接 + mcp 注入),不升版本、不装 agent,**永远刷不了 skill 内容**(只有 `bump` 里的 `skills:bump` 联网能);`bump`=只刷**元数据**(secrets 拉取 + version pins + `skills:bump` 刷 skill commit),不分发、不装 agent、不改本机环境;`update`=`bump`+`apply`(secrets 只拉一次),日常「把本机弄成最新」的主入口,不装 agent;`init`=`bump` + agent 生命周期(装缺失 agent)+ `apply` + 失效引用审计;`agents:apply` 只跑 agent 生命周期(`--no-install`、默认 `--wait-async-actions`),按**当前版本锁**升级/修复已装 agent、自身不刷锁,所以「只想把 agent 升到上游最新」是 `bump` + `agents:apply` 两步(或一步 `init`),`update` 只刷锁不落地到 agent 软件。`plan` 是 `init` 的 `--dry-run` 预演。以上刻意**非交互**(`--yes` 自动确认、`make plan` 给 per-agent 升级预览),让 CI/bootstrap 不卡 stdin。日常跑 `make update`;只改 grok/MCP 凭证跑 `make apply` 就够(顺带刷 rules + skills 链接 + mcp);只想联网刷元数据不分发才用 `make bump`。默认 `./manage.sh help` 只列日常入口+进阶原语,完整命令(运维/分步原子)见 `./manage.sh help:all`。
- `config/manager.json` is the repo baseline; `config/manager.local.json` is the per-machine override.
- `agent-rule/rules/src/` holds the editable rule sources; `agent-rule/rules/dist/` holds the generated artifacts.
- `config/rules-targets.list` maps each built rule artifact to its per-agent target path.
- `codex` is under Agent lifecycle management, defaults to the npm provider, and is upgraded with `codex update`; it also receives MCP, Rules, and Skills distribution.
- The Claude Code statusline is maintained by `scripts/claude/statusline.sh` (pure bash, no jq/git/network/transcript IO; the 5h/7d usage is followed by a "reset countdown" that forks `date +%s` once for `now` only when `resets_at` is present, shared by both windows, with the duration computed in pure bash and zero forks otherwise). `make claude:statusline:apply` installs it to `~/.claude/statusline.sh`; `make claude:statusline:wire` writes it into settings.json (preserving any existing theme/effortLevel, without forcing a theme). 配色要在深/浅背景都可读:非语义段(分隔符、成本、倒计时、输出风格)用终端默认前景色或加粗,**不用** `90m`(亮黑/暗灰,深背景近隐形)或 `34m`(深蓝,深背景偏暗);仅语义处用标准 16 色阈值色(红/黄/绿),终端会按主题重映射成可读值——"看不清 statusline"的真实根因常是低对比度专用色而非渲染故障。
- Tests under `tests/` are mainly on-disk contract/integration tests; when iterating, run the narrowest unittest target first.
- Terminal dotfiles management is no longer maintained; do not let agent-manager rewrite `~/.tmux.conf` or iTerm2 config. **例外**:shell alias 的受管区块(marker 圈定、幂等、只动区块内)由 `sync_shell_aliases.sh` 同步到 `~/.zshrc`(interactive shell 读)与 `~/.zprofile`(login shell 读);写 `.zprofile` 是刻意为之——让非交互 login shell(如 UU 远程 term)也能加载别名。除这个 alias 区块外,不改任何终端 dotfile。

## Scoped Memory

- When editing `agent-rule/`, read `agent-rule/CLAUDE.md` first.
- When editing `tests/`, read `tests/CLAUDE.md` first.
