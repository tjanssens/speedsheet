# SpeedSheet: notes for Claude

- All spreadsheet behaviour belongs in `crates/speedsheet-core` with a test in `crates/speedsheet-core/tests/` or a unit test. The Tauri shell (`src-tauri`) and UI (`src`) stay thin.
- New UI-facing operations: add a command to `src-tauri/src/api.rs::dispatch`, call it from TS with `call("name", {...})`.
- Speed is the product. Run `cargo run --release -p speedsheet-core --example bench` before and after changes that touch load, recalc, sort, filter or search, and mention the numbers in the PR.
- UI can be exercised in a browser: `cargo run --release -p speedsheet --example dev_bridge -- <file>` + `npm run dev`, then Playwright against http://localhost:1420. `npx playwright test` runs the browser tests in `e2e/` (it starts both itself; each test opens its own copy of the test file).
- `docs/features.md` maps every feature to how it is reached, its code and its tests. Adding a feature means adding a row; `e2e/features.spec.ts` fails when a toolbar or engine command is missing from it.
- Issues labelled `feedback` come from the in-app feedback button: reproduce with a test first, then fix.
- Formatting: `testdata/styled-members.xlsx` (from `cargo run -p speedsheet-core --example gen_styled`) is the visual test file; compare the dev bridge UI against `soffice --headless --convert-to pdf` of it.

## Quality pipeline (what CI enforces, and how to work with it)

Run `scripts/check.sh` before pushing (`--full` adds the UI browser tests, coverage and bench). CI (`.github/workflows/ci.yml`) runs everything in parallel; the one required check is **CI ok**.

- **Bugs: test first.** Write the test that fails, see it fail, then fix. Prefer making a general test catch it: the property tests in `tests/invariants.rs` (incremental recalc == full recalc, undo/redo restores everything, save/open keeps cells) or the corpus in `tests/corpus.rs`. If an invariant test finds a bug you cannot fix now, copy the test, switch the triggering op off in the main one, and mark the copy `#[ignore = "known bug: ..."]`; CI's "Known bugs" summary shows when it passes.
- **Ratchets only shrink**: `KNOWN_MISMATCHES` and the known-silent-loss lists in `tests/corpus.rs`, `eslint-suppressions.json` (prune with `npx eslint --prune-suppressions src`), `.cargo/mutants.toml` excludes. Never add to them to get a PR green, unless it is a found problem you report in the PR. The coverage floor (`.github/coverage-floor`) only goes up: raise it when coverage rose.
- **No silent data loss.** A file feature SpeedSheet does not keep must be listed in `Workbook::losses` (the save warning). `tests/corpus.rs` fails on anything that disappears on save without a warning. New feature in the corpus: add it to `examples/gen_corpus.rs` and run `scripts/refresh-corpus.sh` (needs LibreOffice, which also computes the cached values the oracle test compares with).
- **Property test failure**: proptest prints a shrunk minimal case and saves its seed in `tests/*.proptest-regressions`. Commit that file with the fix, and add a plain regression test for the minimal case. Nightly runs 50x more cases in two time zones.
- **Mutation test** (PRs, changed core lines): a "missed" mutant is a deliberate bug no test noticed. Add or sharpen a test. For an equivalent mutant, rewrite the code so it disappears, or as a last resort exclude it in `.cargo/mutants.toml` with a reason.
- **Performance**: CI fails when an operation is over its budget in `bench/budget.txt` or >25% slower than main. New benchmark operation: add a budget line. Raising a budget needs a reason in the PR.
- **Time**: core tests run in UTC+14 and UTC-11 in CI. Never assume UTC for anything the user sees.
- **Panics** are crashes in the app. Text parsing (formulas, input, number formats) is fuzzed in `tests/fuzz.rs`; slice strings with `.get(..)`, not `[..]`, when the index comes from input.
- **Rules are lint, not prose.** What used to be advice here is now enforced; fix the code, don't silence the rule.
  - Core (`clippy`): no string slicing with `[..]` (`clippy::string_slice`) and no `unwrap()` outside tests; use `.get(..)`, `split_once`, `?`. No direct clock reads (`Local::now`, `SystemTime::now`): go through `crate::clock` so time stays testable.
  - UI (`npm run lint`, type-aware ESLint): every promise is awaited or handed to `spawn()` (failures reach `showError`); no `setTimeout`/`setInterval`/`requestAnimationFrame` outside `src/schedule.ts` (use `Debounced`, `after`, `FrameTask`); no module-level `let` (state lives in one named object such as `app`); only `src/api.ts` talks to Tauri; files stay under 700 lines. `eslint-suppressions.json` is empty and stays empty.
- **UI tests.** Pure UI logic gets a vitest test next to it (`src/*.test.ts`, `npm test`). Browser tests in `e2e/`: `smoke`, `regressions` (one test per UI bug found), `features` (the feature map is complete), `screens` (light/dark screenshots, CI artifact `ui-screens`: put them in UI PRs) and `ui-fuzz`, a property test that drives the grid randomly and checks rules after each step. A fuzz failure prints its seed: replay with `UI_FUZZ_SEED=<seed> npx playwright test ui-fuzz`, add a test to `regressions.spec.ts`, then fix. Nightly runs 3 x 3000 steps.
- **Release test.** After each release an agent uses the app like a user, following `docs/release-test.md`, and files what it finds as `feedback` issues.
