//! Tests against the file corpus in `testdata/corpus/` (rebuild with `scripts/refresh-corpus.sh`).
//!
//! - Oracle: LibreOffice calculated and cached every formula. SpeedSheet's recalculation of the
//!   same file must give the same values.
//! - Fidelity: open + save must never lose anything silently. The test lists every part of the
//!   zip and every top-level element of the workbook and sheet XML, before and after. Anything
//!   that disappears must be in the warning SpeedSheet shows (`Workbook::losses`).
//!
//! Known differences are listed below, each with its reason. The lists are a ratchet: a new
//! difference fails the test, and so does a listed one that has gone away, so the list can only
//! shrink. When fixing one, delete its line.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use speedsheet_core::{io, Cell, Value};

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/corpus").join(name)
}

/// (sheet, cell, reason). Cells whose recalculated value differs from LibreOffice's.
const KNOWN_MISMATCHES: &[(&str, &str, &str)] =
    &[("Calc", "B9", "found gap: array constants like {\"x\",\"y\";1,2} give #NAME?")];

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => (x - y).abs() <= 1e-9 * x.abs().max(1.0),
        // Some writers store TRUE/FALSE results as 1/0.
        (Value::Bool(x), Value::Number(y)) | (Value::Number(y), Value::Bool(x)) => (*x as u8 as f64) == *y,
        _ => a.display() == b.display(),
    }
}

#[test]
fn recalculation_matches_libreoffice() {
    let mut wb = io::open(&corpus("formulas.xlsx")).unwrap();
    let cached: Vec<(usize, usize, usize, Value)> = formula_cells(&wb);
    assert!(cached.len() > 200, "corpus has formulas ({})", cached.len());
    wb.recalc_everything();
    let mut mismatches: Vec<((String, String), String)> = Vec::new();
    for (s, r, c, want) in cached {
        let got = wb.sheets[s].value(r, c);
        if !same(got, &want) {
            let sheet = &wb.sheets[s].name;
            let cell = format!("{}{}", speedsheet_core::col_name(c), r + 1);
            let line = format!("{sheet}!{cell}: {} gives {got:?}, LibreOffice {want:?}", wb.sheets[s].cell(r, c).input_text());
            mismatches.push(((sheet.clone(), cell), line));
        }
    }
    let known: BTreeSet<(String, String)> = KNOWN_MISMATCHES.iter().map(|(s, c, _)| (s.to_string(), c.to_string())).collect();
    let new: Vec<&str> = mismatches.iter().filter(|(k, _)| !known.contains(k)).map(|(_, l)| l.as_str()).collect();
    let found: BTreeSet<(String, String)> = mismatches.iter().map(|(k, _)| k.clone()).collect();
    let fixed: Vec<_> = known.difference(&found).collect();
    assert!(new.is_empty(), "values differ from LibreOffice:\n{}", new.join("\n"));
    assert!(fixed.is_empty(), "these now match; remove them from KNOWN_MISMATCHES: {fixed:?}");
}

fn formula_cells(wb: &speedsheet_core::Workbook) -> Vec<(usize, usize, usize, Value)> {
    let mut out = Vec::new();
    for (si, s) in wb.sheets.iter().enumerate() {
        for (r, row) in s.rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                if let Cell::Formula(f) = cell {
                    out.push((si, r, c, f.value.clone()));
                }
            }
        }
    }
    out
}

/// What a file contains: its zip parts (numbers in names replaced by N, so sheet2 and sheet3
/// count the same), the top-level elements of the workbook and of each sheet, and the content
/// type of the workbook part (which says whether macros are enabled).
fn inventory(path: &Path) -> BTreeSet<String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut out = BTreeSet::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let kind: String = name.chars().map(|c| if c.is_ascii_digit() { 'N' } else { c }).collect::<String>();
        let kind = kind.replace("NN", "N");
        out.insert(format!("part {kind}"));
        let is_sheet = name.starts_with("xl/worksheets/") && name.ends_with(".xml");
        if name == "xl/workbook.xml" || is_sheet || name == "[Content_Types].xml" {
            let mut xml = String::new();
            f.read_to_string(&mut xml).unwrap();
            if name == "[Content_Types].xml" {
                if let Some(ct) = content_type_of(&xml, "/xl/workbook.xml") {
                    out.insert(format!("workbook content type {ct}"));
                }
            } else {
                for el in top_level_elements(&xml) {
                    out.insert(format!("{kind} <{el}>"));
                }
            }
        }
    }
    out
}

fn top_level_elements(xml: &str) -> BTreeSet<String> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut depth = 0;
    let mut out = BTreeSet::new();
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                if depth == 1 {
                    out.insert(e.local_name().as_ref().to_string());
                }
                depth += 1;
            }
            // An empty element without attributes (`<workbookProtection/>`) holds nothing to lose.
            Ok(Event::Empty(e)) if depth == 1 && e.attributes().next().is_some() => {
                out.insert(e.local_name().as_ref().to_string());
            }
            Ok(Event::End(_)) => depth -= 1,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn content_type_of(xml: &str, part: &str) -> Option<String> {
    let at = xml.find(&format!("PartName=\"{part}\""))?;
    let tag_start = xml[..at].rfind('<')?;
    let tag = &xml[tag_start..tag_start + xml[tag_start..].find('>')?];
    let ct = tag.split("ContentType=\"").nth(1)?;
    Some(ct[..ct.find('"')?].to_string())
}

/// Open + save `name` under another name (Save As, which is allowed even with losses).
/// Returns what disappeared and what SpeedSheet warned about when it opened the file.
fn lost_on_save(name: &str) -> (BTreeSet<String>, Vec<String>) {
    let src = corpus(name);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join(name);
    let mut wb = io::open(&src).unwrap();
    let warned = wb.losses.clone();
    if let Err(e) = io::save(&mut wb, &out, 0) {
        eprintln!("{name}: save refused ({e}), nothing lost silently");
        return (BTreeSet::new(), warned);
    }
    let before = inventory(&src);
    let after = inventory(&out);
    (before.difference(&after).cloned().collect(), warned)
}

/// Which warning (any of them) covers a lost item, by a piece of its inventory line.
/// `None` means: covered when there is any warning at all (e.g. the relationships file that only
/// exists for the lost charts or notes).
const WARNED_AS: &[(&str, Option<&[&str]>)] = &[
    ("xl/charts/", Some(&["Charts"])),
    ("xl/media/", Some(&["Images"])),
    ("xl/commentsN", Some(&["Notes (comments)"])),
    ("vmlDrawing", Some(&["Notes (comments)", "Form and ActiveX controls"])),
    ("<legacyDrawing>", Some(&["Notes (comments)", "Form and ActiveX controls"])),
    ("xl/drawings/", Some(&["Charts", "Images", "Shapes and text boxes"])),
    ("<drawing>", Some(&["Charts", "Images", "Shapes and text boxes"])),
    ("<dataValidations>", Some(&["Data validation (drop-down lists)"])),
    ("<hyperlinks>", Some(&["Hyperlinks"])),
    ("<headerFooter>", Some(&["Print headers and footers"])),
    ("<definedNames>", Some(&["Some defined names", "Print areas and print titles"])),
    ("<sheetProtection>", Some(&["Sheet protection"])),
    ("<workbookProtection>", Some(&["Workbook protection"])),
    ("<rowBreaks>", Some(&["Page breaks"])),
    ("<colBreaks>", Some(&["Page breaks"])),
    ("vbaProject", Some(&["Macros (VBA)"])),
    ("macroEnabled", Some(&["Macros (VBA)"])),
    ("xl/worksheets/_rels/", None),
];

fn warned_for(item: &str, warned: &[String]) -> bool {
    WARNED_AS.iter().filter(|(pat, _)| item.contains(pat)).any(|(_, labels)| match labels {
        Some(labels) => labels.iter().any(|l| warned.iter().any(|w| w == l)),
        None => !warned.is_empty(),
    })
}

fn check_fidelity(name: &str, known_silent: &[&str]) {
    let (lost, warned) = lost_on_save(name);
    let silent: BTreeSet<String> = lost.iter().filter(|item| !warned_for(item, &warned)).cloned().collect();
    let known: BTreeSet<String> = known_silent.iter().map(|s| s.to_string()).collect();
    let new: Vec<_> = silent.difference(&known).collect();
    let fixed: Vec<_> = known.difference(&silent).collect();
    assert!(new.is_empty(), "{name}: saving drops these without a warning: {new:#?}\n(warned: {warned:?})");
    assert!(fixed.is_empty(), "{name}: no longer lost silently; remove from the known list: {fixed:#?}");
}

#[test]
fn save_keeps_or_warns_named_ranges_validation_links_notes_charts() {
    check_fidelity("features.xlsx", &[PAGE_SETUP, &[SHEET_PROPERTIES, WORKBOOK_EXTENSIONS]].concat());
}

#[test]
fn save_keeps_or_warns_macros() {
    check_fidelity("macros.xlsm", &[PAGE_SETUP, &[CUSTOM_PROPERTIES]].concat());
}

#[test]
fn save_keeps_or_warns_formula_file() {
    check_fidelity("formulas.xlsx", &[PAGE_SETUP, &[SHEET_PROPERTIES, CUSTOM_PROPERTIES, WORKBOOK_EXTENSIONS]].concat());
}

// Found gaps (2026-11-28): saving drops these without a warning. Fix by keeping them or by
// adding them to `Workbook::losses` (and to WARNED_AS above), then delete the line.
const PAGE_SETUP: &[&str] = &[
    // Orientation, paper size, fit to page; print gridlines and centering.
    "xl/worksheets/sheetN.xml <pageSetup>",
    "xl/worksheets/sheetN.xml <printOptions>",
];
/// Tab color and fit-to-page flag.
const SHEET_PROPERTIES: &str = "xl/worksheets/sheetN.xml <sheetPr>";
/// Custom document properties (used for e.g. sensitivity labels).
const CUSTOM_PROPERTIES: &str = "part docProps/custom.xml";
/// Workbook extensions; in these LibreOffice files only its own calculation settings.
const WORKBOOK_EXTENSIONS: &str = "xl/workbook.xml <extLst>";
