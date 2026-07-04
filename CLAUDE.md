# CLAUDE.md

This file keeps repo-level memory for Claude Code in this repository.

## Common commands

- `python3 -m unittest discover -s tests -p 'test*.py' -v`
- `make smoke`
- `make sync`
- `make doctor`

## Key facts

- `manage.sh` is the single top-level orchestration entry point; it centrally interprets `--dry-run`, `--offline`, the lock, and logging.
- `manage.sh` 命令分三层 `sync ⊂ pull ⊂ update`:`sync`=纯离线本地分发(rules 构建+分发、skills 链接、mcp 注入),**永远刷不了 skill 内容**(只有 update/pull 里的 `skills_upgrade` 联网能);`pull`=联网刷新 rules/skills/mcp **和 secrets** 再分发,不动 agent/tools/browser;`update`=`pull` + agent/tools/browser 生命周期。三者刻意**非交互**(`--yes` 自动确认、`make plan` 给 per-agent 升级预览),让 CI/bootstrap 不卡 stdin;`bootstrap`/`all` 走 `run_update_pipeline` 但不带 `--no-install`,会装缺失 agent。改 grok/MCP 凭证跑 `make sync` 就够(顺带刷 rules+skills)。
- `config/manager.json` is the repo baseline; `config/manager.local.json` is the per-machine override.
- `agent-rule/rules/src/` holds the editable rule sources; `agent-rule/rules/dist/` holds the generated artifacts.
- `config/rules-targets.list` maps each built rule artifact to its per-agent target path.
- `codex` is under Agent lifecycle management, defaults to the npm provider, and is upgraded with `codex update`; it also receives MCP, Rules, and Skills distribution.
- The Claude Code statusline is maintained by `scripts/claude/statusline.sh` (pure bash, no jq/git/network/transcript IO; the 5h/7d usage is followed by a "reset countdown" that forks `date +%s` once for `now` only when `resets_at` is present, shared by both windows, with the duration computed in pure bash and zero forks otherwise). `make claude:statusline:install` installs it to `~/.claude/statusline.sh`; `make claude:statusline:wire` writes it into settings.json (preserving any existing theme/effortLevel, without forcing a theme). 配色要在深/浅背景都可读:非语义段(分隔符、成本、倒计时、输出风格)用终端默认前景色或加粗,**不用** `90m`(亮黑/暗灰,深背景近隐形)或 `34m`(深蓝,深背景偏暗);仅语义处用标准 16 色阈值色(红/黄/绿),终端会按主题重映射成可读值——"看不清 statusline"的真实根因常是低对比度专用色而非渲染故障。
- Tests under `tests/` are mainly on-disk contract/integration tests; when iterating, run the narrowest unittest target first.
- Terminal dotfiles management is no longer maintained; do not let agent-manager rewrite `~/.zshrc`, `~/.tmux.conf`, or iTerm2 config.

## Scoped Memory

- When editing `agent-rule/`, read `agent-rule/CLAUDE.md` first.
- When editing `tests/`, read `tests/CLAUDE.md` first.
