//! Moving the cursor by the data: Ctrl+Arrow jumps like Excel.

use crate::{Sheet, MAX_COLS, MAX_ROWS};

/// Excel's Ctrl+Arrow from (`view_row`, `col`) one way along a row or column (`dr`/`dc` is -1,
/// 0 or 1). Inside a run of filled cells it stops at the run's last cell; otherwise it goes to
/// the next filled cell; with nothing further it goes to the edge of the sheet. Rows are view
/// rows, so a filtered sheet jumps over the rows the filter hides.
pub fn jump(sheet: &Sheet, view_row: usize, col: usize, dr: i32, dc: i32) -> (usize, usize) {
    if dr != 0 {
        // A filtered view ends at its last row; an unfiltered sheet at Excel's last row.
        let last = if sheet.view.is_some() { sheet.view_len().saturating_sub(1) } else { MAX_ROWS - 1 };
        let filled = |r: usize| sheet.data_row(r).is_some_and(|d| !sheet.cell(d, col).is_empty());
        let used = sheet.view_len();
        (step(view_row.min(last), dr.is_positive(), last, used, filled), col)
    } else if dc != 0 {
        let filled = |c: usize| sheet.data_row(view_row).is_some_and(|d| !sheet.cell(d, c).is_empty());
        let last = MAX_COLS - 1;
        (view_row, step(col.min(last), dc.is_positive(), last, sheet.col_count(), filled))
    } else {
        (view_row, col)
    }
}

/// One Ctrl+Arrow along a line of cells `0..=last`, of which only `0..used` can be filled
/// (`used <= last + 1`).
fn step(at: usize, forward: bool, last: usize, used: usize, filled: impl Fn(usize) -> bool) -> usize {
    let next = |i: usize| if forward { (i != last).then(|| i.saturating_add(1)) } else { i.checked_sub(1) };
    let Some(first) = next(at) else { return at };
    if filled(at) && filled(first) {
        // Inside a run: go to its end.
        let mut i = first;
        while let Some(n) = next(i).filter(|&n| filled(n)) {
            i = n;
        }
        return i;
    }
    // To the next filled cell, or to the edge. Past `used` nothing is filled.
    if forward {
        (first..used).find(|&i| filled(i)).unwrap_or(last)
    } else {
        (0..=first).rev().find(|&i| filled(i)).unwrap_or(0)
    }
}
