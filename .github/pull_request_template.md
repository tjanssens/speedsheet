## What and why

<!-- One or two sentences. Link the issue (Fixes #123). -->

## How it is tested

<!-- Which test fails without this change? For a bug: the test reproduces it first. -->

## Checklist

- [ ] Behaviour lives in `crates/speedsheet-core` with a test; shell and UI stay thin
- [ ] Bug fix: a test that failed before the fix (and, if it fits, the general invariant in `tests/invariants.rs` or the corpus in `tests/corpus.rs` would have caught it)
- [ ] Nothing a user had can disappear silently: new file features are kept, or listed in `Workbook::losses`
- [ ] Ratchet lists (known mismatches, known silent losses, ESLint suppressions) only shrink
- [ ] Touches load, recalc, sort, filter, search or save: bench numbers below (CI posts them too)
- [ ] `scripts/check.sh` passes locally

## Bench (if relevant)

<!-- Paste the table from the "Performance" job summary, or before/after numbers. -->
