# Architecture Guidelines

Read this rule before introducing new modules, changing shared interfaces, or altering dependency boundaries.

## Required Checks

- Prefer the smallest local change that satisfies the task. Do not introduce a new abstraction unless there are at least two real consumers or a clear boundary problem.
- Keep dependency direction explicit. UI or CLI may depend on application or service code; application code may depend on domain code; infrastructure must stay behind adapters instead of leaking inward.
- Shared modules must expose a narrow interface, have a clear owner, and justify their existence with a concrete caller or test.
- Config, schema, and workflow changes must define defaults, failure mode, rollback path, and whether migration is required.

## Before Completion

- Verify every changed entry point still has an obvious dependency path and no new circular dependency.
