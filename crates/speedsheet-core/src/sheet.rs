//! Sheet storage: row-major, sparse at the end of each row.

use std::sync::Arc;

use crate::filter::Filter;
use crate::layout::{Area, CondFormat};
use crate::value::Value;

/// A formula cell: source text (without the leading '=') and its last computed value.
#[derive(Clone, Debug, PartialEq)]
pub struct FormulaCell {
    pub src: Arc<str>,
    pub value: Value,
}

/// What is stored in a cell.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Cell {
    #[default]
    Empty,
    Value(Value),
    Formula(Box<FormulaCell>),
}

impl Cell {
    pub fn value(&self) -> &Value {
        static EMPTY: Value = Value::Empty;
        match self {
            Cell::Empty => &EMPTY,
            Cell::Value(v) => v,
            Cell::Formula(f) => &f.value,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Cell::Empty => true,
            Cell::Value(v) => v.is_empty(),
            Cell::Formula(_) => false,
        }
    }

    pub fn from_value(v: Value) -> Cell {
        if v.is_empty() {
            Cell::Empty
        } else {
            Cell::Value(v)
        }
    }

    pub fn formula(src: &str, value: Value) -> Cell {
        Cell::Formula(Box::new(FormulaCell { src: Arc::from(src), value }))
    }

    /// The text a user edits: "=SUM(A1:A3)" for formulas, the plain value otherwise.
    pub fn input_text(&self) -> String {
        match self {
            Cell::Empty => String::new(),
            Cell::Formula(f) => format!("={}", f.src),
            Cell::Value(Value::Number(n)) => {
                // Full precision for editing.
                let s = format!("{}", n);
                if s.ends_with(".0") {
                    s[..s.len() - 2].to_string()
                } else {
                    s
                }
            }
            Cell::Value(Value::Text(t)) => {
                // Protect text that would otherwise be parsed as a number/formula.
                if crate::input::would_parse_as_non_text(t) {
                    format!("'{}", t)
                } else {
                    t.to_string()
                }
            }
            Cell::Value(v) => v.display(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<Cell>>,
    /// Column widths in pixels (None = default). Kept sparse.
    pub col_widths: Vec<Option<f32>>,
    /// Active filter (non-destructive). When set, `view` maps visible rows to data rows.
    pub filter: Option<Filter>,
    pub view: Option<Vec<u32>>,
    /// Style id per cell (index into `Workbook::styles`, 0 = default). Rows/cols beyond the end are 0.
    pub styles: Vec<Vec<u16>>,
    /// Row heights in pixels (None = default, 0 = hidden). Per data row.
    pub row_heights: Vec<Option<f32>>,
    /// Defaults from the file, in pixels (None = the app's defaults).
    pub default_col_width: Option<f32>,
    pub default_row_height: Option<f32>,
    pub merges: Vec<Area>,
    /// Frozen (rows, cols) at the top/left.
    pub freeze: Option<(u32, u32)>,
    /// The file's autofilter range; its first row holds the filter buttons.
    pub autofilter: Option<Area>,
    pub cond: Vec<CondFormat>,
    pub hide_grid: bool,
    cols_cache: usize,
}

impl Sheet {
    pub fn new(name: impl Into<String>) -> Sheet {
        Sheet { name: name.into(), ..Default::default() }
    }

    pub fn from_rows(name: impl Into<String>, mut rows: Vec<Vec<Cell>>) -> Sheet {
        for r in rows.iter_mut() {
            trim_row(r);
        }
        while rows.last().is_some_and(|r| r.is_empty()) {
            rows.pop();
        }
        let mut s = Sheet { name: name.into(), rows, ..Default::default() };
        s.recount_cols();
        s
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn col_count(&self) -> usize {
        self.cols_cache
    }

    pub fn recount_cols(&mut self) {
        self.cols_cache = self.rows.iter().map(|r| r.len()).max().unwrap_or(0);
    }

    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        static EMPTY: Cell = Cell::Empty;
        self.rows.get(row).and_then(|r| r.get(col)).unwrap_or(&EMPTY)
    }

    pub fn value(&self, row: usize, col: usize) -> &Value {
        self.cell(row, col).value()
    }

    /// Set a cell, returning the previous content.
    pub fn set(&mut self, row: usize, col: usize, cell: Cell) -> Cell {
        if cell.is_empty() {
            let Some(r) = self.rows.get_mut(row) else { return Cell::Empty };
            let Some(slot) = r.get_mut(col) else { return Cell::Empty };
            let old = std::mem::take(slot);
            if col + 1 == r.len() {
                trim_row(r);
                if row + 1 == self.rows.len() {
                    while self.rows.last().is_some_and(|r| r.is_empty()) {
                        self.rows.pop();
                    }
                }
                if col + 1 == self.cols_cache {
                    self.recount_cols();
                }
            }
            return old;
        }
        if self.rows.len() <= row {
            self.rows.resize_with(row + 1, Vec::new);
        }
        let r = &mut self.rows[row];
        if r.len() <= col {
            r.resize_with(col + 1, Cell::default);
        }
        self.cols_cache = self.cols_cache.max(col + 1);
        std::mem::replace(&mut r[col], cell)
    }

    /// Mutable access to a formula cell's slot, if present.
    pub fn cell_mut(&mut self, row: usize, col: usize) -> Option<&mut Cell> {
        self.rows.get_mut(row).and_then(|r| r.get_mut(col))
    }

    /// Number of rows visible in the current view.
    pub fn view_len(&self) -> usize {
        match &self.view {
            Some(v) => v.len(),
            None => self.rows.len(),
        }
    }

    /// Map a view row to a data row. Rows beyond the data are mapped 1:1 when unfiltered.
    pub fn data_row(&self, view_row: usize) -> Option<usize> {
        match &self.view {
            Some(v) => v.get(view_row).map(|&r| r as usize),
            None => Some(view_row),
        }
    }

    /// Map a data row to its view row, if visible.
    pub fn view_row(&self, data_row: usize) -> Option<usize> {
        match &self.view {
            Some(v) => v.binary_search(&(data_row as u32)).ok(),
            None => Some(data_row),
        }
    }

    pub fn col_width(&self, col: usize) -> Option<f32> {
        self.col_widths.get(col).copied().flatten()
    }

    pub fn set_col_width(&mut self, col: usize, width: Option<f32>) {
        if self.col_widths.len() <= col {
            self.col_widths.resize(col + 1, None);
        }
        self.col_widths[col] = width;
    }

    pub fn row_height(&self, row: usize) -> Option<f32> {
        self.row_heights.get(row).copied().flatten()
    }

    pub fn set_row_height(&mut self, row: usize, height: Option<f32>) {
        if self.row_heights.len() <= row {
            if height.is_none() {
                return;
            }
            self.row_heights.resize(row + 1, None);
        }
        self.row_heights[row] = height;
    }

    pub fn style_id(&self, row: usize, col: usize) -> u16 {
        self.styles.get(row).and_then(|r| r.get(col)).copied().unwrap_or(0)
    }

    pub fn set_style_id(&mut self, row: usize, col: usize, id: u16) -> u16 {
        if self.style_id(row, col) == id {
            return id;
        }
        if self.styles.len() <= row {
            self.styles.resize_with(row + 1, Vec::new);
        }
        let r = &mut self.styles[row];
        if r.len() <= col {
            r.resize(col + 1, 0);
        }
        std::mem::replace(&mut r[col], id)
    }

    /// Rows and columns that have content or formatting.
    pub fn extent(&self) -> (usize, usize) {
        let rows = self.rows.len().max(self.styles.len());
        let cols = self.cols_cache.max(self.styles.iter().map(|r| r.len()).max().unwrap_or(0));
        (rows, cols)
    }

    /// First data row below the header: the autofilter's header row from the file, else row 0
    /// when `header` is set.
    pub fn data_start(&self, header: bool) -> usize {
        match self.autofilter {
            Some(a) => a.r0 as usize + 1,
            None => header as usize,
        }
    }

    /// One past the last data row the filter and sort apply to: the end of the filter range from
    /// the file or the Filter button, so a totals line under the list isn't filtered or sorted
    /// away. Without a range, all rows.
    pub fn data_end(&self) -> usize {
        let n = self.row_count();
        match self.autofilter {
            Some(a) => (a.r1 as usize + 1).min(n),
            None => n,
        }
    }

    /// Rows typed straight under the filter range join it, like in Excel.
    pub fn grow_autofilter(&mut self, last_row: usize) {
        if let Some(a) = self.autofilter.as_mut() {
            let mut r1 = a.r1 as usize;
            while r1 < last_row && r1 + 1 < self.rows.len() && !self.rows[r1 + 1].iter().all(|c| c.is_empty()) {
                r1 += 1;
            }
            a.r1 = r1 as u32;
        }
    }

    pub fn row_is_blank(&self, row: usize) -> bool {
        self.rows.get(row).is_none_or(|r| r.iter().all(|c| c.is_empty()))
    }

    /// The block of filled cells around (row, col), bounded by blank rows and columns, like
    /// Excel's "current region". Used to place a new filter range.
    pub fn current_region(&self, row: usize, col: usize) -> Area {
        let filled = |r: usize, c: usize| !self.cell(r, c).is_empty();
        let (mut r0, mut r1, mut c0, mut c1) = (row, row, col, col);
        loop {
            let mut grown = false;
            let row_has = |r: usize, c0: usize, c1: usize| (c0.saturating_sub(1)..=c1 + 1).any(|c| filled(r, c));
            let col_has = |c: usize, r0: usize, r1: usize| (r0.saturating_sub(1)..=r1 + 1).any(|r| filled(r, c));
            if r0 > 0 && row_has(r0 - 1, c0, c1) {
                r0 -= 1;
                grown = true;
            }
            if r1 + 1 < self.row_count() && row_has(r1 + 1, c0, c1) {
                r1 += 1;
                grown = true;
            }
            if c0 > 0 && col_has(c0 - 1, r0, r1) {
                c0 -= 1;
                grown = true;
            }
            if c1 + 1 < self.col_count() && col_has(c1 + 1, r0, r1) {
                c1 += 1;
                grown = true;
            }
            if !grown {
                break;
            }
        }
        Area { r0: r0 as u32, c0: c0 as u32, r1: r1 as u32, c1: c1 as u32 }
    }

    /// The merged range whose top-left cell is (row, col), or that covers it.
    pub fn merge_at(&self, row: u32, col: u32) -> Option<&Area> {
        self.merges.iter().find(|m| m.contains(row, col))
    }
}

fn trim_row(r: &mut Vec<Cell>) {
    while r.last().is_some_and(|c| c.is_empty()) {
        r.pop();
    }
}

/// Column index to letters: 0 -> A, 25 -> Z, 26 -> AA.
pub fn col_name(mut col: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

/// Letters to column index: A -> 0. Returns None for invalid or out-of-range input.
pub fn col_index(letters: &str) -> Option<usize> {
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let mut n: usize = 0;
    for b in letters.bytes() {
        let b = b.to_ascii_uppercase();
        if !b.is_ascii_uppercase() {
            return None;
        }
        n = n * 26 + (b - b'A' + 1) as usize;
    }
    if n > crate::MAX_COLS {
        return None;
    }
    Some(n - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_names() {
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
        assert_eq!(col_name(701), "ZZ");
        assert_eq!(col_name(702), "AAA");
        for i in [0, 5, 26, 700, 16383] {
            assert_eq!(col_index(&col_name(i)), Some(i));
        }
    }

    #[test]
    fn set_and_trim() {
        let mut s = Sheet::new("S");
        s.set(4, 3, Cell::Value(Value::Number(1.0)));
        assert_eq!(s.row_count(), 5);
        assert_eq!(s.col_count(), 4);
        s.set(4, 3, Cell::Empty);
        assert_eq!(s.row_count(), 0);
        assert_eq!(s.col_count(), 0);
    }
}
