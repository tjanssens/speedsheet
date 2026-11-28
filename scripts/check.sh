#!/usr/bin/env bash
# The fast local gate: the same checks as CI's required jobs, minus the slow ones.
#   scripts/check.sh          format, lint, types, core tests (about a minute when warm)
#   scripts/check.sh --full   plus UI browser tests (smoke, fuzz, screenshots), coverage floor, benchmark budgets
set -euo pipefail
cd "$(dirname "$0")/.."
step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

step "format";        cargo fmt --all --check
step "versions";      scripts/check-versions.sh
step "clippy";        cargo clippy --workspace --all-targets -- -D warnings
step "core tests";    cargo test -q -p speedsheet-core
step "UI lint+types"; npm run -s lint && npm run -s build >/dev/null
step "UI unit tests"; npm test --silent

if [ "${1:-}" = "--full" ]; then
  step "UI tests";      cargo build -q -p speedsheet --example dev_bridge && npx playwright test --trace off
  step "coverage";      cargo llvm-cov -q -p speedsheet-core --summary-only --fail-under-lines "$(cat .github/coverage-floor)" | tail -1
  step "benchmark";     cargo run -q --release -p speedsheet-core --example bench -- testdata/large-500k.xlsx | tee target/bench.txt
  python3 scripts/bench_check.py --budget bench/budget.txt --head target/bench.txt
fi
printf '\n\033[32mAll checks passed.\033[0m\n'
