#!/bin/sh
set -eu

repository_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

case "${1:-}" in
  ""|--activate) ;;
  *)
    printf '%s\n' "usage: $0 [--activate]" >&2
    exit 2
    ;;
esac

cargo_home=${CARGO_HOME:-"$HOME/.cargo"}
install_root=${CARGO_INSTALL_ROOT:-$cargo_home}
installed_bin="$install_root/bin/hcodex"

if [ -n "${HCODEX_CARGO:-}" ]; then
  cargo_bin=$HCODEX_CARGO
elif [ -x "$cargo_home/bin/cargo" ]; then
  cargo_bin="$cargo_home/bin/cargo"
else
  cargo_bin=$(command -v cargo)
fi

cd "$repository_dir"
"$cargo_bin" install --locked --path crates/hcodex

if [ "${1:-}" = "--activate" ]; then
  if [ ! -x "$installed_bin" ]; then
    printf '%s\n' "installed hcodex binary not found: $installed_bin" >&2
    exit 1
  fi
  "$installed_bin" --workspace "$repository_dir" setup
  printf '%s\n' "hcodex Rust binary installed and integrations activated."
else
  printf '%s\n' "hcodex Rust binary installed. Run '$installed_bin setup' to activate integrations and retire a known Python launcher."
fi
