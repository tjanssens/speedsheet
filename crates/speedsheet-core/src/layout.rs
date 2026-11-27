//! Sheet layout read straight from the xlsx XML: what calamine does not expose.
//!
//! One fast pass over each worksheet collects per-cell style ids, column widths, row heights,
//! hidden rows/columns, merged ranges, frozen panes, the autofilter range and conditional
//! formats. It runs on its own thread while calamine reads the values, so it adds little to
//! the load time.

use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Seek};
use std::sync::Arc;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::Serialize;

use crate::style::CellStyle;

/// An inclusive cell rectangle (data rows/cols).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Default)]
pub struct Area {
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

impl Area {
    pub fn contains(&self, r: u32, c: u32) -> bool {
        r >= self.r0 && r <= self.r1 && c >= self.c0 && c <= self.c1
    }
}

/// When a conditional format applies.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CfTest {
    /// `cellIs`: op is one of eq, ne, gt, ge, lt, le, between, notBetween. Operands are formula
    /// sources; only constants are evaluated.
    Cell {
        op: String,
        a: String,
        b: Option<String>,
    },
    ContainsText {
        text: String,
    },
    NotContainsText {
        text: String,
    },
    BeginsWith {
        text: String,
    },
    EndsWith {
        text: String,
    },
    Blank,
    NotBlank,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CfRule {
    pub test: CfTest,
    /// Index into `Workbook::dxfs`.
    pub dxf: u32,
    pub stop: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CondFormat {
    pub areas: Vec<Area>,
    pub rules: Vec<CfRule>,
}

#[derive(Default, Debug)]
pub(crate) struct SheetLayout {
    /// (row, col, style id) for cells with a non-default style.
    pub styles: Vec<(u32, u32, u16)>,
    /// (first col, last col, width px or None, hidden)
    pub cols: Vec<(u32, u32, Option<f32>, bool)>,
    /// (row, height px if custom, hidden)
    pub rows: Vec<(u32, Option<f32>, bool)>,
    pub default_col_width: Option<f32>,
    pub default_row_height: Option<f32>,
    pub merges: Vec<Area>,
    pub freeze: Option<(u32, u32)>,
    pub autofilter: Option<Area>,
    pub cond: Vec<CondFormat>,
    pub show_grid: bool,
    /// VBA code name (`<sheetPr codeName>`).
    pub codename: Option<String>,
    /// Features in this sheet that SpeedSheet does not keep (see [`BookLayout::losses`]).
    pub losses: BTreeSet<&'static str>,
}

/// A `<definedName>` from workbook.xml.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawName {
    pub name: String,
    /// Name of the sheet the name is scoped to (from `localSheetId`), None = workbook-wide.
    pub sheet: Option<String>,
    pub formula: String,
    pub hidden: bool,
}

#[derive(Default, Debug)]
pub(crate) struct BookLayout {
    pub sheets: HashMap<String, SheetLayout>,
    /// Differential formats used by conditional formatting.
    pub dxfs: Vec<CellStyle>,
    pub names: Vec<RawName>,
    /// `xl/vbaProject.bin` and `xl/vbaProjectSignature.bin`.
    pub vba: Option<Vec<u8>>,
    pub vba_signature: Option<Vec<u8>>,
    /// The workbook's VBA code name (`<workbookPr codeName>`).
    pub codename: Option<String>,
    /// Features of the file that SpeedSheet cannot keep: writing the workbook back would
    /// remove them. Human-readable, for the warning before saving.
    pub losses: BTreeSet<&'static str>,
}

/// Reported when the package could not be inspected: saving over it is then never silent.
pub(crate) const UNCHECKED: &str = "Content SpeedSheet could not check";

/// Parts of an xlsx package SpeedSheet does not write back, by path prefix (lowercase).
const UNKEPT_PARTS: &[(&str, &str)] = &[
    ("xl/charts/", "Charts"),
    ("xl/chartsheets/", "Chart sheets"),
    ("xl/media/", "Images"),
    ("xl/pivottables/", "Pivot tables"),
    ("xl/pivotcache/", "Pivot tables"),
    ("xl/comments", "Notes (comments)"),
    ("xl/threadedcomments/", "Threaded comments"),
    ("xl/tables/", "Excel tables (Format as Table)"),
    ("xl/slicers/", "Slicers"),
    ("xl/slicercaches/", "Slicers"),
    ("xl/timelines/", "Timelines"),
    ("xl/timelinecaches/", "Timelines"),
    ("xl/externallinks/", "Links to other workbooks"),
    ("xl/connections.xml", "Data connections"),
    ("xl/querytables/", "Data connections"),
    ("xl/model/", "Data model (Power Pivot)"),
    ("xl/activex/", "Form and ActiveX controls"),
    ("xl/ctrlprops/", "Form and ActiveX controls"),
    ("xl/embeddings/", "Embedded objects"),
    ("xl/richdata/", "Pictures in cells and data types"),
    ("xl/macrosheets/", "Excel 4 macro sheets"),
    ("xl/dialogsheets/", "Dialog sheets"),
    ("customui/", "Custom ribbon"),
    ("xl/customui", "Custom ribbon"),
];

/// Decode XML text with its entity references (quick-xml reports `&amp;` and friends apart).
fn push_entity(out: &mut String, name: &str) {
    match name {
        "lt" => out.push('<'),
        "gt" => out.push('>'),
        "amp" => out.push('&'),
        "quot" => out.push('"'),
        "apos" => out.push('\''),
        n => {
            let code = n
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()));
            if let Some(c) = code.and_then(char::from_u32) {
                out.push(c);
            }
        }
    }
}

/// Is there a rich-text run (`<r>`) in the shared strings? Read in chunks, stops at the first.
fn has_rich_text<R: Read>(mut r: R) -> bool {
    let mut buf = vec![0u8; 1 << 16];
    let mut carry = 0usize;
    loop {
        let n = match r.read(&mut buf[carry..]) {
            Ok(0) | Err(_) => return false,
            Ok(n) => n,
        };
        let data = &buf[..carry + n];
        if data.windows(3).any(|w| w == b"<r>") || data.windows(5).any(|w| w == b"<x:r>") {
            return true;
        }
        // Keep the tail so a tag split over two chunks is still seen.
        let keep = data.len().min(4);
        let start = data.len() - keep;
        buf.copy_within(start..start + keep, 0);
        carry = keep;
    }
}

/// Parse "AB12" into (row, col), 0-based.
pub(crate) fn parse_ref(s: &str) -> Option<(u32, u32)> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut col: u32 = 0;
    while i < b.len() && (b[i] == b'$' || b[i].is_ascii_alphabetic()) {
        if b[i] != b'$' {
            col = col.checked_mul(26)?.checked_add((b[i].to_ascii_uppercase() - b'A') as u32 + 1)?;
        }
        i += 1;
    }
    let mut row: u32 = 0;
    let mut digits = 0;
    while i < b.len() && (b[i] == b'$' || b[i].is_ascii_digit()) {
        if b[i] != b'$' {
            row = row.checked_mul(10)?.checked_add((b[i] - b'0') as u32)?;
            digits += 1;
        }
        i += 1;
    }
    if col == 0 || digits == 0 || row == 0 || i != b.len() {
        return None;
    }
    Some((row - 1, col - 1))
}

pub(crate) fn parse_area(s: &str) -> Option<Area> {
    let (a, b) = s.split_once(':').unwrap_or((s, s));
    let (r0, c0) = parse_ref(a)?;
    let (r1, c1) = parse_ref(b)?;
    Some(Area { r0: r0.min(r1), c0: c0.min(c1), r1: r0.max(r1), c1: c0.max(c1) })
}

fn attr<'a>(e: &'a BytesStart, name: &str) -> Option<std::borrow::Cow<'a, str>> {
    for a in e.attributes().with_checks(false).flatten() {
        if a.key.local_name().as_ref() == name {
            let v = a
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(|v| v.into_owned())
                .unwrap_or_else(|_| a.value.into_owned());
            return Some(std::borrow::Cow::Owned(v));
        }
    }
    None
}

fn attr_f32(e: &BytesStart, name: &str) -> Option<f32> {
    attr(e, name).and_then(|v| v.trim().parse().ok())
}

fn attr_true(e: &BytesStart, name: &str) -> bool {
    attr(e, name).is_some_and(|v| v == "1" || v == "true")
}

/// A protection element that actually locks something: one of `flags` set, or a password.
fn is_locked(e: &BytesStart, flags: &[&str]) -> bool {
    flags.iter().any(|f| attr_true(e, f))
        || e.attributes().with_checks(false).flatten().any(|a| {
            let k = a.key.local_name();
            let k = k.as_ref().to_ascii_lowercase();
            k.contains("password") || k == "hashvalue"
        })
}

fn xml<R: BufRead>(r: R) -> Reader<R> {
    let mut x = Reader::from_reader(r);
    let c = x.config_mut();
    c.check_end_names = false;
    c.trim_text(false);
    c.check_comments = false;
    x
}

fn read_entry<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, path: &str) -> Option<String> {
    let mut f = zip.by_name(path).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

/// Column width in Excel's character units (with padding) to pixels, for the default font.
pub(crate) fn width_to_px(w: f32) -> f32 {
    (w * 7.0).round()
}

/// Row height in points to pixels.
pub(crate) fn pt_to_px(pt: f32) -> f32 {
    (pt * 4.0 / 3.0).round()
}

/// Read the layout of every worksheet in an xlsx file. Errors yield an empty layout: the
/// values are what matters, the layout is a bonus.
pub(crate) fn read_book_layout<R: Read + Seek>(reader: R, palette_len: usize) -> BookLayout {
    let mut out = BookLayout::default();
    let Ok(mut zip) = zip::ZipArchive::new(reader) else {
        out.losses.insert(UNCHECKED);
        return out;
    };
    let names: HashMap<String, String> = zip.file_names().flatten().map(|n| (n.to_ascii_lowercase(), n.to_string())).collect();
    let find = |p: &str| -> Option<String> { names.get(&p.to_ascii_lowercase()).cloned() };

    // What the package holds that is never written back.
    let lower: Vec<&String> = names.keys().collect();
    for (prefix, what) in UNKEPT_PARTS {
        if lower.iter().any(|n| n.starts_with(prefix)) {
            out.losses.insert(what);
        }
    }
    let has = |prefix: &str| lower.iter().any(|n| n.starts_with(prefix));
    if has("xl/drawings/drawing") && !has("xl/charts/") && !has("xl/media/") {
        out.losses.insert("Shapes and text boxes");
    }
    if let Some(p) = find("xl/vbaProject.bin") {
        if let Ok(mut f) = zip.by_name(&p) {
            let mut v = Vec::new();
            if f.read_to_end(&mut v).is_ok() {
                out.vba = Some(v);
            }
        }
    }
    if let Some(p) = find("xl/vbaProjectSignature.bin") {
        if let Ok(mut f) = zip.by_name(&p) {
            let mut v = Vec::new();
            if f.read_to_end(&mut v).is_ok() {
                out.vba_signature = Some(v);
            }
        }
    }
    if let Some(p) = find("xl/sharedStrings.xml") {
        if let Ok(f) = zip.by_name(&p) {
            if has_rich_text(f) {
                out.losses.insert("Text formatting within cells (rich text)");
            }
        }
    }

    // Sheet name -> r:id, r:id -> target.
    let mut sheets: Vec<(String, String)> = Vec::new();
    if let Some(wb) = find("xl/workbook.xml").and_then(|p| read_entry(&mut zip, &p)) {
        let mut x = xml(wb.as_bytes());
        let mut buf = Vec::new();
        // <definedName> being read: (name, localSheetId, hidden, text).
        let mut name: Option<(String, Option<usize>, bool, String)> = None;
        let mut raw: Vec<(String, Option<usize>, bool, String)> = Vec::new();
        loop {
            match x.read_event_into(&mut buf) {
                Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == "sheet" => {
                    if let (Some(n), Some(id)) = (attr(&e, "name"), attr(&e, "id")) {
                        sheets.push((n.into_owned(), id.into_owned()));
                    }
                    if attr(&e, "state").is_some_and(|v| v != "visible") {
                        out.losses.insert("Hidden sheets (they will show)");
                    }
                }
                Ok(Event::Start(e)) if e.local_name().as_ref() == "definedName" => {
                    let n = attr(&e, "name").map(|v| v.into_owned()).unwrap_or_default();
                    let local = attr(&e, "localSheetId").and_then(|v| v.parse().ok());
                    name = Some((n, local, attr_true(&e, "hidden"), String::new()));
                }
                Ok(Event::Text(t)) => {
                    if let Some(n) = name.as_mut() {
                        n.3.push_str(&t);
                    }
                }
                Ok(Event::GeneralRef(g)) => {
                    if let Some(n) = name.as_mut() {
                        push_entity(&mut n.3, g.as_ref());
                    }
                }
                Ok(Event::End(e)) if e.local_name().as_ref() == "definedName" => raw.extend(name.take()),
                Ok(Event::Start(e) | Event::Empty(e)) => match e.local_name().as_ref() {
                    "workbookPr" => out.codename = attr(&e, "codeName").map(|v| v.into_owned()),
                    // Some writers emit an empty element: only a real lock counts.
                    "workbookProtection" if is_locked(&e, &["lockStructure", "lockWindows"]) => {
                        out.losses.insert("Workbook protection");
                    }
                    _ => {}
                },
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
        for (n, local, hidden, formula) in raw {
            let sheet = match local {
                Some(i) => match sheets.get(i) {
                    Some(s) => Some(s.0.clone()),
                    None => continue,
                },
                None => None,
            };
            let upper = n.to_ascii_uppercase();
            if upper == "_XLNM._FILTERDATABASE" {
                continue; // written again from the autofilter
            }
            if upper == "_XLNM.PRINT_AREA" || upper == "_XLNM.PRINT_TITLES" {
                out.losses.insert("Print areas and print titles");
                continue;
            }
            if upper.starts_with("_XLNM.") {
                continue;
            }
            out.names.push(RawName { name: n, sheet, formula: formula.trim().to_string(), hidden });
        }
    }
    let mut targets: HashMap<String, String> = HashMap::new();
    if let Some(rels) = find("xl/_rels/workbook.xml.rels").and_then(|p| read_entry(&mut zip, &p)) {
        let mut x = xml(rels.as_bytes());
        let mut buf = Vec::new();
        loop {
            match x.read_event_into(&mut buf) {
                Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == "Relationship" => {
                    if let (Some(id), Some(t)) = (attr(&e, "Id"), attr(&e, "Target")) {
                        let t = if let Some(abs) = t.strip_prefix('/') { abs.to_string() } else { format!("xl/{}", t) };
                        targets.insert(id.into_owned(), t);
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
    }
    if let Some(styles) = find("xl/styles.xml").and_then(|p| read_entry(&mut zip, &p)) {
        out.dxfs = read_dxfs(&styles);
    }
    for (name, id) in sheets {
        let Some(path) = targets.get(&id).and_then(|t| find(t)) else { continue };
        let Ok(f) = zip.by_name(&path) else { continue };
        let layout = read_sheet_layout(BufReader::with_capacity(1 << 16, f), palette_len);
        out.sheets.insert(name, layout);
    }
    out
}

/// A `cfRule` being read: (type, operator, text, dxf id, stopIfTrue).
type PendingRule = (String, Option<String>, Option<String>, u32, bool);

fn read_sheet_layout<R: BufRead>(r: R, palette_len: usize) -> SheetLayout {
    let mut l = SheetLayout { show_grid: true, ..Default::default() };
    let mut in_header_footer = false;
    let mut x = xml(r);
    let mut buf = Vec::with_capacity(1024);
    let mut row: u32 = 0;
    let mut col: u32 = 0;
    // Conditional formatting state.
    let mut cf: Option<CondFormat> = None;
    let mut rule: Option<PendingRule> = None;
    let mut formulas: Vec<String> = Vec::new();
    let mut in_formula = false;
    loop {
        let ev = x.read_event_into(&mut buf);
        let (e, empty) = match ev {
            Ok(Event::Start(e)) => (e, false),
            Ok(Event::Empty(e)) => (e, true),
            Ok(Event::Text(t)) if in_formula => {
                if let Some(f) = formulas.last_mut() {
                    f.push_str(&t);
                }
                buf.clear();
                continue;
            }
            Ok(Event::GeneralRef(g)) if in_formula => {
                if let Some(f) = formulas.last_mut() {
                    let s = match g.as_ref() {
                        "lt" => "<",
                        "gt" => ">",
                        "amp" => "&",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    };
                    f.push_str(s);
                }
                buf.clear();
                continue;
            }
            Ok(Event::End(e)) => {
                match e.local_name().as_ref() {
                    "row" => {
                        row += 1;
                        col = 0;
                    }
                    "formula" => in_formula = false,
                    "headerFooter" => in_header_footer = false,
                    "cfRule" => {
                        if let (Some(c), Some((kind, op, text, dxf, stop))) = (cf.as_mut(), rule.take()) {
                            match make_test(&kind, op.as_deref(), text, std::mem::take(&mut formulas)) {
                                Some(test) => c.rules.push(CfRule { test, dxf, stop }),
                                None => {
                                    l.losses.insert("Some conditional formatting rules");
                                }
                            }
                        }
                        formulas.clear();
                    }
                    "conditionalFormatting" => {
                        if let Some(c) = cf.take() {
                            if !c.rules.is_empty() && !c.areas.is_empty() {
                                l.cond.push(c);
                            }
                        }
                    }
                    _ => {}
                }
                buf.clear();
                continue;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {
                buf.clear();
                continue;
            }
        };
        match e.local_name().as_ref() {
            "c" => {
                // Attributes in their usual order: r, s, t.
                let mut s: u32 = 0;
                for a in e.attributes().with_checks(false).flatten() {
                    match a.key.as_ref() {
                        "r" => {
                            if let Some((rr, cc)) = parse_ref(&a.value) {
                                row = rr;
                                col = cc;
                            }
                        }
                        "s" => s = a.value.parse().unwrap_or(0),
                        _ => {}
                    }
                }
                if s > 0 && (s as usize) < palette_len && s <= u16::MAX as u32 {
                    l.styles.push((row, col, s as u16));
                }
                col += 1;
                if !empty {
                    // Skip the cell body quickly.
                    let mut depth = 1;
                    loop {
                        buf.clear();
                        match x.read_event_into(&mut buf) {
                            Ok(Event::Start(_)) => depth += 1,
                            Ok(Event::End(_)) => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Ok(Event::Eof) | Err(_) => return l,
                            _ => {}
                        }
                    }
                }
            }
            "row" => {
                if let Some(r) = attr(&e, "r").and_then(|v| v.parse::<u32>().ok()) {
                    row = r.saturating_sub(1);
                }
                col = 0;
                let hidden = attr_true(&e, "hidden");
                if attr(&e, "outlineLevel").is_some_and(|v| v != "0") {
                    l.losses.insert("Grouped rows or columns (outline)");
                }
                let ht = if attr_true(&e, "customHeight") { attr_f32(&e, "ht").map(pt_to_px) } else { None };
                if hidden || ht.is_some() {
                    l.rows.push((row, ht, hidden));
                }
                if empty {
                    row += 1;
                }
            }
            "col" => {
                let min = attr(&e, "min").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
                let max = attr(&e, "max").and_then(|v| v.parse::<u32>().ok()).unwrap_or(min).min(crate::MAX_COLS as u32);
                let w = attr_f32(&e, "width").map(width_to_px);
                let hidden = attr_true(&e, "hidden");
                if attr(&e, "outlineLevel").is_some_and(|v| v != "0") {
                    l.losses.insert("Grouped rows or columns (outline)");
                }
                if w.is_some() || hidden {
                    l.cols.push((min.saturating_sub(1), max.saturating_sub(1), w, hidden));
                }
            }
            "sheetFormatPr" => {
                if let Some(w) = attr_f32(&e, "defaultColWidth") {
                    l.default_col_width = Some(width_to_px(w));
                } else if let Some(b) = attr_f32(&e, "baseColWidth") {
                    // baseColWidth is in characters without padding.
                    l.default_col_width = Some(b * 7.0 + 8.0);
                }
                l.default_row_height = attr_f32(&e, "defaultRowHeight").map(pt_to_px);
            }
            "sheetView" => {
                if attr(&e, "showGridLines").is_some_and(|v| v == "0" || v == "false") {
                    l.show_grid = false;
                }
            }
            "pane" => {
                let state = attr(&e, "state");
                if state.as_deref().is_some_and(|s| s == "frozen" || s == "frozenSplit") {
                    let xs = attr_f32(&e, "xSplit").unwrap_or(0.0) as u32;
                    let ys = attr_f32(&e, "ySplit").unwrap_or(0.0) as u32;
                    if xs > 0 || ys > 0 {
                        l.freeze = Some((ys, xs));
                    }
                }
            }
            "mergeCell" => {
                if let Some(a) = attr(&e, "ref").and_then(|v| parse_area(&v)) {
                    l.merges.push(a);
                }
            }
            "autoFilter" => {
                if l.autofilter.is_none() {
                    l.autofilter = attr(&e, "ref").and_then(|v| parse_area(&v));
                }
                if !empty {
                    // Skip filter column criteria.
                    let mut depth = 1;
                    loop {
                        buf.clear();
                        match x.read_event_into(&mut buf) {
                            Ok(Event::Start(_)) => depth += 1,
                            Ok(Event::End(_)) => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Ok(Event::Eof) | Err(_) => return l,
                            _ => {}
                        }
                    }
                }
            }
            "conditionalFormatting" => {
                let areas = attr(&e, "sqref")
                    .map(|v| v.split_whitespace().filter_map(parse_area).collect::<Vec<_>>())
                    .unwrap_or_default();
                cf = Some(CondFormat { areas, rules: Vec::new() });
            }
            "cfRule" if cf.is_some() => {
                let kind = attr(&e, "type").map(|v| v.into_owned()).unwrap_or_default();
                let op = attr(&e, "operator").map(|v| v.into_owned());
                let text = attr(&e, "text").map(|v| v.into_owned());
                let dxf = attr(&e, "dxfId").and_then(|v| v.parse::<u32>().ok());
                let stop = attr_true(&e, "stopIfTrue");
                formulas.clear();
                rule = dxf.map(|d| (kind, op, text, d, stop));
                if rule.is_none() {
                    // Data bars, color scales and icon sets have no dxf.
                    l.losses.insert("Some conditional formatting rules");
                }
                if empty {
                    if let (Some(c), Some((kind, op, text, dxf, stop))) = (cf.as_mut(), rule.take()) {
                        match make_test(&kind, op.as_deref(), text, Vec::new()) {
                            Some(test) => c.rules.push(CfRule { test, dxf, stop }),
                            None => {
                                l.losses.insert("Some conditional formatting rules");
                            }
                        }
                    }
                }
            }
            "formula" if rule.is_some() && !empty => {
                formulas.push(String::new());
                in_formula = true;
            }
            "sheetPr" => l.codename = attr(&e, "codeName").map(|v| v.into_owned()),
            "dataValidation" => {
                l.losses.insert("Data validation (drop-down lists)");
            }
            "hyperlink" => {
                l.losses.insert("Hyperlinks");
            }
            "sheetProtection" if is_locked(&e, &["sheet"]) => {
                l.losses.insert("Sheet protection");
            }
            "sparklineGroup" => {
                l.losses.insert("Sparklines");
            }
            "brk" => {
                l.losses.insert("Page breaks");
            }
            "headerFooter" if !empty => in_header_footer = true,
            "oddHeader" | "oddFooter" | "evenHeader" | "evenFooter" | "firstHeader" | "firstFooter"
                if in_header_footer && !empty =>
            {
                l.losses.insert("Print headers and footers");
            }
            _ => {}
        }
        buf.clear();
    }
    l
}

fn make_test(kind: &str, op: Option<&str>, text: Option<String>, mut formulas: Vec<String>) -> Option<CfTest> {
    Some(match kind {
        "cellIs" => {
            let op = match op.unwrap_or("equal") {
                "equal" => "eq",
                "notEqual" => "ne",
                "greaterThan" => "gt",
                "greaterThanOrEqual" => "ge",
                "lessThan" => "lt",
                "lessThanOrEqual" => "le",
                "between" => "between",
                "notBetween" => "notBetween",
                _ => return None,
            };
            if formulas.is_empty() {
                return None;
            }
            let a = formulas.remove(0);
            let b = formulas.into_iter().next();
            CfTest::Cell { op: op.to_string(), a, b }
        }
        "containsText" => CfTest::ContainsText { text: text? },
        "notContainsText" => CfTest::NotContainsText { text: text? },
        "beginsWith" => CfTest::BeginsWith { text: text? },
        "endsWith" => CfTest::EndsWith { text: text? },
        "containsBlanks" => CfTest::Blank,
        "notContainsBlanks" => CfTest::NotBlank,
        _ => return None,
    })
}

/// Differential formats (`<dxfs>` in styles.xml): font bold/italic/color and fill.
/// Theme colors map to the default Office palette.
fn read_dxfs(styles: &str) -> Vec<CellStyle> {
    let mut out = Vec::new();
    let mut x = xml(styles.as_bytes());
    let mut buf = Vec::new();
    let mut in_dxfs = false;
    let mut cur: Option<CellStyle> = None;
    let mut ctx = "";
    loop {
        let ev = x.read_event_into(&mut buf);
        match ev {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let n = e.local_name();
                match n.as_ref() {
                    "dxfs" => in_dxfs = true,
                    "dxf" if in_dxfs => cur = Some(CellStyle::default()),
                    "font" if cur.is_some() => ctx = "font",
                    "fill" if cur.is_some() => ctx = "fill",
                    "border" if cur.is_some() => ctx = "border",
                    "b" if ctx == "font" => {
                        if let Some(c) = cur.as_mut() {
                            c.bold = attr(&e, "val").is_none_or(|v| v != "0" && v != "false");
                        }
                    }
                    "i" if ctx == "font" => {
                        if let Some(c) = cur.as_mut() {
                            c.italic = attr(&e, "val").is_none_or(|v| v != "0" && v != "false");
                        }
                    }
                    "u" if ctx == "font" => {
                        if let Some(c) = cur.as_mut() {
                            c.underline = true;
                        }
                    }
                    "strike" if ctx == "font" => {
                        if let Some(c) = cur.as_mut() {
                            c.strike = true;
                        }
                    }
                    "color" if ctx == "font" => {
                        if let (Some(c), Some(rgb)) = (cur.as_mut(), color_of(&e)) {
                            c.color = Some(rgb);
                        }
                    }
                    "bgColor" | "fgColor" if ctx == "fill" => {
                        if let (Some(c), Some(rgb)) = (cur.as_mut(), color_of(&e)) {
                            // In a dxf the bgColor is the cell color; take fgColor only if no bgColor.
                            if n.as_ref() == "bgColor" || c.fill.is_none() {
                                c.fill = Some(rgb);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                "dxfs" => break,
                "dxf" => {
                    if let Some(c) = cur.take() {
                        out.push(c);
                    }
                    ctx = "";
                }
                "font" | "fill" | "border" => ctx = "",
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Default Office theme colors by `theme` index (lt1, dk1, lt2, dk2, accent1..6, hlink, folHlink).
const THEME: [u32; 12] =
    [0xFFFFFF, 0x000000, 0xE7E6E6, 0x44546A, 0x4472C4, 0xED7D31, 0xA5A5A5, 0xFFC000, 0x5B9BD5, 0x70AD47, 0x0563C1, 0x954F72];

fn color_of(e: &BytesStart) -> Option<u32> {
    if let Some(rgb) = attr(e, "rgb") {
        let v = u32::from_str_radix(rgb.trim(), 16).ok()?;
        return Some(v & 0xFFFFFF);
    }
    if let Some(t) = attr(e, "theme").and_then(|v| v.parse::<usize>().ok()) {
        let base = *THEME.get(t)?;
        let tint = attr_f32(e, "tint").unwrap_or(0.0);
        return Some(apply_tint(base, tint));
    }
    if let Some(i) = attr(e, "indexed").and_then(|v| v.parse::<usize>().ok()) {
        const IDX: [u32; 8] = [0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF];
        return IDX.get(i % 8).copied();
    }
    None
}

fn apply_tint(rgb: u32, tint: f32) -> u32 {
    if tint == 0.0 {
        return rgb;
    }
    let ch = |shift: u32| {
        let v = ((rgb >> shift) & 0xFF) as f32;
        let v = if tint < 0.0 { v * (1.0 + tint) } else { v + (255.0 - v) * tint };
        (v.round().clamp(0.0, 255.0) as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

/// Shared, immutable conditional formats of a sheet.
pub type CondFormats = Arc<Vec<CondFormat>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs() {
        assert_eq!(parse_ref("A1"), Some((0, 0)));
        assert_eq!(parse_ref("$AB$12"), Some((11, 27)));
        assert_eq!(parse_ref("A0"), None);
        assert_eq!(parse_area("B2:A1"), Some(Area { r0: 0, c0: 0, r1: 1, c1: 1 }));
    }

    #[test]
    fn sheet_xml() {
        let xml = r#"<worksheet><sheetViews><sheetView><pane xSplit="1" ySplit="2" state="frozen"/></sheetView></sheetViews>
<sheetFormatPr defaultRowHeight="15"/><cols><col min="1" max="2" width="20.7109375" customWidth="1"/><col min="4" max="4" hidden="1" width="0"/></cols>
<sheetData><row r="1" ht="30" customHeight="1"><c r="A1" s="3" t="s"><v>0</v></c><c s="2"/></row><row r="3" hidden="1"><c r="C3"><v>1</v></c></row></sheetData>
<autoFilter ref="A2:E9"><filterColumn colId="0"/></autoFilter><mergeCells count="1"><mergeCell ref="A1:C1"/></mergeCells>
<conditionalFormatting sqref="B3:B9"><cfRule type="cellIs" dxfId="0" priority="1" operator="greaterThan"><formula>10</formula></cfRule><cfRule type="containsText" dxfId="1" priority="2" text="a&amp;b"><formula>NOT(ISERROR(SEARCH("a&amp;b",B3)))</formula></cfRule></conditionalFormatting>
</worksheet>"#;
        let l = read_sheet_layout(xml.as_bytes(), 10);
        assert_eq!(l.styles, vec![(0, 0, 3), (0, 1, 2)]);
        assert_eq!(l.cols, vec![(0, 1, Some(145.0), false), (3, 3, Some(0.0), true)]);
        assert_eq!(l.rows, vec![(0, Some(40.0), false), (2, None, true)]);
        assert_eq!(l.default_row_height, Some(20.0));
        assert_eq!(l.freeze, Some((2, 1)));
        assert_eq!(l.autofilter, Some(Area { r0: 1, c0: 0, r1: 8, c1: 4 }));
        assert_eq!(l.merges, vec![Area { r0: 0, c0: 0, r1: 0, c1: 2 }]);
        assert_eq!(l.cond.len(), 1);
        assert_eq!(l.cond[0].rules[0].test, CfTest::Cell { op: "gt".into(), a: "10".into(), b: None });
        assert_eq!(l.cond[0].rules[1].test, CfTest::ContainsText { text: "a&b".into() });
    }
}
