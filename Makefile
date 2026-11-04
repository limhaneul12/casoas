.DEFAULT_GOAL := ci
.PHONY: ci check harness fmt lint test build install doctor tui status help

# Route Cargo through rustup's proxy; rust-toolchain.toml owns the version.
export PATH := $(HOME)/.cargo/bin:$(PATH)
CARGO ?= cargo

ci:
	$(CARGO) xtask ci

check:
	$(CARGO) xtask check

harness:
	$(CARGO) xtask rules

fmt:
	$(CARGO) xtask fmt

lint:
	$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

test:
	$(CARGO) xtask test

build:
	$(CARGO) build --release --package hcodex

install:
	$(CARGO) install --path crates/hcodex --locked

doctor:
	$(CARGO) xtask doctor


tui:
	$(CARGO) run --package hcodex -- tui

status:
	$(CARGO) run --package hcodex -- status

help:
	@printf '%s\n' 'make ci       Rust rules + fmt + check + strict Clippy + tests' 'make harness  Rust mechanical contracts' 'make build    release binary' 'make install  install hcodex' 'make tui      interactive workbench'
