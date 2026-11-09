#!/bin/sh
set -eu
repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
if [ -x "$repo_root/.tooling/cargo/bin/cargo" ]; then
  export CARGO_HOME="$repo_root/.tooling/cargo"
  export RUSTUP_HOME="$repo_root/.tooling/rustup"
  export PATH="$CARGO_HOME/bin:$PATH"
fi
exec cargo "$@"
