# SpeedSheet feature map

Every feature as the user sees it: how to reach it, where its code lives, and which tests cover
it. It is the shared vocabulary for agents and people ("the filter button" means the same thing
to everyone), the checklist the release test agent works through (see `docs/release-test.md`),
and a map of what has no test yet.

`e2e/features.spec.ts` keeps this file honest: every toolbar command (`data-cmd` in
`index.html`) and every engine command (`api.rs::dispatch`) must appear here in backticks, and
every file named here must exist. Adding a feature means adding a row.

UI code is in `src/`, engine code in `crates/speedsheet-core/src/`, core tests in
`crates/speedsheet-core/tests/`, UI tests in `e2e/` and `src/*.test.ts`.

## Files

| Feature | How | Code | Tests | Engine commands |
|---|---|---|---|---|
| Open a file (xlsx, xlsm, xls, xlsb, ods, csv, tsv) | toolbar `open`, Ctrl+O, drag a file onto the window, open with SpeedSheet | `src/files.ts`, `crates/speedsheet-core/src/io.rs` | `workbook.rs`, `corpus.rs`, `formatting.rs`, `smoke.spec.ts` | `open_file`, `book_info` |
| New workbook | toolbar `new`, Ctrl+N | `src/files.ts` | `ui-fuzz.spec.ts` (indirectly) | `new_book` |
| New window | Ctrl+Shift+N (desktop) | `src-tauri/src/lib.rs` | none | |
| Save, save as | toolbar `save`, `saveAs`, Ctrl+S, Ctrl+Shift+S | `src/files.ts`, `crates/speedsheet-core/src/io.rs` | `invariants.rs` (save/open keeps cells), `corpus.rs`, `feedback.rs` | `save_file` |
| Warning before saving would drop content | save over a file with charts, validation, ... | `src/files.ts` (`confirmLosses`), `Workbook::losses` | `corpus.rs`, `feedback.rs` | `save_file` |
| Macros kept (.xlsm) | open and save an .xlsm | `crates/speedsheet-core/src/io.rs` | `feedback.rs` | |
| Undo, redo | toolbar `undo`, `redo`, Ctrl+Z, Ctrl+Y, Ctrl+Shift+Z | `src/files.ts` (`cmdUndo`), `crates/speedsheet-core/src/workbook.rs` | `invariants.rs` (undo/redo restores everything), `regressions.spec.ts`, `smoke.spec.ts` | `undo`, `redo` |

## Moving and selecting

| Feature | How | Code | Tests | Engine commands |
|---|---|---|---|---|
| Move the cursor | arrows, Enter, Tab, Home, End, PageUp, PageDown | `src/keyboard.ts`, `src/grid.ts` | `ui-fuzz.spec.ts`, `cells.test.ts` | |
| Jump to the edge of the data | Ctrl+Arrow (Shift extends) | `crates/speedsheet-core/src/nav.rs`, `src/keyboard.ts` | `nav.rs`, `ui-fuzz.spec.ts` | `jump` |
| Go to an address | type it in the name box, Enter | `src/editing.ts` | `smoke.spec.ts` | |
| Select cells, rows, columns, everything | click, Shift+click, drag, row/column headers, corner, Ctrl+A | `src/grid-mouse.ts`, `src/grid.ts` | `ui-fuzz.spec.ts` | |
| Walk through a selection | Enter/Tab inside a multi-cell selection | `src/cells.ts` (`advanceWithin`) | `cells.test.ts` | |
| Selection statistics | status bar: sum, average, min, max, count | `src/book.ts` (`updateStatus`), `crates/speedsheet-core/src/ops.rs` | none in the UI | `range_stats` |

## Editing

| Feature | How | Code | Tests | Engine commands |
|---|---|---|---|---|
| Type into a cell | type, F2, double-click; Enter/Tab/arrows commit, Escape cancels, Alt+Enter new line | `src/editing.ts` | `smoke.spec.ts`, `ui-fuzz.spec.ts` | `set_inputs`, `get_input` |
| Formula bar | click it, edit, Enter | `src/editing.ts` | `smoke.spec.ts` | `get_input` |
| Formulas (Excel functions, names, cross-sheet) | type `=...` | `crates/speedsheet-core/src/formula/` | `workbook.rs`, `invariants.rs`, `corpus.rs` (vs LibreOffice), `fuzz.rs`, `stress.rs` | |
| Formula help: colored references, point mode, autocomplete, argument hints | while typing a formula | `src/formula-edit.ts`, `src/formulaHelp.ts` | none | |
| Copy, cut, paste | Ctrl+C, Ctrl+X, Ctrl+V, context menu | `src/clipboard.ts`, `crates/speedsheet-core/src/ops.rs` | none in the UI | `copy_range`, `paste` |
| Clear contents | Delete, Backspace, context menu | `src/clipboard.ts` | `workbook.rs`, `ui-fuzz.spec.ts` | `clear_range` |
| Fill handle | drag the square at the selection's corner; double-click fills down | `src/grid-mouse.ts`, `crates/speedsheet-core/src/fill.rs` | `workbook.rs`, `cells.test.ts` (`fillTarget`) | `fill`, `fill_auto` |

## Data

| Feature | How | Code | Tests | Engine commands |
|---|---|---|---|---|
| Sort | toolbar `sortAsc`, `sortDesc`, context menu, filter popover | `src/sort-filter.ts`, `crates/speedsheet-core/src/workbook.rs` | `workbook.rs`, `feedback.rs`, `formatting.rs` | `sort` |
| Filter buttons on the header row | toolbar `filter`, Ctrl+Shift+L | `src/sort-filter.ts` | `formatting.rs` | `toggle_autofilter` |
| Filter popover (values, conditions) | filter button on a column | `src/popover.ts`, `crates/speedsheet-core/src/filter.rs` | `workbook.rs` | `set_filter`, `column_values` |
| Clear all filters | toolbar `clearFilters` | `src/sort-filter.ts` | none | `set_filter` |
| Find | toolbar `find`, Ctrl+F, Enter/Shift+Enter, "all sheets" | `src/commands.ts`, `Workbook::find` | `workbook.rs` | `find` |

## Sheets, layout and looks

| Feature | How | Code | Tests | Engine commands |
|---|---|---|---|---|
| Sheet tabs: switch, add, rename, delete | click, +, double-click, right-click | `src/book.ts` | `workbook.rs`, `feedback.rs` | `add_sheet`, `rename_sheet`, `delete_sheet` |
| Column width, row height, autofit | drag a header border, double-click it | `src/grid-mouse.ts`, `src/commands.ts` | none in the UI | `set_col_width`, `set_row_height` |
| Excel formatting shown (fonts, fills, borders, number formats, merges, frozen panes, conditional formats) | open a formatted file | `src/grid-paint.ts`, `crates/speedsheet-core/src/style.rs`, `crates/speedsheet-core/src/layout.rs`, `crates/speedsheet-core/src/numfmt.rs` | `formatting.rs`, `screens.spec.ts` (screenshots) | `get_cells`, `get_styles`, `get_layout` |
| Theme: system, light, dark, light sheet | toolbar `theme` | `src/commands.ts` | `screens.spec.ts` (screenshots) | |
| Print | toolbar `print`, Ctrl+P | `src/commands.ts`, `crates/speedsheet-core/src/ops.rs` | none | `print_html` |

## Windows, feedback, updates

| Feature | How | Code | Tests | Engine commands |
|---|---|---|---|---|
| Same file in two windows, kept in sync, other window's cursor | open the file again, "Open here too" | `src/peers.ts`, `crates/speedsheet-core/src/shared.rs` | `shared.rs` | `find_open`, `set_presence`, `peers`, `close_view` |
| Feedback button | toolbar `feedback` | `src/feedback.ts`, `feedback-worker/` | none | |
| Updates | toolbar `updates`, quietly at start | `src/updates.ts` | none | |

## No test yet

Formula help, copy/paste in the UI, column and row resizing, clear all filters, print, feedback,
updates and new window have no automated test. The release test agent checks them by hand
(`docs/release-test.md`); a bug found there gets a test first.
