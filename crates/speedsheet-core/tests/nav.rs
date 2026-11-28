// Ctrl+Arrow, compared with what Excel does on the same layout.
use speedsheet_core::{input::parse_input, nav::jump, Filter, FilterCond, FilterOp, Sheet, Workbook, MAX_COLS, MAX_ROWS};

/// A sheet from a picture: `x` is a filled cell, `.` an empty one.
fn sheet(rows: &[&str]) -> Sheet {
    let cells = rows.iter().map(|r| r.chars().map(|ch| parse_input(if ch == 'x' { "1" } else { "" })).collect()).collect();
    Sheet::from_rows("S", cells)
}

#[test]
fn down_goes_to_the_end_of_a_block_then_to_the_next_block_then_the_edge() {
    let s = sheet(&["x", "x", "x", ".", ".", "x", "x", "."]);
    assert_eq!(jump(&s, 0, 0, 1, 0), (2, 0), "inside a block: its last cell");
    assert_eq!(jump(&s, 2, 0, 1, 0), (5, 0), "at the end of a block: the next filled cell");
    assert_eq!(jump(&s, 3, 0, 1, 0), (5, 0), "from an empty cell: the next filled cell");
    assert_eq!(jump(&s, 5, 0, 1, 0), (6, 0));
    assert_eq!(jump(&s, 6, 0, 1, 0), (MAX_ROWS - 1, 0), "nothing below: the last row of the sheet");
    assert_eq!(jump(&s, MAX_ROWS - 1, 0, 1, 0), (MAX_ROWS - 1, 0), "already at the edge");
}

#[test]
fn up_mirrors_down() {
    let s = sheet(&["x", "x", ".", "x", ".", "."]);
    assert_eq!(jump(&s, 5, 0, -1, 0), (3, 0));
    assert_eq!(jump(&s, 3, 0, -1, 0), (1, 0));
    assert_eq!(jump(&s, 1, 0, -1, 0), (0, 0));
    assert_eq!(jump(&s, 0, 0, -1, 0), (0, 0));
    let empty = sheet(&[".", ".", "."]);
    assert_eq!(jump(&empty, 2, 0, -1, 0), (0, 0), "nothing above: row 1");
}

#[test]
fn sideways_works_the_same_and_ends_at_the_last_column() {
    let s = sheet(&["xx..x."]);
    assert_eq!(jump(&s, 0, 0, 0, 1), (0, 1));
    assert_eq!(jump(&s, 0, 1, 0, 1), (0, 4));
    assert_eq!(jump(&s, 0, 4, 0, 1), (0, MAX_COLS - 1));
    assert_eq!(jump(&s, 0, 4, 0, -1), (0, 1));
    assert_eq!(jump(&s, 0, 1, 0, -1), (0, 0));
}

#[test]
fn a_single_filled_cell_is_its_own_block() {
    let s = sheet(&[".", "x", ".", "x"]);
    assert_eq!(jump(&s, 1, 0, 1, 0), (3, 0), "from a lone cell: the next filled cell");
    assert_eq!(jump(&s, 3, 0, -1, 0), (1, 0));
}

#[test]
fn a_filtered_sheet_jumps_over_hidden_rows_and_ends_at_its_last_visible_row() {
    // Rows a, b, (empty), c, d; the filter hides b and d, so the view is a, (empty), c.
    let rows = ["a", "b", "", "c", "d"].iter().map(|t| vec![parse_input(t)]).collect();
    let mut wb = Workbook::from_sheets(vec![Sheet::from_rows("S", rows)], speedsheet_core::FileFormat::Xlsx);
    let f = Filter {
        header: false,
        conds: vec![FilterCond { col: 0, op: FilterOp::NotIn { values: vec!["b".into(), "d".into()] } }],
    };
    wb.set_filter(0, Some(f)).unwrap();
    let s = &wb.sheets[0];
    assert_eq!(s.view_len(), 3);
    assert_eq!(jump(s, 0, 0, 1, 0), (2, 0), "over the empty row to c");
    assert_eq!(jump(s, 2, 0, 1, 0), (2, 0), "the last visible row is the edge");
}
