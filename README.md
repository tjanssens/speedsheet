# SpeedSheet ⚡

A fast, lightweight spreadsheet editor for Windows, macOS and Linux. Open an Excel or CSV file,
edit it, save it. Built to stay smooth with hundreds of thousands of rows.

- Opens `.xlsx`, `.xlsm`, `.xls`, `.xlsb`, `.ods`, `.csv`/`.tsv`; saves `.xlsx` and `.csv`
- Multiple sheets, formulas (90+ common functions), sort, filter, find, undo/redo
- Copy/paste with Excel, Google Sheets, Numbers and other apps (formulas move along when copied inside SpeedSheet)
- Print, or save as PDF from the print dialog
- Feedback button that files a GitHub issue

## Speed

Measured on the included `testdata/large-500k.xlsx` (500,000 rows × 10 columns, 500,000 formulas),
release build, one CPU core of a CI-class Linux machine:

| Operation | Time |
|---|---|
| Open | ~2.1 s |
| Edit a cell + recalculate dependents (first edit builds the dependency index) | ~0.2 s |
| Sort 500k rows (formulas move with their rows) | ~1.1 s |
| Filter | ~0.09 s |
| Search with no match (full scan) | ~0.5 s |
| Save as xlsx | ~3.8 s |

Scrolling cost does not depend on the file size: the grid is drawn on a canvas and only fetches
the cells that are visible. Run the benchmark yourself:

```sh
cargo run --release -p speedsheet-core --example bench -- testdata/large-500k.xlsx
```

Regenerate the test file (or a bigger one): `cargo run --release -p speedsheet-core --example gen_large -- 1000000 testdata/large-1m.xlsx`.

## Architecture

```
crates/speedsheet-core   All spreadsheet logic, no UI: file IO (calamine / rust_xlsxwriter / csv),
                         cell model, formula parser + evaluator with incremental recalculation,
                         sort, filter, search, undo/redo, clipboard and print helpers.
src-tauri                Thin Tauri 2 shell. api.rs maps one IPC command onto the core.
src                      TypeScript UI: canvas-virtualized grid (grid.ts), app glue (main.ts),
                         filter popover, feedback dialog. No framework.
feedback-worker          Cloudflare Worker that turns feedback into GitHub issues.
```

Because the core is UI-independent, a fully native Rust UI can replace the Tauri shell later
without touching the engine.

## Developing

Requirements: Rust (stable), Node 22, and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```sh
npm install
npm run tauri dev          # desktop app with hot reload
cargo test -p speedsheet-core
```

UI work in a normal browser (handy for quick iteration and Playwright tests): run the dev bridge,
which serves the same API over HTTP, then the Vite dev server:

```sh
cargo run --release -p speedsheet --example dev_bridge -- testdata/large-500k.xlsx
npm run dev                # open http://localhost:1420
```

## Releasing

Bump `version` in `package.json` and `Cargo.toml`, then push a tag:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

The **Release** workflow builds installers for Windows (`.msi`, `.exe`), macOS (Apple Silicon and
Intel `.dmg`) and Linux (`.AppImage`, `.deb`, `.rpm`) and attaches them to a draft GitHub release.
Builds are not code-signed yet, so Windows SmartScreen and macOS Gatekeeper will warn on first launch.

## Automatic updates

The app checks the latest GitHub release on startup and offers *Install and restart* when a newer
version exists (there is also a *Check for updates* button). Updates are verified with a signature,
so releases must be signed. One-time setup:

1. `npx tauri signer generate -w ~/.tauri/speedsheet.key` (choose a password, keep the key safe: losing it means existing installs can't update).
2. GitHub repo → Settings → Secrets and variables → Actions:
   - Secret `TAURI_SIGNING_PRIVATE_KEY`: contents of `~/.tauri/speedsheet.key`
   - Secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the password
   - Variable `UPDATER_PUBKEY`: contents of `~/.tauri/speedsheet.key.pub`
3. Publish the draft release after the Release workflow finishes: the app only sees published releases.

Without these settings releases still build, just without the update channel.

## Feedback button

See [feedback-worker/README.md](feedback-worker/README.md). Without the proxy configured, the button
opens a prefilled GitHub issue page instead.

## Known limitations (v0.1)

- Cell formatting (fonts, colours, borders, number formats other than dates) is not shown and is
  not written back when saving. Values, formulas, dates, sheets and column widths are kept.
- `.xls`, `.xlsb` and `.ods` files are read-only: saving them writes a new `.xlsx`.
- No charts, merged cells, conditional formatting, pivot tables or macros.
- Unsupported functions show `#NAME?` after the cell is recalculated; their stored values from
  Excel are kept until then.
