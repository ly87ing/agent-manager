# CLAUDE.md

## Scope

- 本目录测试适用范围与迁移要求见 [AGENTS.md](AGENTS.md)：Python 专属测试随实现退役，仍适用的安全反例迁入 Rust。
- 原生功能与界面分别使用 `engine-rs/`、`app/src/test/` 的最小相关测试。旧文案断言不能覆盖用户已确定的新目标。

## Fixture discipline

- Keep fixtures under `tests/fixtures/`.
- Prefer reading repo files directly for file contracts; use mocks sparingly.
- keep each test focused on one contract and one expectation.

## Clarity

- Test names describe the protected behavior directly.
- Write assertions explicitly enough that a failure points straight at the missing rule or doc constraint.
