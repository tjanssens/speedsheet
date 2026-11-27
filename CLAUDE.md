# SpeedSheet: notes for Claude

- All spreadsheet behaviour belongs in `crates/speedsheet-core` with a test in `crates/speedsheet-core/tests/` or a unit test. The Tauri shell (`src-tauri`) and UI (`src`) stay thin.
- New UI-facing operations: add a command to `src-tauri/src/api.rs::dispatch`, call it from TS with `call("name", {...})`.
- Speed is the product. Run `cargo run --release -p speedsheet-core --example bench` before and after changes that touch load, recalc, sort, filter or search, and mention the numbers in the PR.
- Checks CI runs: `cargo test -p speedsheet-core`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `npm run build`.
- UI can be exercised in a browser: `cargo run --release -p speedsheet --example dev_bridge -- <file>` + `npm run dev`, then Playwright against http://localhost:1420.
- Issues labelled `feedback` come from the in-app feedback button: reproduce with a test first, then fix.
- Formatting: `testdata/styled-members.xlsx` (from `cargo run -p speedsheet-core --example gen_styled`) is the visual test file; compare the dev bridge UI against `soffice --headless --convert-to pdf` of it.
