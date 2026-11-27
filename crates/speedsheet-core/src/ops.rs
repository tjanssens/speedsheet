//! Read-side helpers used by front-ends: viewport blocks, clipboard, stats, printing.

use serde::Serialize;

use crate::formula::parser::shift_formula;
use crate::sheet::{col_name, Cell, Sheet};
use crate::value::Value;
use crate::workbook::Workbook;

/// A rectangle of displayed values for the grid.
#[derive(Serialize, Debug)]
pub struct Block {
    /// Data row number for each view row (differs from the view row when filtered).
    pub rows: Vec<u32>,
    pub cols: u32,
    /// Row-major display strings.
    pub text: Vec<String>,
    /// Row-major kind codes: 0 empty, 1 number, 2 text, 3 bool, 4 error; +8 when the cell is a formula.
    pub kinds: Vec<u8>,
}

pub fn block(sheet: &Sheet, vr0: usize, vr1: usize, c0: usize, c1: usize) -> Block {
    let cols = c1.saturating_sub(c0) + 1;
    let mut out = Block { rows: Vec::new(), cols: cols as u32, text: Vec::new(), kinds: Vec::new() };
    for vr in vr0..=vr1 {
        let Some(r) = sheet.data_row(vr) else { break };
        out.rows.push(r as u32);
        let row = sheet.rows.get(r);
        for c in c0..c0 + cols {
            let cell = row.and_then(|x| x.get(c)).unwrap_or(&Cell::Empty);
            let v = cell.value();
            out.text.push(v.display());
            out.kinds.push(v.kind() | if matches!(cell, Cell::Formula(_)) { 8 } else { 0 });
        }
    }
    out
}

/// Heuristic: the first row is a header if all its filled cells are text and there is more data
/// below. When unsure we assume a header, which is what most real-world sheets have; the UI
/// lets the user override it.
pub fn guess_header(sheet: &Sheet) -> bool {
    let Some(first) = sheet.rows.first() else { return false };
    let mut filled = first.iter().map(|c| c.value()).filter(|v| !v.is_empty()).peekable();
    filled.peek().is_some() && filled.all(|v| matches!(v, Value::Text(_))) && sheet.row_count() > 1
}

#[derive(Serialize, Debug, Default)]
pub struct Stats {
    pub count: usize,
    pub numbers: usize,
    pub sum: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

pub fn stats(sheet: &Sheet, vr0: usize, c0: usize, vr1: usize, c1: usize) -> Stats {
    let mut s = Stats::default();
    let end = (vr1 + 1).min(sheet.view_len());
    for vr in vr0..end {
        let Some(r) = sheet.data_row(vr) else { break };
        let Some(row) = sheet.rows.get(r) else { continue };
        for cell in row.iter().take(c1 + 1).skip(c0) {
            let v = cell.value();
            if v.is_empty() {
                continue;
            }
            s.count += 1;
            if let Value::Number(n) | Value::Date(n) = v {
                s.numbers += 1;
                s.sum += n;
                s.min = Some(s.min.map_or(*n, |m| m.min(*n)));
                s.max = Some(s.max.map_or(*n, |m| m.max(*n)));
            }
        }
    }
    s
}

#[derive(Serialize, Debug)]
pub struct Copied {
    /// Tab separated displayed values (what other apps receive).
    pub tsv: String,
    /// HTML table, which Excel/Sheets/Word paste with cell structure.
    pub html: String,
    /// What the user typed, per cell, so formulas survive a copy inside SpeedSheet.
    pub inputs: Vec<Vec<String>>,
}

pub fn copy(sheet: &Sheet, vr0: usize, c0: usize, vr1: usize, c1: usize) -> Copied {
    let mut tsv = String::new();
    let mut html = String::from("<table>");
    let mut inputs = Vec::new();
    for vr in vr0..=vr1 {
        let r = sheet.data_row(vr);
        let mut row_inputs = Vec::new();
        html.push_str("<tr>");
        for c in c0..=c1 {
            let cell = r.map(|r| sheet.cell(r, c)).unwrap_or(&Cell::Empty);
            let shown = cell.value().display();
            if c > c0 {
                tsv.push('\t');
            }
            if shown.contains(['\t', '\n', '"']) {
                tsv.push('"');
                tsv.push_str(&shown.replace('"', "\"\""));
                tsv.push('"');
            } else {
                tsv.push_str(&shown);
            }
            html.push_str("<td>");
            html.push_str(&escape_html(&shown));
            html.push_str("</td>");
            row_inputs.push(cell.input_text());
        }
        html.push_str("</tr>");
        tsv.push('\n');
        inputs.push(row_inputs);
        if r.is_none() {
            break;
        }
    }
    html.push_str("</table>");
    Copied { tsv, html, inputs }
}

/// Adjust copied formulas for a paste at a new position (Excel-style relative references).
pub fn shift_inputs(inputs: &[Vec<String>], dr: i64, dc: i64) -> Vec<Vec<String>> {
    inputs
        .iter()
        .map(|row| {
            row.iter()
                .map(|s| match s.strip_prefix('=') {
                    Some(f) if s.len() > 1 => format!("={}", shift_formula(f, dr, dc)),
                    _ => s.clone(),
                })
                .collect()
        })
        .collect()
}

/// Parse clipboard text from other apps: TSV with Excel-style quoting.
pub fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_suffix("\r\n").or_else(|| text.strip_suffix('\n')).unwrap_or(text);
    let mut rdr =
        csv::ReaderBuilder::new().delimiter(b'\t').has_headers(false).flexible(true).quoting(true).from_reader(text.as_bytes());
    let mut out = Vec::new();
    for rec in rdr.records() {
        match rec {
            Ok(r) => out.push(r.iter().map(|s| s.to_string()).collect()),
            Err(_) => return text.lines().map(|l| l.split('\t').map(|s| s.to_string()).collect()).collect(),
        }
    }
    out
}

pub fn escape_html(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '&' => o.push_str("&amp;"),
            '"' => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

/// A printable HTML table of a range. Returns (html, rows included, truncated).
pub fn print_html(
    wb: &Workbook,
    sheet: usize,
    vr0: usize,
    c0: usize,
    vr1: usize,
    c1: usize,
    max_rows: usize,
) -> (String, usize, bool) {
    let sh = &wb.sheets[sheet];
    let vr1 = vr1.min(sh.view_len().saturating_sub(1));
    let c1 = c1.min(sh.col_count().saturating_sub(1).max(c0));
    let total = vr1.saturating_sub(vr0) + 1;
    let n = total.min(max_rows);
    let mut html = format!("<h1>{}</h1><table><thead><tr><th></th>", escape_html(&sh.name));
    for c in c0..=c1 {
        html.push_str(&format!("<th>{}</th>", col_name(c)));
    }
    html.push_str("</tr></thead><tbody>");
    for vr in vr0..vr0 + n {
        let Some(r) = sh.data_row(vr) else { break };
        html.push_str(&format!("<tr><th>{}</th>", r + 1));
        for c in c0..=c1 {
            let v = sh.value(r, c);
            let cls = match v {
                Value::Number(_) | Value::Date(_) => " class=\"n\"",
                _ => "",
            };
            html.push_str(&format!("<td{}>{}</td>", cls, escape_html(&v.display())));
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table>");
    (html, n, n < total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_round_trip() {
        let rows = parse_tsv("a\tb\n\"multi\nline\"\tc\n");
        assert_eq!(rows, vec![vec!["a", "b"], vec!["multi\nline", "c"]]);
        let shifted = shift_inputs(&[vec!["=A1+$B$1".into(), "x".into()]], 2, 1);
        assert_eq!(shifted[0], vec!["=B3+$B$1", "x"]);
    }
}
