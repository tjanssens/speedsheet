//! Regression tests for reported bugs: each test reproduces the reported scenario.

use speedsheet_core::{formula, DefinedName, Value, Workbook};

fn wb_with(rows: &[&[&str]]) -> Workbook {
    let mut wb = Workbook::new();
    let grid: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect();
    wb.set_inputs(0, 0, 0, &grid).unwrap();
    wb
}

fn show(wb: &Workbook, sheet: usize, r: usize, c: usize) -> String {
    wb.sheets[sheet].value(r, c).display()
}

fn set(wb: &mut Workbook, sheet: usize, r: usize, c: usize, input: &str) {
    wb.set_inputs(sheet, r, c, &[vec![input.to_string()]]).unwrap();
}

fn name(n: &str, f: &str) -> DefinedName {
    DefinedName { name: n.into(), formula: f.into(), hidden: false }
}

// ---- Renaming a sheet keeps formulas pointing at it.

#[test]
fn rename_sheet_updates_formulas() {
    let mut wb = wb_with(&[&["5"]]);
    let s2 = wb.add_sheet(Some("Report".into()));
    set(&mut wb, s2, 0, 0, "=Sheet1!A1*2");
    set(&mut wb, s2, 1, 0, "=SUM('Sheet1'!A1:A3)");
    assert_eq!(show(&wb, s2, 0, 0), "10");
    wb.rename_sheet(0, "Data".into()).unwrap();
    assert_eq!(wb.sheets[s2].cell(0, 0).input_text(), "=Data!A1*2");
    assert_eq!(wb.sheets[s2].cell(1, 0).input_text(), "=SUM(Data!A1:A3)");
    set(&mut wb, 0, 0, 0, "7");
    assert_eq!(show(&wb, s2, 0, 0), "14", "the formula must follow the renamed sheet");
    assert_eq!(show(&wb, s2, 1, 0), "7");

    wb.undo(); // A1 back to 5
    wb.undo(); // rename back
    assert_eq!(wb.sheets[0].name, "Sheet1");
    assert_eq!(wb.sheets[s2].cell(0, 0).input_text(), "=Sheet1!A1*2");
    assert_eq!(show(&wb, s2, 0, 0), "10");
    wb.redo();
    assert_eq!(wb.sheets[s2].cell(0, 0).input_text(), "=Data!A1*2");
    assert_eq!(show(&wb, s2, 0, 0), "10");
}

#[test]
fn rename_sheet_resolves_dangling_refs_and_quotes() {
    let mut wb = wb_with(&[&["3"]]);
    // A reference to a sheet that does not exist yet.
    set(&mut wb, 0, 1, 0, "='My Data'!A1+1");
    assert_eq!(show(&wb, 0, 1, 0), "#REF!");
    let s2 = wb.add_sheet(Some("Tmp".into()));
    set(&mut wb, s2, 0, 0, "41");
    set(&mut wb, 0, 2, 0, "=Tmp!A1");
    wb.rename_sheet(s2, "My Data".into()).unwrap();
    assert_eq!(wb.sheets[0].cell(2, 0).input_text(), "='My Data'!A1");
    assert_eq!(show(&wb, 0, 1, 0), "42", "a dangling reference resolves once the sheet exists");
    wb.undo();
    assert_eq!(show(&wb, 0, 1, 0), "#REF!");
    assert_eq!(wb.sheets[0].cell(2, 0).input_text(), "=Tmp!A1");
}

#[test]
fn rename_sheet_updates_defined_names() {
    let mut wb = wb_with(&[&["x", "0.21"]]);
    wb.names.push(name("Rate", "Sheet1!$B$1"));
    set(&mut wb, 0, 1, 0, "=Rate*100");
    wb.rename_sheet(0, "Config".into()).unwrap();
    assert_eq!(wb.names[0].formula, "Config!$B$1");
    set(&mut wb, 0, 0, 1, "0.06");
    assert_eq!(show(&wb, 0, 1, 0), "6");
    wb.undo();
    wb.undo();
    assert_eq!(wb.names[0].formula, "Sheet1!$B$1");
}

// ---- Defined names.

#[test]
fn defined_names_evaluate_and_update() {
    let mut wb = Workbook::new();
    wb.names.push(name("Rate", "Sheet1!$B$1"));
    wb.names.push(name("Sales", "Sheet1!$A$1:$A$3"));
    wb.names.push(name("Vat", "0.21"));
    wb.names.push(name("Twice", "Rate*2"));
    let grid: Vec<Vec<String>> =
        [["1", "0.5"], ["2", ""], ["3", ""]].iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect();
    wb.set_inputs(0, 0, 0, &grid).unwrap();
    set(&mut wb, 0, 0, 2, "=Rate*100");
    set(&mut wb, 0, 1, 2, "=SUM(Sales)");
    set(&mut wb, 0, 2, 2, "=vat*100");
    set(&mut wb, 0, 3, 2, "=Twice");
    set(&mut wb, 0, 4, 2, "=Missing*2");
    assert_eq!(show(&wb, 0, 0, 2), "50");
    assert_eq!(show(&wb, 0, 1, 2), "6");
    assert_eq!(show(&wb, 0, 2, 2), "21", "names are case-insensitive");
    assert_eq!(show(&wb, 0, 3, 2), "1");
    assert_eq!(show(&wb, 0, 4, 2), "#NAME?");
    // Editing a cell a name points at recalculates formulas using the name.
    set(&mut wb, 0, 0, 1, "0.25");
    assert_eq!(show(&wb, 0, 0, 2), "25");
    assert_eq!(show(&wb, 0, 3, 2), "0.5");
    set(&mut wb, 0, 1, 0, "20");
    assert_eq!(show(&wb, 0, 1, 2), "24");
}

#[test]
fn sheet_local_names_win_and_name_cycles_stop() {
    let mut wb = wb_with(&[&["1"]]);
    let s2 = wb.add_sheet(Some("Two".into()));
    wb.names.push(name("Level", "1"));
    wb.sheets[s2].names.push(name("Level", "2"));
    wb.names.push(name("Loop", "Loop+1"));
    set(&mut wb, 0, 0, 1, "=Level");
    set(&mut wb, s2, 0, 0, "=Level");
    set(&mut wb, 0, 0, 2, "=Loop");
    assert_eq!(show(&wb, 0, 0, 1), "1");
    assert_eq!(show(&wb, s2, 0, 0), "2");
    assert_eq!(show(&wb, 0, 0, 2), "#NAME?");
}

#[test]
fn names_from_file_recalculate_on_load() {
    // A formula without a cached value (as written by scripts) is computed with the names known.
    let mut sheet = speedsheet_core::Sheet::from_rows(
        "Sheet1",
        vec![vec![
            speedsheet_core::Cell::from_value(Value::Number(4.0)),
            speedsheet_core::Cell::formula("Base*10", Value::Empty),
        ]],
    );
    sheet.names.push(name("Base", "Sheet1!$A$1"));
    let wb = Workbook::from_sheets(vec![sheet], speedsheet_core::FileFormat::Xlsx);
    assert_eq!(show(&wb, 0, 0, 1), "40");
}

// ---- Undo after a sort restores formulas the sort turned into #REF!.

#[test]
fn undo_sort_restores_ref_errors() {
    let mut wb = wb_with(&[&["b", "x"], &["a", "=A1"]]);
    assert_eq!(show(&wb, 0, 1, 1), "b");
    wb.sort(0, 0, true, false).unwrap();
    // Row "a" moved to the top: its reference to the row above fell off the sheet.
    assert_eq!(wb.sheets[0].cell(0, 1).input_text(), "=#REF!");
    assert_eq!(show(&wb, 0, 0, 1), "#REF!");
    wb.undo();
    assert_eq!(wb.sheets[0].cell(1, 1).input_text(), "=A1");
    assert_eq!(show(&wb, 0, 1, 1), "b");
    wb.redo();
    assert_eq!(wb.sheets[0].cell(0, 1).input_text(), "=#REF!");
    assert_eq!(show(&wb, 0, 0, 1), "#REF!");
    wb.undo();
    assert_eq!(wb.sheets[0].cell(1, 1).input_text(), "=A1");
}

#[test]
fn sort_recalculates_formula_that_became_ref_error() {
    let mut wb = wb_with(&[&["name", "qty", "x2"], &["b", "2", "=B2*2"], &["a", "5", "=B3*2"], &["c", "1", "=SUM(B2:B4)"]]);
    assert_eq!(show(&wb, 0, 3, 2), "8");
    wb.sort(0, 0, false, false).unwrap();
    // "c" moved from row 4 to row 2, so SUM(B2:B4) shifted to rows 0..2 and lost its first row.
    let r = (0..4).find(|&r| show(&wb, 0, r, 0) == "c").unwrap();
    assert!(wb.sheets[0].cell(r, 2).input_text().contains("#REF!"));
    assert_eq!(show(&wb, 0, r, 2), "#REF!", "the stale value must not survive");
    wb.undo();
    assert_eq!(wb.sheets[0].cell(3, 2).input_text(), "=SUM(B2:B4)");
    assert_eq!(show(&wb, 0, 3, 2), "8");
}

// ---- Long dependency chains are not reported as circular.

#[test]
fn long_upward_chain_is_not_a_cycle() {
    const N: usize = 25_000;
    let mut grid: Vec<Vec<String>> = (0..N - 1).map(|r| vec![format!("=A{}+1", r + 2)]).collect();
    grid.push(vec!["1".into()]);
    let mut wb = Workbook::new();
    wb.set_inputs(0, 0, 0, &grid).unwrap();
    assert_eq!(show(&wb, 0, 0, 0), N.to_string());
    set(&mut wb, 0, N - 1, 0, "101");
    assert_eq!(show(&wb, 0, 0, 0), (N + 100).to_string());
}

#[test]
fn long_chain_on_load_is_not_a_cycle() {
    use speedsheet_core::{Cell, Sheet};
    const N: usize = 30_000;
    let mut rows: Vec<Vec<Cell>> = (0..N - 1).map(|r| vec![Cell::formula(&format!("A{}*1+1", r + 2), Value::Empty)]).collect();
    rows.push(vec![Cell::from_value(Value::Number(0.0))]);
    let wb = Workbook::from_sheets(vec![Sheet::from_rows("Sheet1", rows)], speedsheet_core::FileFormat::Xlsx);
    assert_eq!(show(&wb, 0, 0, 0), (N - 1).to_string());
}

#[test]
fn real_cycles_are_still_reported() {
    let mut wb = wb_with(&[&["=B1", "=A1", "=A1+1", "=IFERROR(A1,7)", "=D2"], &["", "", "", "=D2+1"]]);
    assert_eq!(show(&wb, 0, 0, 0), "#CYCLE!");
    assert_eq!(show(&wb, 0, 0, 1), "#CYCLE!");
    assert_eq!(show(&wb, 0, 0, 2), "#CYCLE!", "depends on a cycle");
    assert_eq!(show(&wb, 0, 0, 3), "7");
    assert_eq!(show(&wb, 0, 1, 3), "#CYCLE!", "a formula reading itself");
    assert_eq!(show(&wb, 0, 0, 4), "#CYCLE!");
    // Breaking the cycle fixes everything downstream.
    set(&mut wb, 0, 0, 1, "5");
    assert_eq!(show(&wb, 0, 0, 0), "5");
    assert_eq!(show(&wb, 0, 0, 2), "6");
    assert_eq!(show(&wb, 0, 0, 3), "5");
}

// ---- TODAY() and NOW() use local time.

#[test]
fn today_uses_the_local_date() {
    let wb = Workbook::new();
    let serial = |src: &str| match wb.evaluate(0, src) {
        Value::Number(n) | Value::Date(n) => n,
        v => panic!("{src}: {v:?}"),
    };
    // 2026-11-27 23:13 UTC is 00:13 on the 28th in Brussels (UTC+1 in winter).
    let unix = 1_795_821_180.0; // 2026-11-27T23:13:00Z
    assert_eq!(formula::serial_from_unix(unix, 0).floor(), serial("=DATE(2026,11,27)"));
    assert_eq!(formula::serial_from_unix(unix, 3600).floor(), serial("=DATE(2026,11,28)"));
    // Summer time: 2026-07-01 23:30 UTC is 01:30 on July 2nd at UTC+2.
    let unix = 1_782_948_600.0; // 2026-07-01T23:30:00Z
    assert_eq!(formula::serial_from_unix(unix, 7200).floor(), serial("=DATE(2026,7,2)"));
    // And TODAY() agrees with the system's local calendar date.
    let today = chrono::Local::now().date_naive();
    let expect = serial(&format!(
        "=DATE({},{},{})",
        chrono::Datelike::year(&today),
        chrono::Datelike::month(&today),
        chrono::Datelike::day(&today)
    ));
    let got = serial("=TODAY()");
    // Allow for the test running across midnight.
    assert!(got == expect || got == expect + 1.0, "TODAY()={got}, local date={expect}");
}

// ---- Saving never silently drops what SpeedSheet cannot keep.

fn zip_entry(path: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut f = z.by_name(name).ok()?;
    let mut v = Vec::new();
    f.read_to_end(&mut v).unwrap();
    Some(v)
}

/// The reported file: a named range, a drop-down list, a hyperlink and a note (plus a chart).
fn rich_file(path: &std::path::Path) {
    use rust_xlsxwriter::{Chart, DataValidation, Formula, Note, Workbook as X};
    let mut x = X::new();
    let ws = x.add_worksheet();
    ws.write_number(0, 1, 0.21).unwrap();
    ws.write_formula(1, 0, Formula::new("=Rate*100").set_result("21")).unwrap();
    ws.add_data_validation(2, 0, 9, 0, &DataValidation::new().allow_list_strings(&["yes", "no"]).unwrap()).unwrap();
    ws.write_url(3, 1, "https://example.com").unwrap();
    ws.insert_note(0, 1, &Note::new("the VAT rate")).unwrap();
    let mut chart = Chart::new_column();
    chart.add_series().set_values("Sheet1!$B$1:$B$3");
    ws.insert_chart(5, 3, &chart).unwrap();
    x.define_name("Rate", "=Sheet1!$B$1").unwrap();
    x.save(path).unwrap();
}

#[test]
fn saving_over_a_file_with_unkept_features_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("rich.xlsx");
    rich_file(&p);
    let mut wb = speedsheet_core::io::open(&p).unwrap();
    for what in ["Data validation (drop-down lists)", "Hyperlinks", "Notes (comments)", "Charts"] {
        assert!(wb.losses.iter().any(|l| l == what), "{what} missing from {:?}", wb.losses);
    }
    assert!(!wb.losses.iter().any(|l| l.contains("name")), "defined names are kept: {:?}", wb.losses);
    assert!(speedsheet_core::io::can_save_in_place(&wb));
    let before = std::fs::read(&p).unwrap();
    set(&mut wb, 0, 0, 0, "edit");
    match speedsheet_core::io::save(&mut wb, &p, 0) {
        Err(speedsheet_core::Error::WouldLoseData(l)) => assert_eq!(l, wb.losses),
        other => panic!("expected WouldLoseData, got {other:?}"),
    }
    assert_eq!(std::fs::read(&p).unwrap(), before, "the original file is untouched");
    // A copy is fine: the original keeps everything.
    let copy = dir.path().join("copy.xlsx");
    speedsheet_core::io::save(&mut wb, &copy, 0).unwrap();
    assert!(wb.losses.is_empty(), "the copy holds exactly what the workbook does");
    // Overwriting after the user agreed works too.
    let mut wb = speedsheet_core::io::open(&p).unwrap();
    speedsheet_core::io::save_anyway(&mut wb, &p, 0).unwrap();
    assert!(speedsheet_core::io::open(&p).unwrap().losses.is_empty());
}

#[test]
fn defined_names_survive_open_and_save() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("rich.xlsx");
    rich_file(&p);
    let mut wb = speedsheet_core::io::open(&p).unwrap();
    assert_eq!(wb.names, vec![name("Rate", "Sheet1!$B$1")]);
    assert_eq!(show(&wb, 0, 1, 0), "21", "=Rate*100 evaluates");
    set(&mut wb, 0, 0, 1, "0.06");
    assert_eq!(show(&wb, 0, 1, 0), "6", "and follows its cell");
    wb.sheets[0].names.push(name("Local", "Sheet1!$B$1*2"));
    let copy = dir.path().join("copy.xlsx");
    speedsheet_core::io::save(&mut wb, &copy, 0).unwrap();
    let back = speedsheet_core::io::open(&copy).unwrap();
    assert_eq!(back.names, vec![name("Rate", "Sheet1!$B$1")]);
    assert_eq!(back.sheets[0].names, vec![name("Local", "Sheet1!$B$1*2")]);
    assert_eq!(back.sheets[0].cell(1, 0).input_text(), "=Rate*100");
    assert_eq!(show(&back, 0, 1, 0), "6");
}

#[test]
fn xlsm_keeps_its_macros() {
    use rust_xlsxwriter::Workbook as X;
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("vbaProject.bin");
    let project = b"\xd0\xcf\x11\xe0 not a real VBA project, just bytes to carry".to_vec();
    std::fs::write(&bin, &project).unwrap();
    let p = dir.path().join("macros.xlsm");
    let mut x = X::new();
    x.add_vba_project(&bin).unwrap();
    x.set_vba_name("MyBook").unwrap();
    x.add_worksheet().set_vba_name("DataSheet").unwrap().write_number(0, 0, 1.0).unwrap();
    x.save(&p).unwrap();

    let mut wb = speedsheet_core::io::open(&p).unwrap();
    assert!(wb.losses.is_empty(), "{:?}", wb.losses);
    assert!(speedsheet_core::io::can_save_in_place(&wb));
    set(&mut wb, 0, 0, 0, "2");
    speedsheet_core::io::save(&mut wb, &p, 0).unwrap();
    let types = String::from_utf8(zip_entry(&p, "[Content_Types].xml").unwrap()).unwrap();
    assert!(types.contains("application/vnd.ms-excel.sheet.macroEnabled.main+xml"), "{types}");
    assert_eq!(zip_entry(&p, "xl/vbaProject.bin").unwrap(), project);
    let wbxml = String::from_utf8(zip_entry(&p, "xl/workbook.xml").unwrap()).unwrap();
    assert!(wbxml.contains("codeName=\"MyBook\""), "{wbxml}");
    let sheet = String::from_utf8(zip_entry(&p, "xl/worksheets/sheet1.xml").unwrap()).unwrap();
    assert!(sheet.contains("codeName=\"DataSheet\""), "{sheet}");
    assert_eq!(speedsheet_core::io::open(&p).unwrap().sheets[0].value(0, 0), &Value::Number(2.0));

    // Saved as .xlsx: a plain workbook without macros.
    let plain = dir.path().join("plain.xlsx");
    speedsheet_core::io::save(&mut wb, &plain, 0).unwrap();
    let types = String::from_utf8(zip_entry(&plain, "[Content_Types].xml").unwrap()).unwrap();
    assert!(!types.contains("macroEnabled"));
    assert!(zip_entry(&plain, "xl/vbaProject.bin").is_none());
}

#[test]
fn xlsm_without_macros_is_never_written() {
    let dir = tempfile::tempdir().unwrap();
    let mut wb = wb_with(&[&["1"]]);
    let p = dir.path().join("new.xlsm");
    assert!(matches!(speedsheet_core::io::save(&mut wb, &p, 0), Err(speedsheet_core::Error::Unsupported(_))));
    assert!(!p.exists());
    // An .xlsx that pretends to have macros is not trusted either.
    let x = dir.path().join("t.xltx");
    assert!(speedsheet_core::io::save(&mut wb, &x, 0).is_err());
}

#[test]
fn hidden_sheets_and_rich_text_are_reported() {
    use rust_xlsxwriter::{Format, Workbook as X};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t.xlsx");
    let mut x = X::new();
    let bold = Format::new().set_bold();
    x.add_worksheet().write_rich_string(0, 0, &[(&bold, "bold"), (&Format::default(), " plain")]).unwrap();
    x.add_worksheet().set_hidden(true);
    x.add_worksheet().protect();
    x.save(&p).unwrap();
    let wb = speedsheet_core::io::open(&p).unwrap();
    assert_eq!(wb.losses, ["Hidden sheets (they will show)", "Sheet protection", "Text formatting within cells (rich text)"]);
    // A plain file has nothing to lose.
    let q = dir.path().join("plain.xlsx");
    let mut plain = wb_with(&[&["1", "=A1*2"]]);
    speedsheet_core::io::save(&mut plain, &q, 0).unwrap();
    assert!(speedsheet_core::io::open(&q).unwrap().losses.is_empty());
}
