//! Read-side helpers used by front-ends: viewport blocks, clipboard, stats, printing.

use serde::Serialize;

use crate::formula::parser::shift_formula;
use crate::layout::CfTest;
use crate::sheet::{col_name, Cell, Sheet};
use crate::style::CellStyle;
use crate::value::Value;
use crate::workbook::Workbook;

/// A rectangle of displayed values for the grid.
#[derive(Serialize, Debug)]
pub struct Block {
    /// Data row number for each view row (differs from the view row when filtered).
    pub rows: Vec<u32>,
    pub cols: u32,
    /// Row-major display strings (number formats applied).
    pub text: Vec<String>,
    /// Row-major kind codes: 0 empty, 1 number, 2 text, 3 bool, 4 error; +8 when the cell is a formula.
    pub kinds: Vec<u8>,
    /// Row-major style ids (index into the workbook's styles).
    pub styles: Vec<u16>,
    /// Row-major conditional format: 0 none, else dxf index + 1. Omitted when the sheet has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cf: Option<Vec<u16>>,
}

/// Display text of a value in a given style.
pub fn display_styled(v: &Value, style: Option<&CellStyle>) -> String {
    match style.and_then(|s| s.number_format()) {
        Some(code) if !v.is_empty() => crate::numfmt::format_value(v, code),
        _ => v.display(),
    }
}

pub fn block(wb: &Workbook, sheet: usize, vr0: usize, vr1: usize, c0: usize, c1: usize) -> Block {
    let sh = &wb.sheets[sheet];
    let cols = c1.saturating_sub(c0) + 1;
    let mut out =
        Block { rows: Vec::new(), cols: cols as u32, text: Vec::new(), kinds: Vec::new(), styles: Vec::new(), cf: None };
    let rules = CompiledCf::new(wb, sheet, vr0, vr1, c0, c1);
    let mut cf = Vec::new();
    let (nrows, _) = sh.extent();
    let last = if sh.view.is_some() { vr1 } else { vr1.min(nrows.max(vr0 + 1) - 1) };
    for vr in vr0..=last {
        let Some(r) = sh.data_row(vr) else { break };
        out.rows.push(r as u32);
        let row = sh.rows.get(r);
        let srow = sh.styles.get(r);
        for c in c0..c0 + cols {
            let cell = row.and_then(|x| x.get(c)).unwrap_or(&Cell::Empty);
            let sid = srow.and_then(|x| x.get(c)).copied().unwrap_or(0);
            let v = cell.value();
            out.text.push(display_styled(v, wb.styles.get(sid as usize)));
            out.kinds.push(v.kind() | if matches!(cell, Cell::Formula(_)) { 8 } else { 0 });
            out.styles.push(sid);
            if !rules.is_empty() {
                cf.push(rules.eval(r as u32, c as u32, v));
            }
        }
    }
    if !rules.is_empty() {
        out.cf = Some(cf);
    }
    out
}

enum Operand {
    Num(f64),
    Text(String),
}

struct Rule<'a> {
    areas: &'a [crate::layout::Area],
    test: &'a CfTest,
    a: Option<Operand>,
    b: Option<Operand>,
    dxf: u16,
    stop: bool,
}

/// Conditional format rules that touch a block, with constant operands evaluated once.
struct CompiledCf<'a> {
    rules: Vec<Rule<'a>>,
}

impl<'a> CompiledCf<'a> {
    fn new(wb: &'a Workbook, sheet: usize, vr0: usize, vr1: usize, c0: usize, c1: usize) -> CompiledCf<'a> {
        let sh = &wb.sheets[sheet];
        let mut rules = Vec::new();
        if sh.cond.is_empty() {
            return CompiledCf { rules };
        }
        // Data rows of the block (the view may reorder them, so use the bounding box).
        let (mut lo, mut hi) = (u32::MAX, 0u32);
        for vr in vr0..=vr1 {
            if let Some(r) = sh.data_row(vr) {
                lo = lo.min(r as u32);
                hi = hi.max(r as u32);
            } else {
                break;
            }
        }
        let operand = |src: &str| -> Option<Operand> {
            let src = src.trim();
            if let Some(t) = src.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
                return Some(Operand::Text(t.replace("\"\"", "\"")));
            }
            if let Ok(n) = src.parse::<f64>() {
                return Some(Operand::Num(n));
            }
            match wb.evaluate(sheet, src) {
                Value::Number(n) | Value::Date(n) => Some(Operand::Num(n)),
                Value::Text(t) => Some(Operand::Text(t.to_string())),
                _ => None,
            }
        };
        for cfm in &sh.cond {
            let touches = cfm.areas.iter().any(|a| a.r1 >= lo && a.r0 <= hi && a.c1 as usize >= c0 && a.c0 as usize <= c1);
            if !touches {
                continue;
            }
            for rule in &cfm.rules {
                let (a, b) = match &rule.test {
                    CfTest::Cell { a, b, .. } => (operand(a), b.as_deref().and_then(operand)),
                    _ => (None, None),
                };
                if matches!(rule.test, CfTest::Cell { .. }) && a.is_none() {
                    continue;
                }
                rules.push(Rule { areas: &cfm.areas, test: &rule.test, a, b, dxf: rule.dxf as u16 + 1, stop: rule.stop });
            }
        }
        CompiledCf { rules }
    }

    fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    fn eval(&self, r: u32, c: u32, v: &Value) -> u16 {
        for rule in &self.rules {
            if !rule.areas.iter().any(|a| a.contains(r, c)) {
                continue;
            }
            if rule_matches(rule, v) {
                return rule.dxf;
            }
            if rule.stop {
                return 0;
            }
        }
        0
    }
}

fn rule_matches(rule: &Rule, v: &Value) -> bool {
    use std::cmp::Ordering::*;
    let text = || v.display().to_lowercase();
    match rule.test {
        CfTest::Blank => v.is_empty() || matches!(v, Value::Text(t) if t.trim().is_empty()),
        CfTest::NotBlank => !(v.is_empty() || matches!(v, Value::Text(t) if t.trim().is_empty())),
        CfTest::ContainsText { text: t } => text().contains(&t.to_lowercase()),
        CfTest::NotContainsText { text: t } => !text().contains(&t.to_lowercase()),
        CfTest::BeginsWith { text: t } => text().starts_with(&t.to_lowercase()),
        CfTest::EndsWith { text: t } => text().ends_with(&t.to_lowercase()),
        CfTest::Cell { op, .. } => {
            let cmp = |o: &Option<Operand>| -> Option<std::cmp::Ordering> {
                match (o.as_ref()?, v) {
                    (Operand::Num(n), Value::Number(x) | Value::Date(x)) => x.partial_cmp(n),
                    (Operand::Num(n), Value::Empty) => 0f64.partial_cmp(n),
                    (Operand::Text(t), Value::Text(x)) => Some(crate::value::compare_text(x, t)),
                    (Operand::Text(t), Value::Empty) => Some(crate::value::compare_text("", t)),
                    // Text sorts after numbers in Excel comparisons.
                    (Operand::Num(_), Value::Text(_)) => Some(Greater),
                    (Operand::Text(_), Value::Number(_) | Value::Date(_)) => Some(Less),
                    _ => None,
                }
            };
            let Some(a) = cmp(&rule.a) else { return false };
            match op.as_str() {
                "eq" => a == Equal,
                "ne" => a != Equal,
                "gt" => a == Greater,
                "ge" => a != Less,
                "lt" => a == Less,
                "le" => a != Greater,
                "between" | "notBetween" => {
                    let Some(b) = cmp(&rule.b) else { return false };
                    // Excel accepts the bounds in either order.
                    let inside = (a != Less && b != Greater) || (a != Greater && b != Less);
                    inside == (op == "between")
                }
                _ => false,
            }
        }
    }
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

pub fn copy(wb: &Workbook, sheet: usize, vr0: usize, c0: usize, vr1: usize, c1: usize) -> Copied {
    let styles = &wb.styles;
    let sheet = &wb.sheets[sheet];
    let mut tsv = String::new();
    let mut html = String::from("<table>");
    let mut inputs = Vec::new();
    for vr in vr0..=vr1 {
        let r = sheet.data_row(vr);
        let mut row_inputs = Vec::new();
        html.push_str("<tr>");
        for c in c0..=c1 {
            let cell = r.map(|r| sheet.cell(r, c)).unwrap_or(&Cell::Empty);
            let style = r.and_then(|r| styles.get(sheet.style_id(r, c) as usize));
            let shown = display_styled(cell.value(), style);
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
            let style = wb.styles.get(sh.style_id(r, c) as usize);
            let cls = match v {
                Value::Number(_) | Value::Date(_) => " class=\"n\"",
                _ => "",
            };
            let css = style.map(print_css).unwrap_or_default();
            let attr = if css.is_empty() { String::new() } else { format!(" style=\"{}\"", css) };
            html.push_str(&format!("<td{}{}>{}</td>", cls, attr, escape_html(&display_styled(v, style))));
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table>");
    (html, n, n < total)
}

/// Inline CSS for a style in print output.
fn print_css(s: &CellStyle) -> String {
    use crate::style::HAlign;
    let mut css = String::new();
    if s.bold {
        css.push_str("font-weight:bold;");
    }
    if s.italic {
        css.push_str("font-style:italic;");
    }
    if s.underline {
        css.push_str("text-decoration:underline;");
    }
    if let Some(c) = s.color {
        css.push_str(&format!("color:#{:06x};", c));
    }
    if let Some(c) = s.fill {
        css.push_str(&format!("background:#{:06x};print-color-adjust:exact;-webkit-print-color-adjust:exact;", c));
    }
    match s.h_align {
        HAlign::Center | HAlign::CenterAcross => css.push_str("text-align:center;"),
        HAlign::Right => css.push_str("text-align:right;"),
        HAlign::Left => css.push_str("text-align:left;"),
        _ => {}
    }
    if s.wrap {
        css.push_str("white-space:pre-wrap;");
    }
    css
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
