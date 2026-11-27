//! Reading and writing files: xlsx/xlsm/xls/xlsb/ods via calamine, xlsx via rust_xlsxwriter, CSV.

use std::collections::HashMap;
use std::io::{BufReader, Read, Seek};
use std::path::Path;
use std::sync::Arc;

use calamine::{CellErrorType, Data, DataRef, Reader};

use crate::layout::BookLayout;
use crate::sheet::{Cell, Sheet};
use crate::style::CellStyle;
use crate::value::{CellError, Value};
use crate::workbook::{DefinedName, FileFormat, VbaProject, Workbook};
use crate::{Error, Result};

pub fn format_for_path(path: &Path) -> Result<FileFormat> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    Ok(match ext.as_str() {
        "xlsx" | "xlsm" | "xltx" | "xltm" => FileFormat::Xlsx,
        "xls" | "xla" => FileFormat::Xls,
        "xlsb" => FileFormat::Xlsb,
        "ods" => FileFormat::Ods,
        "csv" | "tsv" | "txt" => FileFormat::Csv,
        other => return Err(Error::Unsupported(other.to_string())),
    })
}

/// Open any supported spreadsheet file.
pub fn open(path: &Path) -> Result<Workbook> {
    let format = format_for_path(path)?;
    let mut wb = match format {
        FileFormat::Csv => {
            let bytes = std::fs::read(path)?;
            let tsv = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("tsv"));
            let (sheet, delim) = read_csv(&bytes, sheet_name_from_path(path), if tsv { Some(b'\t') } else { None })?;
            let mut wb = Workbook::from_sheets(vec![sheet], FileFormat::Csv);
            wb.csv_delimiter = delim;
            wb
        }
        FileFormat::Xlsx => {
            // Layout (styles per cell, widths, heights, merges...) is read on a second thread.
            let layout_file = std::fs::File::open(path)?;
            let layout = std::thread::spawn(move || {
                crate::layout::read_book_layout(BufReader::with_capacity(1 << 20, layout_file), u16::MAX as usize + 1)
            });
            let file = BufReader::with_capacity(1 << 20, std::fs::File::open(path)?);
            let mut x: calamine::Xlsx<_> = calamine::Xlsx::new(file).map_err(|e| Error::Read(e.to_string()))?;
            let (mut sheets, palette) = read_xlsx_sheets(&mut x)?;
            let mut layout = layout.join().unwrap_or_else(|_| {
                let mut l = BookLayout::default();
                l.losses.insert(crate::layout::UNCHECKED);
                l
            });
            // Names first: formulas without a cached value are computed when the workbook is built.
            let names = take_names(&mut sheets, &mut layout);
            let mut wb = Workbook::from_sheets_named(sheets, FileFormat::Xlsx, names);
            let macro_file = is_macro_path(path);
            if let Some(project) = layout.vba.take() {
                if macro_file {
                    wb.vba =
                        Some(VbaProject { project, signature: layout.vba_signature.take(), codename: layout.codename.take() });
                } else {
                    layout.losses.insert("Macros (VBA)");
                }
            }
            apply_layout(&mut wb, palette, layout);
            wb
        }
        _ => {
            let mut s = calamine::open_workbook_auto(path).map_err(|e| Error::Read(e.to_string()))?;
            Workbook::from_sheets(read_generic_sheets(&mut s)?, format)
        }
    };
    wb.path = Some(path.to_path_buf());
    wb.modified = false;
    Ok(wb)
}

/// .xlsm / .xltm: a macro-enabled package.
fn is_macro_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("xlsm") || e.eq_ignore_ascii_case("xltm"))
}

/// Can this defined name be written back?
fn name_writable(name: &str, formula: &str) -> bool {
    // Names pointing into other workbooks would dangle: their links are not kept.
    rust_xlsxwriter::utility::check_name(name).is_ok() && !formula.contains('[')
}

/// Move the defined names read from workbook.xml to their sheets; returns the workbook-wide ones.
fn take_names(sheets: &mut [Sheet], layout: &mut BookLayout) -> Vec<DefinedName> {
    let mut global = Vec::new();
    for n in std::mem::take(&mut layout.names) {
        if !name_writable(&n.name, &n.formula) {
            layout.losses.insert("Some defined names");
        }
        let d = DefinedName { name: n.name, formula: n.formula, hidden: n.hidden };
        match n.sheet {
            None => global.push(d),
            Some(s) => {
                if let Some(sh) = sheets.iter_mut().find(|x| x.name == s) {
                    sh.names.push(d);
                }
            }
        }
    }
    global
}

fn sheet_name_from_path(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Sheet1");
    let clean: String = stem.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
    if clean.is_empty() {
        "Sheet1".into()
    } else {
        clean
    }
}

fn map_error(e: &CellErrorType) -> CellError {
    match e {
        CellErrorType::Div0 => CellError::Div0,
        CellErrorType::NA | CellErrorType::GettingData => CellError::NA,
        CellErrorType::Name => CellError::Name,
        CellErrorType::Null => CellError::Null,
        CellErrorType::Num => CellError::Num,
        CellErrorType::Ref => CellError::Ref,
        CellErrorType::Value => CellError::Value,
    }
}

fn place(rows: &mut Vec<Vec<Cell>>, r: usize, c: usize, cell: Cell) {
    if cell.is_empty() || r >= crate::MAX_ROWS || c >= crate::MAX_COLS {
        return;
    }
    if rows.len() <= r {
        rows.resize_with(r + 1, Vec::new);
    }
    let row = &mut rows[r];
    if row.len() <= c {
        row.resize_with(c + 1, Cell::default);
    }
    row[c] = cell;
}

/// Fast path for xlsx: one streaming pass yields both cached values and formulas,
/// and shared strings are de-duplicated into one allocation each.
fn read_xlsx_sheets<RS: Read + Seek>(x: &mut calamine::Xlsx<RS>) -> Result<(Vec<Sheet>, Vec<CellStyle>)> {
    let names = x.sheet_names();
    let mut out = Vec::with_capacity(names.len());
    let mut interned: HashMap<usize, Arc<str>> = HashMap::new();
    let mut palette: Vec<CellStyle> = Vec::new();
    for name in names {
        let mut reader = x.worksheet_cells_reader(&name).map_err(|e| Error::Read(e.to_string()))?;
        if palette.is_empty() {
            palette = reader.styles().iter().take(u16::MAX as usize + 1).map(CellStyle::from_calamine).collect();
        }
        let dims = reader.dimensions();
        let mut rows: Vec<Vec<Cell>> =
            Vec::with_capacity((dims.end.0.saturating_sub(dims.start.0) as usize + 1).min(crate::MAX_ROWS));
        while let Some(rec) = reader.next_cell_with_formula().map_err(|e| Error::Read(e.to_string()))? {
            let (r, c) = (rec.pos.0 as usize, rec.pos.1 as usize);
            let value = match rec.value {
                DataRef::Empty => Value::Empty,
                DataRef::Int(i) => Value::Number(i as f64),
                DataRef::Float(f) => Value::Number(f),
                DataRef::Bool(b) => Value::Bool(b),
                DataRef::SharedString(s) => {
                    let key = s.as_ptr() as usize;
                    Value::Text(interned.entry(key).or_insert_with(|| Arc::from(s)).clone())
                }
                DataRef::String(s) => Value::text(s),
                DataRef::DateTime(d) => {
                    if d.is_duration() {
                        Value::Number(d.as_f64())
                    } else {
                        Value::Date(d.as_f64())
                    }
                }
                DataRef::DateTimeIso(s) | DataRef::DurationIso(s) => crate::input::parse_value(&s),
                DataRef::Error(e) => Value::Error(map_error(&e)),
            };
            let cell = match rec.formula {
                Some(f) if !f.is_empty() => Cell::formula(f.strip_prefix('=').unwrap_or(&f), value),
                _ => Cell::from_value(value),
            };
            place(&mut rows, r, c, cell);
        }
        drop(reader);
        out.push(Sheet::from_rows(name, rows));
    }
    Ok((out, palette))
}

/// Merge the separately read layout into the sheets.
fn apply_layout(wb: &mut Workbook, palette: Vec<CellStyle>, mut layout: BookLayout) {
    if !palette.is_empty() {
        wb.styles = palette;
    }
    wb.dxfs = std::mem::take(&mut layout.dxfs);
    let mut losses = std::mem::take(&mut layout.losses);
    let n = wb.styles.len();
    for sh in wb.sheets.iter_mut() {
        let Some(l) = layout.sheets.remove(&sh.name) else { continue };
        losses.extend(l.losses.iter());
        sh.codename = l.codename;
        for (r, c, s) in l.styles {
            if (s as usize) < n && (r as usize) < crate::MAX_ROWS && (c as usize) < crate::MAX_COLS {
                sh.set_style_id(r as usize, c as usize, s);
            }
        }
        sh.default_col_width = l.default_col_width.or(Some(64.0));
        sh.default_row_height = l.default_row_height.or(Some(20.0));
        for (c0, c1, w, hidden) in l.cols {
            let c1 = c1.min(crate::MAX_COLS as u32 - 1);
            // A range to the last column is really a default width.
            if c1 as usize == crate::MAX_COLS - 1 && c1 - c0 > 1000 {
                if !hidden {
                    if let Some(w) = w {
                        sh.default_col_width = Some(w);
                    }
                }
                continue;
            }
            for c in c0..=c1 {
                sh.set_col_width(c as usize, if hidden { Some(0.0) } else { w });
            }
        }
        for (r, h, hidden) in l.rows {
            if (r as usize) < crate::MAX_ROWS {
                sh.set_row_height(r as usize, if hidden { Some(0.0) } else { h });
            }
        }
        // Merged cells: only multi-cell ranges.
        sh.merges = l.merges.into_iter().filter(|m| m.r0 != m.r1 || m.c0 != m.c1).collect();
        sh.freeze = l.freeze;
        sh.autofilter = l.autofilter;
        sh.cond = l.cond;
        sh.hide_grid = !l.show_grid;
    }
    wb.losses = losses.into_iter().map(String::from).collect();
}

fn data_to_value(d: &Data) -> Value {
    match d {
        Data::Empty => Value::Empty,
        Data::Int(i) => Value::Number(*i as f64),
        Data::Float(f) => Value::Number(*f),
        Data::String(s) => Value::text(s.as_str()),
        Data::Bool(b) => Value::Bool(*b),
        Data::DateTime(d) => {
            if d.is_duration() {
                Value::Number(d.as_f64())
            } else {
                Value::Date(d.as_f64())
            }
        }
        Data::DateTimeIso(s) | Data::DurationIso(s) => crate::input::parse_value(s),
        Data::Error(e) => Value::Error(map_error(e)),
    }
}

fn read_generic_sheets<RS: Read + Seek>(s: &mut calamine::Sheets<RS>) -> Result<Vec<Sheet>> {
    let names = s.sheet_names();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let range = s.worksheet_range(&name).map_err(|e| Error::Read(e.to_string()))?;
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        if let Some((r0, c0)) = range.start() {
            for (dr, row) in range.rows().enumerate() {
                for (dc, d) in row.iter().enumerate() {
                    place(&mut rows, r0 as usize + dr, c0 as usize + dc, Cell::from_value(data_to_value(d)));
                }
            }
        }
        // Formulas are optional: older formats may not expose them.
        if let Ok(formulas) = s.worksheet_formula(&name) {
            if let Some((r0, c0)) = formulas.start() {
                for (dr, row) in formulas.rows().enumerate() {
                    for (dc, f) in row.iter().enumerate() {
                        if f.is_empty() {
                            continue;
                        }
                        let (r, c) = (r0 as usize + dr, c0 as usize + dc);
                        let cached = rows.get(r).and_then(|x| x.get(c)).map(|c| c.value().clone()).unwrap_or_default();
                        let src = f.strip_prefix('=').unwrap_or(f);
                        place(&mut rows, r, c, Cell::formula(src, cached));
                    }
                }
            }
        }
        out.push(Sheet::from_rows(name, rows));
    }
    Ok(out)
}

/// Decode bytes as UTF-8, falling back to Windows-1252 (what Excel on Windows writes).
fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            const HIGH: [char; 32] = [
                '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘',
                '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
            ];
            bytes.iter().map(|&b| if (0x80..0xA0).contains(&b) { HIGH[(b - 0x80) as usize] } else { b as char }).collect()
        }
    }
}

/// Pick the delimiter that splits the first lines most consistently.
fn sniff_delimiter(text: &str) -> u8 {
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).take(50).collect();
    let mut best = (b',', 0usize, false);
    for d in *b",;\t|" {
        let counts: Vec<usize> = lines
            .iter()
            .map(|l| {
                let mut n = 0;
                let mut quoted = false;
                for b in l.bytes() {
                    if b == b'"' {
                        quoted = !quoted;
                    } else if b == d && !quoted {
                        n += 1;
                    }
                }
                n
            })
            .collect();
        let Some(&first) = counts.first() else { continue };
        let consistent = first > 0 && counts.iter().all(|&c| c == first);
        let total: usize = counts.iter().sum();
        if (consistent && !best.2) || (consistent == best.2 && total > best.1) {
            best = (d, total, consistent);
        }
    }
    best.0
}

pub fn read_csv(bytes: &[u8], name: String, delimiter: Option<u8>) -> Result<(Sheet, u8)> {
    let text = decode_text(bytes);
    let delim = delimiter.unwrap_or_else(|| sniff_delimiter(&text));
    let mut rdr = csv::ReaderBuilder::new().delimiter(delim).has_headers(false).flexible(true).from_reader(text.as_bytes());
    let mut rows = Vec::new();
    let mut rec = csv::StringRecord::new();
    while rdr.read_record(&mut rec).map_err(|e| Error::Read(e.to_string()))? {
        if rows.len() >= crate::MAX_ROWS {
            break;
        }
        let row: Vec<Cell> =
            rec.iter().map(|f| if f.is_empty() { Cell::Empty } else { Cell::from_value(crate::input::parse_value(f)) }).collect();
        rows.push(row);
    }
    Ok((Sheet::from_rows(name, rows), delim))
}

/// What saving to a path writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveKind {
    Xlsx,
    /// Macro-enabled workbook: needs the macro project of the opened file.
    Xlsm,
    Csv,
}

fn save_kind(path: &Path) -> Result<SaveKind> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    Ok(match ext.as_str() {
        "xlsx" => SaveKind::Xlsx,
        "xlsm" => SaveKind::Xlsm,
        "csv" | "tsv" | "txt" => SaveKind::Csv,
        _ => return Err(Error::Unsupported("can only save as .xlsx, .xlsm (with macros) or .csv".into())),
    })
}

/// Can `wb` be saved back to the file it was opened from, in its own format?
/// (Not: .xls/.ods/.xlsb, templates, or an .xlsm whose macros could not be read.)
pub fn can_save_in_place(wb: &Workbook) -> bool {
    let Some(path) = &wb.path else { return false };
    wb.format.writable()
        && match save_kind(path) {
            Ok(SaveKind::Xlsm) => wb.vba.is_some(),
            Ok(_) => true,
            Err(_) => false,
        }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Save to `path`. CSV writes only `active_sheet`. The file is written to a temporary
/// name first and then renamed, so a failed save never destroys the original.
///
/// Saving over the opened file is refused with [`Error::WouldLoseData`] when that file holds
/// things SpeedSheet cannot keep ([`Workbook::losses`]); [`save_anyway`] overrides that after
/// the user agreed.
pub fn save(wb: &mut Workbook, path: &Path, active_sheet: usize) -> Result<()> {
    if !wb.losses.is_empty() && wb.path.as_deref().is_some_and(|p| same_file(p, path)) {
        return Err(Error::WouldLoseData(wb.losses.clone()));
    }
    save_anyway(wb, path, active_sheet)
}

/// [`save`] without the check for losses.
pub fn save_anyway(wb: &mut Workbook, path: &Path, active_sheet: usize) -> Result<()> {
    let kind = save_kind(path)?;
    if kind == SaveKind::Xlsm && wb.vba.is_none() {
        // Without a macro project the file would be a plain xlsx that Excel refuses to open
        // under an .xlsm name.
        return Err(Error::Unsupported("this workbook has no macros: save it as .xlsx".into()));
    }
    let format = if kind == SaveKind::Csv { FileFormat::Csv } else { FileFormat::Xlsx };
    let tmp = path.with_extension(format!("{}.speedsheet-tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
    let res = match kind {
        SaveKind::Csv => write_csv(wb, &tmp, active_sheet, path),
        SaveKind::Xlsx => write_xlsx(wb, &tmp, false),
        SaveKind::Xlsm => write_xlsx(wb, &tmp, true),
    };
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path)?;
    wb.path = Some(path.to_path_buf());
    wb.format = format;
    wb.modified = false;
    // The file on disk now holds exactly what the workbook does.
    wb.losses.clear();
    Ok(())
}

fn write_csv(wb: &Workbook, tmp: &Path, sheet: usize, target: &Path) -> Result<()> {
    let sh = wb.sheets.get(sheet).ok_or(Error::NoSuchSheet(sheet))?;
    let delim = if target.extension().is_some_and(|e| e.eq_ignore_ascii_case("tsv")) { b'\t' } else { wb.csv_delimiter };
    let file = std::fs::File::create(tmp)?;
    let mut w =
        csv::WriterBuilder::new().delimiter(delim).flexible(true).from_writer(std::io::BufWriter::with_capacity(1 << 20, file));
    let mut fields: Vec<String> = Vec::new();
    for row in &sh.rows {
        fields.clear();
        fields.extend(row.iter().map(|c| c.value().display()));
        if fields.is_empty() {
            w.write_record([""]).map_err(|e| Error::Write(e.to_string()))?;
        } else {
            w.write_record(&fields).map_err(|e| Error::Write(e.to_string()))?;
        }
    }
    w.flush()?;
    Ok(())
}

fn safe_sheet_name(name: &str, used: &mut Vec<String>) -> String {
    let mut n: String = name.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
    n = n.trim_matches('\'').to_string();
    if n.is_empty() {
        n = "Sheet".into();
    }
    let base = n.clone();
    let mut i = 2;
    while used.iter().any(|u| u.eq_ignore_ascii_case(&n)) {
        let suffix = format!(" ({i})");
        n = format!("{}{}", base.chars().take(31 - suffix.len()).collect::<String>(), suffix);
        i += 1;
    }
    used.push(n.clone());
    n
}

/// Write the macro project next to `tmp` (rust_xlsxwriter reads it from a file) and add it.
fn add_vba(x: &mut rust_xlsxwriter::Workbook, vba: &VbaProject, tmp: &Path) -> Result<()> {
    let werr = |e: rust_xlsxwriter::XlsxError| Error::Write(e.to_string());
    let bin = tmp.with_extension("vba-tmp");
    let sig = tmp.with_extension("vbasig-tmp");
    std::fs::write(&bin, &vba.project)?;
    let res = match &vba.signature {
        Some(s) => std::fs::write(&sig, s)
            .map_err(Error::from)
            .and_then(|_| x.add_vba_project_with_signature(&bin, &sig).map(|_| ()).map_err(werr)),
        None => x.add_vba_project(&bin).map(|_| ()).map_err(werr),
    };
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&sig);
    res?;
    if let Some(c) = &vba.codename {
        x.set_vba_name(c.as_str()).map_err(werr)?;
    }
    Ok(())
}

/// Excel's `Sheet!Name` form for a sheet-local name.
fn local_name(sheet: &str, name: &str) -> String {
    let plain = sheet.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.');
    if plain {
        format!("{sheet}!{name}")
    } else {
        format!("'{}'!{name}", sheet.replace('\'', "''"))
    }
}

fn write_xlsx(wb: &Workbook, tmp: &Path, macros: bool) -> Result<()> {
    use rust_xlsxwriter::{Format, Formula, Workbook as XWorkbook};
    let werr = |e: rust_xlsxwriter::XlsxError| Error::Write(e.to_string());
    let mut x = XWorkbook::new();
    if macros {
        if let Some(vba) = &wb.vba {
            add_vba(&mut x, vba, tmp)?;
        }
    }
    // One Format per style in use; dates without a number format get a readable one.
    let formats: Vec<Format> = wb.styles.iter().map(|s| s.to_xlsx()).collect();
    let date_fmts: Vec<(Format, Format)> = wb
        .styles
        .iter()
        .zip(&formats)
        .map(|(s, f)| {
            if s.number_format().is_some() {
                (f.clone(), f.clone())
            } else {
                (f.clone().set_num_format("yyyy-mm-dd"), f.clone().set_num_format("yyyy-mm-dd hh:mm:ss"))
            }
        })
        .collect();
    let default_fmt = Format::new();
    let mut used = Vec::new();
    // Defined names, with sheet-local ones under the sheet's name as written.
    let mut defined: Vec<(String, &str)> = wb.names.iter().map(|n| (n.name.clone(), n.formula.as_str())).collect();
    for sh in &wb.sheets {
        let ws = x.add_worksheet_with_low_memory().map_err(werr)?;
        let written = safe_sheet_name(&sh.name, &mut used);
        defined.extend(sh.names.iter().map(|n| (local_name(&written, &n.name), n.formula.as_str())));
        ws.set_name(written).map_err(werr)?;
        if macros && wb.vba.is_some() {
            if let Some(c) = &sh.codename {
                ws.set_vba_name(c.as_str()).map_err(werr)?;
            }
        }
        if let Some(h) = sh.default_row_height {
            ws.set_default_row_height_pixels(h.round().max(1.0) as u32);
        }
        for (c, w) in sh.col_widths.iter().enumerate() {
            let c = c as u16;
            match w {
                Some(px) if *px <= 0.0 => {
                    ws.set_column_hidden(c).map_err(werr)?;
                }
                Some(px) => {
                    ws.set_column_width_pixels(c, px.round() as u32).map_err(werr)?;
                }
                None => {
                    if let Some(d) = sh.default_col_width.filter(|d| (*d - 64.0).abs() > 0.5) {
                        ws.set_column_width_pixels(c, d.round() as u32).map_err(werr)?;
                    }
                }
            }
        }
        if let Some((r, c)) = sh.freeze {
            ws.set_freeze_panes(r, c as u16).map_err(werr)?;
        }
        if sh.hide_grid {
            ws.set_screen_gridlines(false);
        }
        if let Some(a) = sh.autofilter {
            ws.autofilter(a.r0, a.c0 as u16, a.r1, a.c1 as u16).map_err(werr)?;
        }
        write_cond_formats(ws, sh, wb).map_err(werr)?;
        let mut merges: Vec<_> = sh.merges.iter().collect();
        merges.sort_by_key(|m| (m.r0, m.c0));
        let mut next_merge = 0;
        let (nrows, _) = sh.extent();
        let nrows = nrows.max(sh.row_heights.len());
        for r in 0..nrows {
            let ri = r as u32;
            match sh.row_height(r) {
                Some(h) if h <= 0.0 => {
                    ws.set_row_hidden(ri).map_err(werr)?;
                }
                Some(h) => {
                    ws.set_row_height_pixels(ri, h.round() as u32).map_err(werr)?;
                }
                None => {}
            }
            while next_merge < merges.len() && merges[next_merge].r0 == ri {
                let m = merges[next_merge];
                let f = formats.get(sh.style_id(r, m.c0 as usize) as usize).unwrap_or(&default_fmt);
                // The top-left value is written below, over the merge's empty string.
                let _ = ws.merge_range(m.r0, m.c0 as u16, m.r1, m.c1 as u16, "", f);
                next_merge += 1;
            }
            let row = sh.rows.get(r).map(|v| v.as_slice()).unwrap_or(&[]);
            let srow = sh.styles.get(r).map(|v| v.as_slice()).unwrap_or(&[]);
            let width = row.len().max(srow.len());
            for c in 0..width {
                let cell = row.get(c).unwrap_or(&Cell::Empty);
                let sid = srow.get(c).copied().unwrap_or(0) as usize;
                let fmt = if sid == 0 { None } else { formats.get(sid) };
                let c = c as u16;
                match cell {
                    Cell::Empty | Cell::Value(Value::Empty) => {
                        if let Some(f) = fmt {
                            ws.write_blank(ri, c, f).map_err(werr)?;
                        }
                    }
                    Cell::Value(v) => match v {
                        Value::Empty => {}
                        Value::Number(n) => {
                            match fmt {
                                Some(f) => ws.write_number_with_format(ri, c, *n, f),
                                None => ws.write_number(ri, c, *n),
                            }
                            .map_err(werr)?;
                        }
                        Value::Date(n) => {
                            let pair = &date_fmts[sid.min(date_fmts.len() - 1)];
                            let f = if n.fract() == 0.0 { &pair.0 } else { &pair.1 };
                            ws.write_number_with_format(ri, c, *n, f).map_err(werr)?;
                        }
                        Value::Text(t) => {
                            match fmt {
                                Some(f) => ws.write_string_with_format(ri, c, t.as_ref(), f),
                                None => ws.write_string(ri, c, t.as_ref()),
                            }
                            .map_err(werr)?;
                        }
                        Value::Bool(b) => {
                            match fmt {
                                Some(f) => ws.write_boolean_with_format(ri, c, *b, f),
                                None => ws.write_boolean(ri, c, *b),
                            }
                            .map_err(werr)?;
                        }
                        Value::Error(e) => {
                            let src = if *e == CellError::NA { "=NA()".to_string() } else { format!("=\"{}\"", e.as_str()) };
                            let formula = Formula::new(src).set_result(e.as_str());
                            ws.write_formula_with_format(ri, c, formula, fmt.unwrap_or(&default_fmt)).map_err(werr)?;
                        }
                    },
                    Cell::Formula(f) => {
                        let result = match &f.value {
                            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
                            Value::Number(n) | Value::Date(n) => format!("{}", n),
                            v => v.display(),
                        };
                        let formula = Formula::new(format!("={}", f.src)).set_result(result);
                        let fm = if let Value::Date(n) = f.value {
                            let pair = &date_fmts[sid.min(date_fmts.len() - 1)];
                            if n.fract() == 0.0 {
                                &pair.0
                            } else {
                                &pair.1
                            }
                        } else {
                            fmt.unwrap_or(&default_fmt)
                        };
                        ws.write_formula_with_format(ri, c, formula, fm).map_err(werr)?;
                    }
                }
            }
        }
    }
    for (name, formula) in defined {
        let bare = name.rsplit('!').next().unwrap_or(&name);
        // Names that cannot be written were reported in `Workbook::losses` when the file opened.
        if name_writable(bare, formula) {
            x.define_name(name, &format!("={formula}")).map_err(werr)?;
        }
    }
    x.save(tmp).map_err(werr)?;
    Ok(())
}

fn write_cond_formats(
    ws: &mut rust_xlsxwriter::Worksheet,
    sh: &Sheet,
    wb: &Workbook,
) -> std::result::Result<(), rust_xlsxwriter::XlsxError> {
    use crate::layout::CfTest;
    use rust_xlsxwriter::{
        ConditionalFormatBlank, ConditionalFormatCell, ConditionalFormatCellRule as R, ConditionalFormatText,
        ConditionalFormatTextRule as T,
    };
    for cf in &sh.cond {
        for rule in &cf.rules {
            let fmt = wb.dxfs.get(rule.dxf as usize).map(|s| s.to_xlsx()).unwrap_or_default();
            for a in &cf.areas {
                let (r0, c0, r1, c1) = (a.r0, a.c0 as u16, a.r1, a.c1 as u16);
                match &rule.test {
                    CfTest::Cell { op, a: x, b } => {
                        // Operands are formulas: pass them through verbatim.
                        let x = rust_xlsxwriter::Formula::new(x.as_str());
                        let y = rust_xlsxwriter::Formula::new(b.as_deref().unwrap_or("0"));
                        let r = match op.as_str() {
                            "eq" => R::EqualTo(x),
                            "ne" => R::NotEqualTo(x),
                            "gt" => R::GreaterThan(x),
                            "ge" => R::GreaterThanOrEqualTo(x),
                            "lt" => R::LessThan(x),
                            "le" => R::LessThanOrEqualTo(x),
                            "between" => R::Between(x, y),
                            _ => R::NotBetween(x, y),
                        };
                        let c = ConditionalFormatCell::new().set_rule(r).set_format(fmt.clone()).set_stop_if_true(rule.stop);
                        ws.add_conditional_format(r0, c0, r1, c1, &c)?;
                    }
                    CfTest::ContainsText { text }
                    | CfTest::NotContainsText { text }
                    | CfTest::BeginsWith { text }
                    | CfTest::EndsWith { text } => {
                        let r = match &rule.test {
                            CfTest::ContainsText { .. } => T::Contains(text.clone()),
                            CfTest::NotContainsText { .. } => T::DoesNotContain(text.clone()),
                            CfTest::BeginsWith { .. } => T::BeginsWith(text.clone()),
                            _ => T::EndsWith(text.clone()),
                        };
                        let c = ConditionalFormatText::new().set_rule(r).set_format(fmt.clone()).set_stop_if_true(rule.stop);
                        ws.add_conditional_format(r0, c0, r1, c1, &c)?;
                    }
                    CfTest::Blank | CfTest::NotBlank => {
                        let mut c = ConditionalFormatBlank::new().set_format(fmt.clone()).set_stop_if_true(rule.stop);
                        if matches!(rule.test, CfTest::NotBlank) {
                            c = c.invert();
                        }
                        ws.add_conditional_format(r0, c0, r1, c1, &c)?;
                    }
                }
            }
        }
    }
    Ok(())
}
