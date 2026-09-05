# AGENTS.md

## Setup
- Python 3 and make are enough for the repo checks; no extra bootstrap is needed for doc or contract edits.
- If you touch rules, edit `agent-rule/rules/src/` and rebuild + distribute with `make update`（构建脚本为 `agent-rule/scripts/build-agent-rules.py`）。

## Test commands
- `python3 -m unittest discover -s tests -p 'test*.py' -v`
- `python3 -m unittest tests.test_project_contracts -v`
- `make smoke`

## done-when
- The targeted test passes first, then the relevant broader test command passes.
- README and docs match the implemented behavior.
- No generated file was hand-edited.

## generated-file boundaries
- Treat `agent-rule/rules/dist/` as generated output only.
- Rebuild from `agent-rule/rules/src/` and sync from the build output instead of editing dist files directly.

## secrets boundaries
- 真实敏感信息只允许存放在外部私有 `secrets/` 仓库，不允许写入当前仓库的代码、文档、示例、计划、HTML、README 或生成产物。
- `Tailscale` / `SSH` / 远程访问场景中的真实用户名、设备名、`.ts.net` 域名、Tailscale IP、SSH Host 配置、公钥内容和可推导内网拓扑的组合信息都按敏感信息处理。
- 当前仓库只能保留占位符、模板、注入逻辑和通用说明；需要举例时统一使用 `YOUR_*` 占位符。
