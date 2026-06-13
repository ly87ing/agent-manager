# CLAUDE.md

## Scope

- 维护 `agent-rule/rules/src/` 下的共享 core 和 agent adapters。
- 将 `agent-rule/rules/dist/` 视为生成产物，不直接手改。

## Workflow

- 修改 rules 后先运行 `make rules-build`。
- 需要分发时再运行 `make rules:sync`。
- 调整目标路径或文件名时，同时检查 `config/rules-targets.list`。

## Guardrails

- 共享规则保持 portable，把 agent-specific 细节留在各自 adapter。
- 如需仓库级命令或通用背景，回到根目录 `CLAUDE.md`。
