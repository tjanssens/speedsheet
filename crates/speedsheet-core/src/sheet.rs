//! Sheet storage: row-major, sparse at the end of each row.

use std::sync::Arc;

use crate::filter::Filter;
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
