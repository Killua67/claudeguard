#!/bin/sh
set -eu
repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
if [ -x "$repo_root/.tooling/cargo/bin/cargo" ]; then
  export CARGO_HOME="$repo_root/.tooling/cargo"
  export RUSTUP_HOME="$repo_root/.tooling/rustup"
  export PATH="$CARGO_HOME/bin:$PATH"
fi
case "${1:-}" in
  build)
    cd "$repo_root/apps/desktop"
    npm exec -- tauri build --debug --bundles app
    # Local preview only. Distribution needs Developer ID + notarization.
    if [ "$(uname -s)" = Darwin ]; then
      preview_app="$repo_root/target/debug/bundle/macos/ClaudeGuard.app"
      /usr/bin/codesign --force --sign - "$preview_app"
      /usr/bin/codesign --verify --strict "$preview_app"
    fi
    ;;
  demo|demo-first-run|observe|run)
    # Development acceptance uses demo or immutable observe-only mode.
    case "$1" in demo) mode_arg=--demo ;; demo-first-run) mode_arg=--demo-first-run ;; observe) mode_arg=--observe-only ;; run) mode_arg= ;; esac
    preview_bundle="$repo_root/target/debug/bundle/macos/ClaudeGuard.app/Contents/MacOS/guard-desktop"
    if [ -x "$preview_bundle" ]; then
      exec "$preview_bundle" ${mode_arg:+"$mode_arg"}
    fi
    exec "$repo_root/target/debug/guard-desktop" ${mode_arg:+"$mode_arg"}
    ;;
  dev-demo)
    cd "$repo_root/apps/desktop"
    exec npm exec -- tauri dev -- --demo
    ;;
  *)
    printf '%s\n' 'Usage: scripts/desktop.sh build | observe (real checks, no launch) | run | demo | demo-first-run | dev-demo' >&2
    exit 2
    ;;
esac
