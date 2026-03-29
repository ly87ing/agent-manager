# CLAUDE.md

## Scope

- 优先使用最窄的 unittest 目标，例如 `python3 -m unittest tests.test_project_contracts -v`。
- 修改单个契约时，进一步缩到具体测试方法。

## Fixture discipline

- fixture 保持在 `tests/fixtures/` 下。
- 文件契约优先直接读取仓库文件，少用 mock。
- keep each test focused on one contract and one expectation.

## Clarity

- 测试名直接描述被保护的行为。
- 断言写得足够明确，让失败能直接指出缺少哪条规则或文档约束。
