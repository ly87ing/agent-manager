# CLAUDE.md

This file keeps repo-level memory for Claude Code in this repository.

## Common commands

- `python3 -m unittest discover -s tests -p 'test*.py' -v`
- `make smoke`
- `make sync`
- `make doctor`

## Key facts

- `manage.sh` is the single top-level orchestration entry point; it centrally interprets `--dry-run`, `--offline`, the lock, and logging.
- `config/manager.json` is the repo baseline; `config/manager.local.json` is the per-machine override.
- `agent-rule/rules/src/` holds the editable rule sources; `agent-rule/rules/dist/` holds the generated artifacts.
- `config/rules-targets.list` maps each built rule artifact to its per-agent target path.
- `codex` is under Agent lifecycle management, defaults to the npm provider, and is upgraded with `codex update`; it also receives MCP, Rules, and Skills distribution.
- The Claude Code statusline is maintained by `scripts/claude/statusline.sh` (pure bash, no jq/git/network/transcript IO; the 5h/7d usage is followed by a "reset countdown" that forks `date +%s` once for `now` only when `resets_at` is present, shared by both windows, with the duration computed in pure bash and zero forks otherwise). `make claude:statusline:install` installs it to `~/.claude/statusline.sh`; `make claude:statusline:wire` writes it into settings.json (preserving any existing theme/effortLevel, without forcing a theme).
- Tests under `tests/` are mainly on-disk contract/integration tests; when iterating, run the narrowest unittest target first.
- Terminal dotfiles management is no longer maintained; do not let agent-manager rewrite `~/.zshrc`, `~/.tmux.conf`, or iTerm2 config.

## Scoped Memory

- When editing `agent-rule/`, read `agent-rule/CLAUDE.md` first.
- When editing `tests/`, read `tests/CLAUDE.md` first.
