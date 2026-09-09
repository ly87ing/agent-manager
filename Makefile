.PHONY: help engine-test app-test test check build smoke

# 默认目标：显示帮助，不执行任何同步操作
.DEFAULT_GOAL := help

help:
	@printf '%s\n' '开发入口:' '  make test       engine-rs 与 Tauri 单测' '  make check      Rust all-targets check 与前端 typecheck' '  make build      前端生产构建'

engine-test:
	cargo test --manifest-path engine-rs/Cargo.toml

app-test:
	cargo test --manifest-path app/src-tauri/Cargo.toml
	cd app && pnpm test:unit run

test: engine-test app-test

check:
	cargo fmt --manifest-path engine-rs/Cargo.toml -- --check
	cargo check --manifest-path engine-rs/Cargo.toml --all-targets
	cargo check --manifest-path app/src-tauri/Cargo.toml --all-targets
	cd app && pnpm typecheck

build:
	cd app && pnpm build

# 跨平台 smoke 验证（安全预演路径）
smoke:
	@./scripts/ci-smoke.sh
