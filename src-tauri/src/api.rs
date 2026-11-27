//! The UI-facing API, independent of Tauri: one `dispatch` function that maps a command name
//! and JSON arguments onto speedsheet-core. The Tauri app routes its single `call` command here,
//! and the browser dev bridge (`examples/dev_bridge.rs`) does the same over HTTP.

use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use speedsheet_core::{io, ops, Change, FileFormat, Filter, Workbook};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetInfo {
    pub name: String,
    /// Rows in the current (possibly filtered) view.
    pub rows: usize,
    pub data_rows: usize,
    pub cols: usize,
    pub filter: Option<Filter>,
    pub header: bool,
    pub col_widths: Vec<Option<f32>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookInfo {
    pub path: Option<String>,
    pub name: String,
    pub format: FileFormat,
    pub writable: bool,
    pub modified: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub sheets: Vec<SheetInfo>,
}

pub fn info(wb: &Workbook) -> BookInfo {
    BookInfo {
        path: wb.path.as_ref().map(|p| p.display().to_string()),
        name: wb
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".into()),
        format: wb.format,
        writable: wb.format.writable(),
        modified: wb.modified,
        can_undo: wb.can_undo(),
        can_redo: wb.can_redo(),
        sheets: wb
            .sheets
            .iter()
            .map(|s| SheetInfo {
                name: s.name.clone(),
                rows: if s.view.is_some() { s.view_len() } else { s.extent().0 },
                data_rows: s.row_count(),
                cols: s.extent().1,
                filter: s.filter.clone(),
                header: s.autofilter.is_some() || s.filter.as_ref().map(|f| f.header).unwrap_or_else(|| ops::guess_header(s)),
                col_widths: s.col_widths.clone(),
            })
            .collect(),
    }
}

/// Layout of a sheet in view coordinates: what the grid needs besides cell contents.
fn layout(s: &speedsheet_core::Sheet) -> Json {
    // Row heights as runs of [first view row, count, height px].
    let mut runs: Vec<(usize, usize, f32)> = Vec::new();
    let mut push = |vr: usize, h: f32| match runs.last_mut() {
        Some(last) if last.0 + last.1 == vr && last.2 == h => last.1 += 1,
        _ => runs.push((vr, 1, h)),
    };
    match &s.view {
        None => {
            for (r, h) in s.row_heights.iter().enumerate() {
                if let Some(h) = h {
                    push(r, *h);
                }
            }
        }
        Some(v) => {
            if !s.row_heights.is_empty() {
                for (vr, &r) in v.iter().enumerate() {
                    if let Some(h) = s.row_height(r as usize) {
                        push(vr, h);
                    }
                }
            }
        }
    }
    let merges: Vec<Json> = s
        .merges
        .iter()
        .filter_map(|m| {
            let v0 = s.view_row(m.r0 as usize)?;
            let v1 = s.view_row(m.r1 as usize)?;
            (v1 - v0 == (m.r1 - m.r0) as usize).then(|| json!({ "r0": v0, "c0": m.c0, "r1": v1, "c1": m.c1 }))
        })
        .collect();
    let autofilter = s
        .autofilter
        .and_then(|a| s.view_row(a.r0 as usize).map(|vr| json!({ "row": vr, "dataRow": a.r0, "c0": a.c0, "c1": a.c1 })));
    json!({
        "rowHeights": runs,
        "defaultRowHeight": s.default_row_height,
        "defaultColWidth": s.default_col_width,
        "merges": merges,
        "freeze": s.freeze,
        "autofilter": autofilter,
        "hideGrid": s.hide_grid,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EditResult {
    info: BookInfo,
    sheet: usize,
    /// View row of the first changed cell (None when hidden by a filter).
    row: Option<usize>,
    col: usize,
    recalculated: usize,
}

fn edit_result(wb: &Workbook, ch: Change) -> EditResult {
    let sheet = ch.sheet.min(wb.sheets.len() - 1);
    EditResult { row: wb.sheets[sheet].view_row(ch.row), info: info(wb), sheet, col: ch.col, recalculated: ch.recalculated }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rect {
    sheet: usize,
    r0: usize,
    c0: usize,
    r1: usize,
    c1: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct At {
    sheet: usize,
    row: usize,
    col: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SheetArg {
    sheet: usize,
}

fn args<T: DeserializeOwned>(v: Json) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("bad arguments: {e}"))
}

fn ok<T: Serialize>(v: T) -> Result<Json, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn e(x: impl std::fmt::Display) -> String {
    x.to_string()
}

pub fn dispatch(wb: &mut Workbook, cmd: &str, a: Json) -> Result<Json, String> {
    let sheet_ok = |wb: &Workbook, s: usize| if s < wb.sheets.len() { Ok(()) } else { Err("no such sheet".to_string()) };
    match cmd {
        "open_file" => {
            #[derive(Deserialize)]
            struct A {
                path: String,
            }
            let A { path } = args(a)?;
            *wb = io::open(PathBuf::from(path).as_path()).map_err(e)?;
            ok(info(wb))
        }
        "new_book" => {
            *wb = Workbook::new();
            ok(info(wb))
        }
        "book_info" => ok(info(wb)),
        "save_file" => {
            #[derive(Deserialize)]
            struct A {
                path: Option<String>,
                sheet: usize,
            }
            let A { path, sheet } = args(a)?;
            let target = match path {
                Some(p) => PathBuf::from(p),
                None => wb.path.clone().ok_or("no file name yet")?,
            };
            io::save(wb, &target, sheet).map_err(e)?;
            ok(info(wb))
        }
        "get_cells" => {
            let r: Rect = args(a)?;
            sheet_ok(wb, r.sheet)?;
            ok(ops::block(wb, r.sheet, r.r0, r.r1, r.c0, r.c1))
        }
        "get_input" => {
            let At { sheet, row, col } = args(a)?;
            sheet_ok(wb, sheet)?;
            let sh = &wb.sheets[sheet];
            ok(sh.data_row(row).map(|r| sh.cell(r, col).input_text()).unwrap_or_default())
        }
        "set_inputs" => {
            #[derive(Deserialize)]
            struct A {
                sheet: usize,
                row: usize,
                col: usize,
                grid: Vec<Vec<String>>,
            }
            let A { sheet, row, col, grid } = args(a)?;
            let ch = wb.set_inputs(sheet, row, col, &grid).map_err(e)?;
            ok(edit_result(wb, ch))
        }
        "paste" => {
            // `internal` is set when the clipboard holds exactly what SpeedSheet copied last:
            // then the original inputs are pasted, with formulas shifted to the new position.
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Internal {
                row: usize,
                col: usize,
                inputs: Vec<Vec<String>>,
            }
            #[derive(Deserialize)]
            struct A {
                sheet: usize,
                row: usize,
                col: usize,
                text: String,
                internal: Option<Internal>,
            }
            let A { sheet, row, col, text, internal } = args(a)?;
            let grid = match internal {
                Some(i) => ops::shift_inputs(&i.inputs, row as i64 - i.row as i64, col as i64 - i.col as i64),
                None => ops::parse_tsv(&text),
            };
            let ch = wb.set_inputs(sheet, row, col, &grid).map_err(e)?;
            ok(edit_result(wb, ch))
        }
        "copy_range" => {
            let r: Rect = args(a)?;
            sheet_ok(wb, r.sheet)?;
            ok(ops::copy(wb, r.sheet, r.r0, r.c0, r.r1, r.c1))
        }
        "clear_range" => {
            let r: Rect = args(a)?;
            let ch = wb.clear(r.sheet, r.r0, r.c0, r.r1, r.c1).map_err(e)?;
            ok(edit_result(wb, ch))
        }
        "fill" => {
            #[derive(Deserialize)]
            struct R {
                r0: usize,
                c0: usize,
                r1: usize,
                c1: usize,
            }
            #[derive(Deserialize)]
            struct A {
                sheet: usize,
                src: R,
                dst: R,
            }
            let A { sheet, src, dst } = args(a)?;
            let ch = wb.fill(sheet, (src.r0, src.c0, src.r1, src.c1), (dst.r0, dst.c0, dst.r1, dst.c1)).map_err(e)?;
            ok(edit_result(wb, ch))
        }
        "fill_auto" => {
            let r: Rect = args(a)?;
            match wb.fill_down_auto(r.sheet, (r.r0, r.c0, r.r1, r.c1)).map_err(e)? {
                Some((end, ch)) => ok(json!({ "end": end, "result": edit_result(wb, ch) })),
                None => ok(Json::Null),
            }
        }
        "undo" => {
            let ch = wb.undo();
            ok(ch.map(|c| edit_result(wb, c)))
        }
        "redo" => {
            let ch = wb.redo();
            ok(ch.map(|c| edit_result(wb, c)))
        }
        "sort" => {
            #[derive(Deserialize)]
            struct A {
                sheet: usize,
                col: usize,
                ascending: bool,
                header: bool,
            }
            let A { sheet, col, ascending, header } = args(a)?;
            let ch = wb.sort(sheet, col, ascending, header).map_err(e)?;
            ok(edit_result(wb, ch))
        }
        "set_filter" => {
            #[derive(Deserialize)]
            struct A {
                sheet: usize,
                filter: Option<Filter>,
            }
            let A { sheet, filter } = args(a)?;
            wb.set_filter(sheet, filter).map_err(e)?;
            ok(info(wb))
        }
        "column_values" => {
            #[derive(Deserialize)]
            struct A {
                sheet: usize,
                col: usize,
                header: bool,
            }
            let A { sheet, col, header } = args(a)?;
            sheet_ok(wb, sheet)?;
            let (values, truncated) = speedsheet_core::filter::column_values(&wb.sheets[sheet], col, header, 10_000);
            ok(json!({ "values": values, "truncated": truncated }))
        }
        "find" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                query: String,
                sheet: usize,
                row: usize,
                col: usize,
                backwards: bool,
                all_sheets: bool,
            }
            let A { query, sheet, row, col, backwards, all_sheets } = args(a)?;
            sheet_ok(wb, sheet)?;
            ok(wb.find(&query, sheet, row, col, backwards, all_sheets))
        }
        "range_stats" => {
            let r: Rect = args(a)?;
            sheet_ok(wb, r.sheet)?;
            ok(ops::stats(&wb.sheets[r.sheet], r.r0, r.c0, r.r1, r.c1))
        }
        "add_sheet" => {
            wb.add_sheet(None);
            ok(info(wb))
        }
        "rename_sheet" => {
            #[derive(Deserialize)]
            struct A {
                sheet: usize,
                name: String,
            }
            let A { sheet, name } = args(a)?;
            wb.rename_sheet(sheet, name).map_err(e)?;
            ok(info(wb))
        }
        "delete_sheet" => {
            let SheetArg { sheet } = args(a)?;
            wb.delete_sheet(sheet).map_err(e)?;
            ok(info(wb))
        }
        "set_col_width" => {
            #[derive(Deserialize)]
            struct A {
                sheet: usize,
                col: usize,
                width: f32,
            }
            let A { sheet, col, width } = args(a)?;
            sheet_ok(wb, sheet)?;
            wb.sheets[sheet].set_col_width(col, Some(width));
            wb.modified = true;
            ok(())
        }
        "toggle_autofilter" => {
            // Excel's Data > Filter: turn the filter buttons off, or on for the selection (when it
            // spans several rows) or else the block of filled cells around the active cell.
            #[derive(Deserialize)]
            struct A {
                sheet: usize,
                r0: usize,
                c0: usize,
                r1: usize,
                c1: usize,
            }
            let A { sheet, r0, c0, r1, c1 } = args(a)?;
            sheet_ok(wb, sheet)?;
            let sh = &wb.sheets[sheet];
            let area = if sh.autofilter.is_some() {
                None
            } else {
                let d0 = sh.data_row(r0).ok_or("no such row")?;
                let d1 = sh.data_row(r1).unwrap_or(d0);
                Some(if d1 > d0 {
                    speedsheet_core::layout::Area { r0: d0 as u32, c0: c0 as u32, r1: d1 as u32, c1: c1 as u32 }
                } else {
                    let mut a = sh.current_region(d0, c0);
                    if c1 > c0 {
                        (a.c0, a.c1) = (c0 as u32, c1 as u32);
                    }
                    a
                })
            };
            wb.set_autofilter(sheet, area).map_err(e)?;
            ok(info(wb))
        }
        "set_row_height" => {
            #[derive(Deserialize)]
            struct A {
                sheet: usize,
                row: usize,
                height: Option<f32>,
            }
            let A { sheet, row, height } = args(a)?;
            sheet_ok(wb, sheet)?;
            let sh = &mut wb.sheets[sheet];
            let r = sh.data_row(row).ok_or("no such row")?;
            sh.set_row_height(r, height);
            wb.modified = true;
            ok(())
        }
        "get_styles" => ok(json!({ "styles": wb.styles, "dxfs": wb.dxfs })),
        "get_layout" => {
            let SheetArg { sheet } = args(a)?;
            sheet_ok(wb, sheet)?;
            ok(layout(&wb.sheets[sheet]))
        }
        "print_html" => {
            let r: Rect = args(a)?;
            sheet_ok(wb, r.sheet)?;
            let (html, rows, truncated) = ops::print_html(wb, r.sheet, r.r0, r.c0, r.r1, r.c1, 20_000);
            ok(json!({ "html": html, "rows": rows, "truncated": truncated }))
        }
        other => Err(format!("unknown command {other}")),
    }
}
