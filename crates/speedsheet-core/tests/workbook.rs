use speedsheet_core::{io, Cell, Filter, FilterCond, FilterOp, Value, Workbook};

fn wb_with(rows: &[&[&str]]) -> Workbook {
    let mut wb = Workbook::new();
    let grid: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect();
    wb.set_inputs(0, 0, 0, &grid).unwrap();
    wb
}

fn show(wb: &Workbook, r: usize, c: usize) -> String {
    wb.sheets[0].value(r, c).display()
}

#[test]
fn formulas_calculate_and_update() {
    let mut wb = wb_with(&[&["1", "=A1*2", "=SUM(A1:A3)"], &["2", "=B1+A2"], &["3", "", "=AVERAGE(A1:A3)"]]);
    assert_eq!(show(&wb, 0, 1), "2");
    assert_eq!(show(&wb, 1, 1), "4");
    assert_eq!(show(&wb, 0, 2), "6");
    assert_eq!(show(&wb, 2, 2), "2");
    wb.set_inputs(0, 0, 0, &[vec!["10".into()]]).unwrap();
    assert_eq!(show(&wb, 0, 1), "20");
    assert_eq!(show(&wb, 1, 1), "22");
    assert_eq!(show(&wb, 0, 2), "15");
    wb.undo();
    assert_eq!(show(&wb, 1, 1), "4");
    wb.redo();
    assert_eq!(show(&wb, 1, 1), "22");
}

#[test]
fn many_functions() {
    let cases: &[(&str, &str)] = &[
        ("=IF(1>2,\"a\",\"b\")", "b"),
        ("=IFERROR(1/0,\"x\")", "x"),
        ("=1/0", "#DIV/0!"),
        ("=ROUND(2.675,2)", "2.68"),
        ("=CONCAT(\"a\",1,TRUE)", "a1TRUE"),
        ("=\"x\"&2", "x2"),
        ("=LEFT(\"hello\",2)&RIGHT(\"hello\")", "heo"),
        ("=MID(\"spreadsheet\",7,5)", "sheet"),
        ("=UPPER(\"abc\")", "ABC"),
        ("=LEN(\"héllo\")", "5"),
        ("=SUBSTITUTE(\"a-b-c\",\"-\",\"+\")", "a+b+c"),
        ("=TEXT(1234.5,\"#,##0.00\")", "1,234.50"),
        ("=DATE(2026,11,27)", "2026-11-27"),
        ("=YEAR(DATE(2026,11,27))", "2026"),
        ("=MOD(-3,5)", "2"),
        ("=2^10", "1024"),
        ("=-2^2", "4"),
        ("=50%", "0.5"),
        ("=AND(TRUE,1)", "TRUE"),
        ("=OR(FALSE,0)", "FALSE"),
        ("=NOT(TRUE)", "FALSE"),
        ("=MAX(1,5,3)", "5"),
        ("=MEDIAN(1,2,3,4)", "2.5"),
        ("=FOO(1)", "#NAME?"),
        ("=CHOOSE(2,\"a\",\"b\")", "b"),
        ("=\"abc\"=\"ABC\"", "TRUE"),
        ("=SEARCH(\"B\",\"abc\")", "2"),
        ("=TRIM(\"  a   b \")", "a b"),
    ];
    for (f, want) in cases {
        let wb = wb_with(&[&[f]]);
        assert_eq!(&show(&wb, 0, 0), want, "formula {f}");
    }
}

#[test]
fn lookups_and_conditionals() {
    let mut wb = wb_with(&[&["apple", "3", "red"], &["banana", "5", "yellow"], &["cherry", "7", "red"]]);
    let f = |s: &str| vec![s.to_string()];
    wb.set_inputs(
        0,
        0,
        4,
        &[
            f("=VLOOKUP(\"banana\",A1:C3,2,FALSE)"),
            f("=MATCH(\"cherry\",A1:A3,0)"),
            f("=INDEX(A1:C3,3,3)"),
            f("=SUMIF(C1:C3,\"red\",B1:B3)"),
            f("=COUNTIF(B1:B3,\">4\")"),
            f("=XLOOKUP(\"kiwi\",A1:A3,B1:B3,\"none\")"),
            f("=SUMIFS(B1:B3,C1:C3,\"red\",A1:A3,\"c*\")"),
            f("=VLOOKUP(6,B1:C3,2)"),
            f("=SUM(B:B)"),
            f("=AVERAGEIF(C:C,\"red\",B:B)"),
        ],
    )
    .unwrap();
    let col = |r| wb.sheets[0].value(r, 4).display();
    assert_eq!(col(0), "5");
    assert_eq!(col(1), "3");
    assert_eq!(col(2), "red");
    assert_eq!(col(3), "10");
    assert_eq!(col(4), "2");
    assert_eq!(col(5), "none");
    assert_eq!(col(6), "7");
    assert_eq!(col(7), "yellow");
    assert_eq!(col(8), "15");
    assert_eq!(col(9), "5");
}

#[test]
fn cross_sheet_and_cycles() {
    let mut wb = wb_with(&[&["5"]]);
    let i = wb.add_sheet(Some("Data 2".into()));
    wb.set_inputs(i, 0, 0, &[vec!["='Sheet1'!A1*3".into(), "=Sheet1!A1+1".into()]]).unwrap();
    assert_eq!(wb.sheets[i].value(0, 0).display(), "15");
    wb.set_inputs(0, 0, 0, &[vec!["2".into()]]).unwrap();
    assert_eq!(wb.sheets[i].value(0, 0).display(), "6");
    assert_eq!(wb.sheets[i].value(0, 1).display(), "3");
    wb.set_inputs(0, 1, 0, &[vec!["=A3".into()], vec!["=A2".into()]]).unwrap();
    assert_eq!(show(&wb, 1, 0), "#CYCLE!");
}

#[test]
fn long_dependency_chain() {
    let mut wb = Workbook::new();
    let mut grid = vec![vec!["1".to_string()]];
    for r in 1..50_000 {
        grid.push(vec![format!("=A{}+1", r)]);
    }
    wb.set_inputs(0, 0, 0, &grid).unwrap();
    assert_eq!(show(&wb, 49_999, 0), "50000");
    wb.set_inputs(0, 0, 0, &[vec!["101".into()]]).unwrap();
    assert_eq!(show(&wb, 49_999, 0), "50100");
}

#[test]
fn sort_moves_formulas_and_undoes() {
    let mut wb = wb_with(&[&["name", "qty", "double"], &["b", "2", "=B2*2"], &["c", "", "=B3*2"], &["a", "10", "=B4*2"]]);
    wb.sort(0, 0, true, true).unwrap();
    let names: Vec<String> = (0..4).map(|r| show(&wb, r, 0)).collect();
    assert_eq!(names, ["name", "a", "b", "c"]);
    assert_eq!(show(&wb, 1, 2), "20");
    assert_eq!(wb.sheets[0].cell(1, 2).input_text(), "=B2*2");
    wb.sort(0, 1, true, true).unwrap();
    assert_eq!(show(&wb, 1, 1), "2");
    assert_eq!(show(&wb, 1, 2), "4");
    assert_eq!(show(&wb, 3, 0), "c", "empty sorts last");
    wb.undo();
    wb.undo();
    let names: Vec<String> = (0..4).map(|r| show(&wb, r, 0)).collect();
    assert_eq!(names, ["name", "b", "c", "a"]);
    assert_eq!(wb.sheets[0].cell(3, 2).input_text(), "=B4*2");
}

#[test]
fn filter_and_edit_through_view() {
    let mut wb = wb_with(&[&["city", "n"], &["Gent", "1"], &["Brussel", "2"], &["Gentbrugge", "3"]]);
    let n = wb
        .set_filter(
            0,
            Some(Filter { header: true, conds: vec![FilterCond { col: 0, op: FilterOp::Contains { value: "gent".into() } }] }),
        )
        .unwrap();
    assert_eq!(n, 3);
    // View row 2 is data row 3.
    wb.set_inputs(0, 2, 1, &[vec!["30".into()]]).unwrap();
    assert_eq!(show(&wb, 3, 1), "30");
    let n = wb
        .set_filter(0, Some(Filter { header: true, conds: vec![FilterCond { col: 1, op: FilterOp::Gt { value: "1".into() } }] }))
        .unwrap();
    assert_eq!(n, 3);
    wb.set_filter(0, None).unwrap();
    assert_eq!(wb.sheets[0].view_len(), 4);
}

#[test]
fn find_wraps() {
    let wb = wb_with(&[&["x", "needle"], &["needle", "y"]]);
    let m = wb.find("NEEDLE", 0, 0, 1, false, false).unwrap();
    assert_eq!((m.row, m.col), (1, 0));
    let m = wb.find("needle", 0, 1, 0, false, false).unwrap();
    assert_eq!((m.row, m.col), (0, 1));
    let m = wb.find("needle", 0, 1, 0, true, false).unwrap();
    assert_eq!((m.row, m.col), (0, 1));
    assert!(wb.find("absent", 0, 0, 0, false, true).is_none());
}

#[test]
fn xlsx_and_csv_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut wb = wb_with(&[&["a", "1.5", "=B1*2", "2026-11-27", "TRUE"], &["ü € \"q\"", "-3", "=SUM(B1:B2)"]]);
    wb.add_sheet(Some("Second".into()));
    wb.set_inputs(1, 0, 0, &[vec!["=Sheet1!B1".into()]]).unwrap();
    let p = dir.path().join("t.xlsx");
    io::save(&mut wb, &p, 0).unwrap();
    let back = io::open(&p).unwrap();
    assert_eq!(back.sheets.len(), 2);
    let s = &back.sheets[0];
    assert_eq!(s.value(0, 0).display(), "a");
    assert_eq!(s.value(0, 1), &Value::Number(1.5));
    assert_eq!(s.cell(0, 2).input_text(), "=B1*2");
    assert_eq!(s.value(0, 2).display(), "3");
    assert_eq!(s.value(0, 3).display(), "2026-11-27");
    assert_eq!(s.value(0, 4), &Value::Bool(true));
    assert_eq!(s.value(1, 0).display(), "ü € \"q\"");
    assert_eq!(s.value(1, 2).display(), "-1.5");
    assert_eq!(back.sheets[1].value(0, 0).display(), "1.5");

    let c = dir.path().join("t.csv");
    io::save(&mut wb, &c, 0).unwrap();
    let back = io::open(&c).unwrap();
    assert_eq!(back.sheets[0].value(1, 0).display(), "ü € \"q\"");
    assert_eq!(back.sheets[0].value(0, 2), &Value::Number(3.0));
}

#[test]
fn csv_semicolon_and_cp1252() {
    let bytes = b"naam;bedrag\nJos;3,5\nCaf\xe9;\x80 2\n";
    let (s, d) = io::read_csv(bytes, "x".into(), None).unwrap();
    assert_eq!(d, b';');
    assert_eq!(s.value(1, 1), &Value::Number(3.5));
    assert_eq!(s.value(2, 0).display(), "Café");
    assert_eq!(s.value(2, 1).display(), "€ 2");
}

#[test]
fn clear_and_undo() {
    let mut wb = wb_with(&[&["1", "2"], &["3", "=A1+B1+A2"]]);
    wb.clear(0, 0, 0, 0, 1).unwrap();
    assert_eq!(show(&wb, 1, 1), "3");
    wb.undo();
    assert_eq!(show(&wb, 1, 1), "6");
    assert!(matches!(wb.sheets[0].cell(0, 0), Cell::Value(_)));
}

#[test]
fn sheet_ops_undo() {
    let mut wb = Workbook::new();
    let i = wb.add_sheet(None);
    assert_eq!(wb.sheets[i].name, "Sheet2");
    wb.rename_sheet(i, "Totals".into()).unwrap();
    assert!(wb.rename_sheet(0, "totals".into()).is_err());
    wb.delete_sheet(0).unwrap();
    assert_eq!(wb.sheets.len(), 1);
    wb.undo();
    assert_eq!(wb.sheets[0].name, "Sheet1");
    wb.undo();
    assert_eq!(wb.sheets[1].name, "Sheet2");
    wb.undo();
    assert_eq!(wb.sheets.len(), 1);
    wb.redo();
    assert_eq!(wb.sheets.len(), 2);
}

#[test]
fn fill_handle_series_formulas_and_undo() {
    let mut wb = wb_with(&[&["1", "=A1*10", "Maandag"], &["2", "", ""], &["", "", ""], &["", "", ""]]);
    // A1:A2 down to A5: 3, 4, 5.
    wb.fill(0, (0, 0, 1, 0), (0, 0, 4, 0)).unwrap();
    assert_eq!((show(&wb, 2, 0), show(&wb, 3, 0), show(&wb, 4, 0)), ("3".into(), "4".into(), "5".into()));
    // B1 down: formulas shift and calculate.
    wb.fill(0, (0, 1, 0, 1), (0, 1, 4, 1)).unwrap();
    assert_eq!(wb.sheets[0].cell(3, 1).input_text(), "=A4*10");
    assert_eq!(show(&wb, 3, 1), "40");
    // C1 right: weekday names continue along the row.
    wb.fill(0, (0, 2, 0, 2), (0, 2, 0, 4)).unwrap();
    assert_eq!((show(&wb, 0, 3), show(&wb, 0, 4)), ("Dinsdag".into(), "Woensdag".into()));
    wb.undo();
    assert_eq!(show(&wb, 0, 3), "");
    wb.undo();
    assert_eq!(show(&wb, 3, 1), "");
    // Double-click fill: B goes down as far as column A has data.
    wb.set_inputs(0, 0, 1, &[vec!["=A1+1".into()]]).unwrap();
    let (end, _) = wb.fill_down_auto(0, (0, 1, 0, 1)).unwrap().unwrap();
    assert_eq!(end, 4);
    assert_eq!(show(&wb, 4, 1), "6");
    assert_eq!(show(&wb, 5, 1), "");
}

#[test]
fn fill_copies_formats() {
    let mut wb = wb_with(&[&["x"], &[""]]);
    wb.styles.push(speedsheet_core::style::CellStyle { bold: true, ..Default::default() });
    wb.sheets[0].set_style_id(0, 0, 1);
    wb.fill(0, (0, 0, 0, 0), (0, 0, 2, 0)).unwrap();
    assert_eq!(wb.sheets[0].style_id(2, 0), 1);
    assert_eq!(show(&wb, 2, 0), "x");
    wb.undo();
    assert_eq!(wb.sheets[0].style_id(2, 0), 0);
    assert_eq!(wb.sheets[0].style_id(0, 0), 1);
}
