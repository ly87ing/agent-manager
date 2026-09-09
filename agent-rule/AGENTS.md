# AGENTS.md

## Scope
- Edit shared rules in `agent-rule/rules/src/`.
- Treat `agent-rule/rules/dist/` as generated output only.

## Workflow
- 规则构建与分发需接入 Rust 原生路径；旧 `make update` / Python 构建器已退出维护目标。替代入口未完成时，明确记录生成与分发尚未验证，不手改产物，也不增加旧构建器功能。

## Boundaries
- Never hand-edit files in `agent-rule/rules/dist/`.
- Keep `rules/src/shared/core.md` limited to always-on cross-agent rules.
- Put low-frequency workflow detail in skills maintained by `agent-skills`; keep agent-specific differences in `rules/src/agents/*/adapter.md`.
- Preserve the shared core bias toward explicit assumptions, simplest-change, surgical-edit, and verification-driven execution.
- Keep this file narrow and do not repeat the repo-wide setup or test guidance.
