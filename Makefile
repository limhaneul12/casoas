.DEFAULT_GOAL := ci
.PHONY: ci check harness local-harness fmt lint test build install doctor tui status help

# Route Cargo through rustup's proxy; rust-toolchain.toml owns the version.
export PATH := $(HOME)/.cargo/bin:$(PATH)
CARGO ?= cargo
XTASK = $(CARGO) run --locked --package xtask --

ci:
	$(XTASK) ci

check:
	$(XTASK) check

harness:
	$(XTASK) rules

local-harness:
	$(XTASK) local-harness

fmt:
	$(XTASK) fmt

lint:
	$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

test:
	$(XTASK) test

build:
	$(CARGO) build --release --package hcodex

install:
	$(CARGO) install --path crates/hcodex --locked

doctor:
	$(XTASK) doctor


tui:
	$(CARGO) run --package hcodex -- tui

status:
	$(CARGO) run --package hcodex -- status

help:
	@printf '%s\n' 'make ci       Rust rules + fmt + check + strict Clippy + tests' 'make harness  Portable Rust mechanical contracts' 'make local-harness  Check ignored local Rust/shared materialization' 'make build    release binary' 'make install  install hcodex' 'make tui      interactive workbench'
