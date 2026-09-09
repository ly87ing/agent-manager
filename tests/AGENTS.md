# AGENTS.md

## Scope
- 本目录主要为待退役 Python 实现的历史测试，不再扩展旧实现功能或双实现对照要求。
- 原生功能测试放在 `engine-rs/`，界面测试放在 `app/src/test/`。仍适用的安全、文件与数据反例必须迁入对应测试，不能借清理丢失保护。
- 文档目标已变化时记录旧文案断言不再适用，不为使旧测试通过而恢复废弃要求。核对历史行为时可运行最具体的既有 unittest，并据实报告范围。

## Fixture discipline
- Keep fixtures under `tests/fixtures/`.
- Prefer direct reads of repo files over mocks when a contract is about on-disk behavior.
- keep each test focused on one contract and one expectation.

## Clarity
- Name tests after the behavior they enforce.
- Make contract assertions explicit so failures point at the missing rule or file.
