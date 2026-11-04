# Installing hcodex

`hcodex` is installed as one Rust binary. Python and `uv` are not production runtime dependencies.

## Requirements

- Rustup with the repository toolchain available
- Git
- OpenAI Codex CLI installed and authenticated for Codex routes
- Google Antigravity IDE running for Argon and Claude RPC routes

## Install from a clone

```bash
git clone https://github.com/limhaneul12/casoas.git
cd casoas
make ci
cargo install --locked --path crates/hcodex
hcodex --workspace "$PWD" setup
hcodex status
```

`cargo install` writes the binary to Cargo's configured binary directory. Ensure that directory is on `PATH` before running `hcodex setup`.

The installed binary embeds a hash-verified plugin and skill bundle, so `hcodex setup` and `hcodex sync` also work from an arbitrary project without the original checkout. When matching checkout assets are explicitly present, their inventory and bytes must match the embedded bundle; mismatches fail rather than silently installing divergent instructions. No Python runtime is required.

`hcodex setup` performs the user-scoped changes explicitly requested by the operator:

- installs the Antigravity plugin under `~/.gemini/config/plugins/heterarchy-codex/`;
- installs the skill under Gemini and Codex skill directories;
- adds the `heterarchy_codex` MCP entry to `~/.codex/config.toml` using the resolved Rust binary path.

The configuration writer preserves existing TOML comments and permissions and uses a same-directory atomic replacement.

## Install without activating integrations

The repository helper installs only the binary by default:

```bash
./scripts/install.sh
```

To run the explicit setup step after installation, using the binary from `CARGO_INSTALL_ROOT` or `CARGO_HOME` directly:

```bash
./scripts/install.sh --activate
```

## Manual MCP configuration

For clients using JSON MCP configuration:

```json
{
  "mcpServers": {
    "heterarchy-codex": {
      "command": "hcodex",
      "args": ["mcp"]
    }
  }
}
```

For Codex TOML configuration, `hcodex setup` resolves the installed binary and writes the corresponding `[mcp_servers.heterarchy_codex]` entry.

## Update and disable

```bash
git pull --ff-only
make ci
cargo install --locked --path crates/hcodex --force
hcodex --workspace "$PWD" sync
hcodex --workspace "$PWD" restart
```

Disable the managed plugin and MCP entry with:

```bash
hcodex off
```

`off` stops only MCP processes registered and identity-verified as owned by `hcodex`; it does not use broad process matching.

## Verification

```bash
hcodex --version
hcodex status
hcodex tui -c "/status"
```

The source repository's canonical development gate is `make ci`. All production entry points and active tests are Rust-native; historical data compatibility uses frozen SQLite fixtures.
