# Installing hcodex

`hcodex` is one Rust binary with an in-process Tokio backend. Python, `uv`, an
MCP client, and a separate hcodex server are not runtime dependencies.

## Requirements

- Rustup with the repository toolchain available
- Git
- OpenAI Codex CLI installed and authenticated for Codex routes
- Google Antigravity CLI authenticated for Argon and Claude routes

hcodex resolves Antigravity via `AGY_CLI_BIN`, then `agy` on `PATH`, then
`~/.local/libexec/hcodex/antigravity`. It does not need a running IDE or a
registered hcodex MCP server. Artifact execution uses the exact tested
macOS arm64 CLI 1.5.0 binary with the strict sandbox policy. An unverified
binary with a nonempty tool inventory is refused before the prompt. A CLI
version check or model catalog alone does not prove live inference.

## Install from a clone

```bash
git clone https://github.com/limhaneul12/casoas.git
cd casoas
make ci
cargo install --locked --path crates/hcodex
hcodex --workspace "$PWD" setup
hcodex status
```

The clone does not contain personal `.agents/` or `.cargo/` directories. `make ci`
uses the published `xtask/HARNESS.toml` mechanical policy and project research
asset manifest, and invokes `cargo run --package xtask -- ...` directly. A local
Cargo alias or materialized private instruction bundle is not a build input.

`cargo install` writes the binary to Cargo's configured binary directory. Put
that directory on `PATH` before running `hcodex setup`.

The binary embeds a hash-verified skill bundle. `setup` and `sync` therefore work
from another project without the source checkout. When matching checkout assets
are present, their inventory and bytes must match the embedded bundle; divergent
assets fail validation.

`hcodex setup` and `hcodex sync` perform these bounded user-scoped operations:

- install the `codex-heterarchy` skill in the managed Gemini and Codex skill
  directories;
- migrate only the recognized historical hcodex launcher to the installed Rust
  binary;
- remove only a legacy hcodex MCP registration or generated `mcp_config.json`
  whose managed identity is proven.

They preserve custom launchers, custom same-name commands, unrelated MCP entries,
comments, permissions, credentials, existing job state, and unrecognized plugin
directories. A same-name entry with different ownership is reported as a conflict
instead of being overwritten or deleted.

The historical state root remains `~/.cache/codex-collab-mcp/` for SQLite
compatibility. The directory name does not mean the retired MCP server is active.

## Install without setup

The repository helper installs only the binary by default:

```bash
./scripts/install.sh
```

Run the explicit setup step after installation with:

```bash
./scripts/install.sh --activate
```

## Update

```bash
git pull --ff-only
make ci
cargo install --locked --path crates/hcodex --force
hcodex --workspace "$PWD" sync
```

The retired `mcp`, `on`, `off`, and `restart` commands are unavailable. CLI and
TUI processes invoke the common backend directly and own their child tasks and
processes for the lifetime of that invocation.

## Verification

```bash
hcodex --version
hcodex --help
hcodex status
hcodex tui -c "/status"
```

Verify that `hcodex --help` does not advertise `mcp`, `on`, `off`, or `restart`.
The source repository's canonical development gate is `make ci`. All production
entry points and active tests are Rust-native; historical compatibility uses
frozen SQLite fixtures.


## Local development instructions

Agent work follows `AGENTS.md` and the locally materialized Rust/shared rules.
Keep that materialization in the ignored `.agents/` directory and retain any
personal Cargo aliases in ignored `.cargo/`. Do not force-add either directory.
To verify the complete local Rust/shared file inventory and recorded hashes:

```bash
make local-harness
```

This explicit check fails when the local materialization is missing or has
integrity drift. It is separate from the portable canonical CI and is not
reported as executed for a clean clone that has no private bundle.
