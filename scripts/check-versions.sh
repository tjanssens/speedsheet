#!/usr/bin/env bash
# The app version lives in three files; a release built from mismatched files ships a wrong
# version number and breaks auto-update. Fail if they differ.
set -euo pipefail
cd "$(dirname "$0")/.."
pkg=$(node -p 'require("./package.json").version')
cargo=$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' Cargo.toml)
tauri=$(node -p 'const v = require("./src-tauri/tauri.conf.json").version ?? ""; v.startsWith("../") ? "" : v')
lock=$(node -p 'require("./package-lock.json").version')
echo "package.json=$pkg Cargo.toml=$cargo tauri.conf.json=${tauri:-(linked to package.json)} package-lock.json=$lock"
[ "$pkg" = "$cargo" ] && [ "$pkg" = "$lock" ] && { [ -z "$tauri" ] || [ "$tauri" = "$pkg" ]; } || {
  echo "::error::Version numbers differ; bump them together."
  exit 1
}
