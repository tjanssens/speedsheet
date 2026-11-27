//! Reading and writing files: xlsx/xlsm/xls/xlsb/ods via calamine, xlsx via rust_xlsxwriter, CSV.

use std::collections::HashMap;
use std::io::{BufReader, Read, Seek};
use std::path::Path;
use std::sync::Arc;

use calamine::{CellErrorType, Data, DataRef, Reader};

use crate::sheet::{Cell, Sheet};
use crate::value::{CellError, Value};
use crate::workbook::{FileFormat, Workbook};
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
            let file = BufReader::with_capacity(1 << 20, std::fs::File::open(path)?);
            let mut x: calamine::Xlsx<_> = calamine::Xlsx::new(file).map_err(|e| Error::Read(e.to_string()))?;
            Workbook::from_sheets(read_xlsx_sheets(&mut x)?, FileFormat::Xlsx)
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
fn read_xlsx_sheets<RS: Read + Seek>(x: &mut calamine::Xlsx<RS>) -> Result<Vec<Sheet>> {
    let names = x.sheet_names();
    let mut out = Vec::with_capacity(names.len());
    let mut interned: HashMap<usize, Arc<str>> = HashMap::new();
    for name in names {
        let mut reader = x.worksheet_cells_reader(&name).map_err(|e| Error::Read(e.to_string()))?;
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
    Ok(out)
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

/// Save to `path`. CSV writes only `active_sheet`. The file is written to a temporary
/// name first and then renamed, so a failed save never destroys the original.
pub fn save(wb: &mut Workbook, path: &Path, active_sheet: usize) -> Result<()> {
    let format = format_for_path(path)?;
    if !format.writable() {
        return Err(Error::Unsupported("can only save as .xlsx or .csv".into()));
    }
    let tmp = path.with_extension(format!("{}.speedsheet-tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
    let res = match format {
        FileFormat::Csv => write_csv(wb, &tmp, active_sheet, path),
        _ => write_xlsx(wb, &tmp),
    };
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path)?;
    wb.path = Some(path.to_path_buf());
    wb.format = format;
    wb.modified = false;
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

fn write_xlsx(wb: &Workbook, tmp: &Path) -> Result<()> {
    use rust_xlsxwriter::{Format, Formula, Workbook as XWorkbook};
    let werr = |e: rust_xlsxwriter::XlsxError| Error::Write(e.to_string());
    let mut x = XWorkbook::new();
    let date_fmt = Format::new().set_num_format("yyyy-mm-dd");
    let datetime_fmt = Format::new().set_num_format("yyyy-mm-dd hh:mm:ss");
    let mut used = Vec::new();
    for sh in &wb.sheets {
        let ws = x.add_worksheet_with_low_memory().map_err(werr)?;
        ws.set_name(safe_sheet_name(&sh.name, &mut used)).map_err(werr)?;
        for (c, w) in sh.col_widths.iter().enumerate() {
            if let Some(px) = w {
                ws.set_column_width(c as u16, ((*px as f64 - 5.0) / 7.0).max(0.5)).map_err(werr)?;
            }
        }
        for (r, row) in sh.rows.iter().enumerate() {
            let r = r as u32;
            for (c, cell) in row.iter().enumerate() {
                let c = c as u16;
                match cell {
                    Cell::Empty => {}
                    Cell::Value(v) => match v {
                        Value::Empty => {}
                        Value::Number(n) => {
                            ws.write_number(r, c, *n).map_err(werr)?;
                        }
                        Value::Date(n) => {
                            let f = if n.fract() == 0.0 { &date_fmt } else { &datetime_fmt };
                            ws.write_number_with_format(r, c, *n, f).map_err(werr)?;
                        }
                        Value::Text(t) => {
                            ws.write_string(r, c, t.as_ref()).map_err(werr)?;
                        }
                        Value::Bool(b) => {
                            ws.write_boolean(r, c, *b).map_err(werr)?;
                        }
                        Value::Error(e) => {
                            let src = if *e == CellError::NA { "=NA()".to_string() } else { format!("=\"{}\"", e.as_str()) };
                            ws.write_formula(r, c, Formula::new(src).set_result(e.as_str())).map_err(werr)?;
                        }
                    },
                    Cell::Formula(f) => {
                        let result = match &f.value {
                            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
                            Value::Number(n) | Value::Date(n) => format!("{}", n),
                            v => v.display(),
                        };
                        let formula = Formula::new(format!("={}", f.src)).set_result(result);
                        if let Value::Date(n) = f.value {
                            let fm = if n.fract() == 0.0 { &date_fmt } else { &datetime_fmt };
                            ws.write_formula_with_format(r, c, formula, fm).map_err(werr)?;
                        } else {
                            ws.write_formula(r, c, formula).map_err(werr)?;
                        }
                    }
                }
            }
        }
    }
    x.save(tmp).map_err(werr)?;
    Ok(())
}
