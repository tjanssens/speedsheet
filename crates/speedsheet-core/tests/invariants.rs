//! Property tests: random sequences of user operations, checked against invariants that must
//! hold for *every* sequence. They catch whole classes of bugs instead of one example:
//!
//! - stale values: the incrementally recalculated workbook must equal a full recalculation;
//! - lossy undo: undoing everything must give back the original workbook, redo the final one;
//! - lossy save: saving as xlsx and opening again must give the same cells.
//!
//! On failure proptest shrinks the sequence to a minimal one and stores its seed in
//! `invariants.proptest-regressions`, which is committed so the case is replayed on every run.
//! More cases: `PROPTEST_CASES=5000 cargo test -p speedsheet-core --test invariants`.

use proptest::prelude::*;
use speedsheet_core::{input, io, FileFormat, Sheet, Workbook};

const ROWS: usize = 6;
const COLS: usize = 4;
const COL_NAMES: [&str; COLS] = ["A", "B", "C", "D"];

#[derive(Clone, Debug)]
enum Op {
    Set { sheet: usize, row: usize, col: usize, input: String },
    Clear { sheet: usize, r0: usize, c0: usize, r1: usize, c1: usize },
    Sort { sheet: usize, col: usize, ascending: bool, header: bool },
    FillDown { sheet: usize, row: usize, col: usize, len: usize },
    AddSheet,
    DeleteSheet { sheet: usize },
    RenameSheet { sheet: usize, name: String },
    Undo,
    Redo,
}

/// Which operations a test may generate.
#[derive(Clone, Copy)]
struct Mix {
    sort: bool,
    rename: bool,
}

fn cell_ref() -> impl Strategy<Value = String> {
    (0..COLS, 1..=ROWS + 1, any::<bool>(), prop::option::weighted(0.2, 0..3usize)).prop_map(|(c, r, abs, sheet)| {
        let a = if abs { "$" } else { "" };
        let s = sheet.map(|s| format!("Sheet{}!", s + 1)).unwrap_or_default();
        format!("{s}{a}{}{a}{r}", COL_NAMES[c])
    })
}

fn formula() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![cell_ref(), (0..100i32).prop_map(|n| n.to_string())];
    let expr = leaf.prop_recursive(3, 12, 3, |inner| {
        prop_oneof![
            (inner.clone(), prop::sample::select(vec!["+", "-", "*"]), inner.clone()).prop_map(|(a, o, b)| format!("{a}{o}{b}")),
            (0..COLS, 1..=ROWS, 1..=ROWS).prop_map(|(c, r0, r1)| {
                let (r0, r1) = (r0.min(r1), r0.max(r1));
                format!("SUM({0}{r0}:{0}{r1})", COL_NAMES[c])
            }),
            (inner.clone(), inner.clone(), inner).prop_map(|(a, b, c)| format!("IF({a}>3,{b},{c})")),
        ]
    });
    expr.prop_map(|e| format!("={e}"))
}

fn input() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => (-50..50i32).prop_map(|n| n.to_string()),
        1 => prop::sample::select(vec!["a", "b", "zz", "Gent"]).prop_map(String::from),
        3 => formula(),
        1 => Just(String::new()),
    ]
}

fn op(mix: Mix) -> impl Strategy<Value = Op> {
    let sheet = 0..3usize;
    let set = (sheet.clone(), 0..ROWS, 0..COLS, input()).prop_map(|(sheet, row, col, input)| Op::Set { sheet, row, col, input });
    let clear = (sheet.clone(), 0..ROWS, 0..COLS, 0..ROWS, 0..COLS).prop_map(|(sheet, a, b, c, d)| Op::Clear {
        sheet,
        r0: a.min(c),
        c0: b.min(d),
        r1: a.max(c),
        c1: b.max(d),
    });
    let sort = (sheet.clone(), 0..COLS, any::<bool>(), any::<bool>()).prop_map(|(sheet, col, ascending, header)| Op::Sort {
        sheet,
        col,
        ascending,
        header,
    });
    let fill =
        (sheet.clone(), 0..ROWS, 0..COLS, 1..4usize).prop_map(|(sheet, row, col, len)| Op::FillDown { sheet, row, col, len });
    let rename = (sheet.clone(), prop::sample::select(vec!["Data", "Totals", "Sheet9"]))
        .prop_map(|(sheet, name)| Op::RenameSheet { sheet, name: name.into() });
    let mut ops: Vec<(u32, BoxedStrategy<Op>)> = vec![
        (8, set.boxed()),
        (1, clear.boxed()),
        (2, fill.boxed()),
        (1, Just(Op::AddSheet).boxed()),
        (1, sheet.clone().prop_map(|sheet| Op::DeleteSheet { sheet }).boxed()),
        (2, Just(Op::Undo).boxed()),
        (1, Just(Op::Redo).boxed()),
    ];
    if mix.sort {
        ops.push((2, sort.boxed()));
    }
    if mix.rename {
        ops.push((1, rename.boxed()));
    }
    prop::strategy::Union::new_weighted(ops)
}

/// Apply an op; ops that do not apply (missing sheet, invalid name) are skipped like the UI would.
fn apply(wb: &mut Workbook, op: &Op) {
    let has = |wb: &Workbook, s: usize| s < wb.sheets.len();
    let _applied = match op {
        Op::Set { sheet, row, col, input } if has(wb, *sheet) => {
            wb.set_inputs(*sheet, *row, *col, &[vec![input.clone()]]).is_ok()
        }
        Op::Clear { sheet, r0, c0, r1, c1 } if has(wb, *sheet) => wb.clear(*sheet, *r0, *c0, *r1, *c1).is_ok(),
        Op::Sort { sheet, col, ascending, header } if has(wb, *sheet) => wb.sort(*sheet, *col, *ascending, *header).is_ok(),
        Op::FillDown { sheet, row, col, len } if has(wb, *sheet) => {
            wb.fill(*sheet, (*row, *col, *row, *col), (*row, *col, row + len, *col)).is_ok()
        }
        Op::AddSheet => wb.add_sheet(None) > 0,
        Op::DeleteSheet { sheet } if has(wb, *sheet) => wb.delete_sheet(*sheet).is_ok(),
        Op::RenameSheet { sheet, name } if has(wb, *sheet) => wb.rename_sheet(*sheet, name.clone()).is_ok(),
        Op::Undo => wb.undo().is_some(),
        Op::Redo => wb.redo().is_some(),
        _ => false,
    };
}

/// Per sheet: its name and, per row, (input text, displayed value) of each cell.
type Snapshot = Vec<(String, Vec<Vec<(String, String)>>)>;

/// Everything a user sees: sheet names, what they would edit, and what is displayed.
/// Trailing empty cells and rows are invisible, so they are left out.
fn snapshot(wb: &Workbook) -> Snapshot {
    wb.sheets
        .iter()
        .map(|s| {
            let mut rows: Vec<Vec<(String, String)>> = (0..s.row_count())
                .map(|r| {
                    let mut row: Vec<(String, String)> =
                        (0..s.col_count()).map(|c| (s.cell(r, c).input_text(), s.value(r, c).display())).collect();
                    while row.last().is_some_and(|(i, v)| i.is_empty() && v.is_empty()) {
                        row.pop();
                    }
                    row
                })
                .collect();
            while rows.last().is_some_and(|r| r.is_empty()) {
                rows.pop();
            }
            (s.name.clone(), rows)
        })
        .collect()
}

/// The starting workbook: two sheets with a few values and formulas, one across sheets.
/// Built directly (not through edits) so it has no undo history.
fn start() -> Workbook {
    let sheet = |name: &str, rows: &[&[&str]]| {
        Sheet::from_rows(name, rows.iter().map(|r| r.iter().map(|s| input::parse_input(s)).collect()).collect())
    };
    Workbook::from_sheets(
        vec![
            sheet("Sheet1", &[&["name", "qty", "x2"], &["b", "2", "=B2*2"], &["a", "5", "=B3*2"], &["c", "1", "=SUM(B2:B4)"]]),
            sheet("Sheet2", &[&["=Sheet1!B2+1", "7"], &["=A1*B1", "=SUM(Sheet1!B2:B4)"]]),
        ],
        FileFormat::New,
    )
}

/// Incremental recalculation after each op must match recomputing everything from scratch.
fn check_matches_full_recalc(ops: &[Op]) -> Result<(), TestCaseError> {
    let mut wb = start();
    for (i, op) in ops.iter().enumerate() {
        apply(&mut wb, op);
        let incremental = snapshot(&wb);
        wb.recalc_everything();
        let full = snapshot(&wb);
        prop_assert_eq!(incremental, full, "after op {} ({:?}) the shown values are stale", i, op);
    }
    Ok(())
}

/// Undo everything: back to the start. Redo everything: back to the end.
fn check_undo_redo_round_trip(ops: &[Op]) -> Result<(), TestCaseError> {
    let mut wb = start();
    let original = snapshot(&wb);
    for op in ops {
        apply(&mut wb, op);
    }
    while wb.redo().is_some() {}
    let end = snapshot(&wb);
    while wb.undo().is_some() {}
    prop_assert_eq!(&snapshot(&wb), &original, "undo all did not restore the start");
    while wb.redo().is_some() {}
    prop_assert_eq!(&snapshot(&wb), &end, "redo all did not restore the end state");
    Ok(())
}

fn check_save_open_round_trip(ops: &[Op]) -> Result<(), TestCaseError> {
    let mut wb = start();
    for op in ops {
        apply(&mut wb, op);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("round-trip.xlsx");
    let before = snapshot(&wb);
    io::save(&mut wb, &path, 0).map_err(|e| TestCaseError::fail(e.to_string()))?;
    let back = io::open(&path).map_err(|e| TestCaseError::fail(e.to_string()))?;
    prop_assert_eq!(snapshot(&back), before, "saving and opening changed the workbook");
    Ok(())
}

// When an operation triggers a bug that cannot be fixed right away, switch it off in a copy
// of the test below that is marked #[ignore = "known bug: ..."], keep the main test green, and
// fold it back in with the fix. CI's "Known bugs" step reports which ignored tests pass.
const ALL: Mix = Mix { sort: true, rename: true };

fn config() -> ProptestConfig {
    // Failing cases are stored next to this file and replayed first on every run.
    ProptestConfig {
        failure_persistence: Some(Box::new(prop::test_runner::FileFailurePersistence::WithSource("proptest-regressions"))),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn incremental_recalc_matches_full_recalc(ops in prop::collection::vec(op(ALL), 1..25)) {
        check_matches_full_recalc(&ops)?;
    }

    #[test]
    fn undo_redo_restores_everything(ops in prop::collection::vec(op(ALL), 1..25)) {
        check_undo_redo_round_trip(&ops)?;
    }

    #[test]
    fn save_and_open_keeps_cells(ops in prop::collection::vec(op(ALL), 0..15)) {
        check_save_open_round_trip(&ops)?;
    }
}
