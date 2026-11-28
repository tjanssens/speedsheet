//! The workbook: sheets plus editing operations with undo/redo.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

use crate::filter::{compute_view, contains_ci, Filter};
use crate::formula::parser::{mentions_sheet, rename_sheet_in_formula, shift_formula};
use crate::formula::{self, DepIndex, Pos};
use crate::layout::Area;
use crate::sheet::{Cell, Sheet};
use crate::style::CellStyle;
use crate::value::Value;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileFormat {
    Xlsx,
    Xls,
    Xlsb,
    Ods,
    Csv,
    New,
}

impl FileFormat {
    /// Can we write this format back in place?
    pub fn writable(self) -> bool {
        matches!(self, FileFormat::Xlsx | FileFormat::Csv)
    }
}

/// A defined name ("named range"), like `Rate` = `Sheet1!$B$1`. Workbook-wide names live in
/// [`Workbook::names`], names scoped to one sheet in [`Sheet::names`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DefinedName {
    pub name: String,
    /// The definition, without the leading '='.
    pub formula: String,
    pub hidden: bool,
}

/// The macro project of an .xlsm file, kept as-is so saving does not drop the macros.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct VbaProject {
    pub project: Vec<u8>,
    pub signature: Option<Vec<u8>>,
    /// The workbook's VBA code name (`ThisWorkbook`).
    pub codename: Option<String>,
}

/// A formula text a sheet rename changed: where, before and after.
#[derive(Clone, Debug)]
enum FormulaEdit {
    Cell {
        sheet: u32,
        row: u32,
        col: u32,
        before: Arc<str>,
        after: Arc<str>,
    },
    /// `scope` None = workbook-wide name, Some(sheet) = sheet-local name.
    Name {
        scope: Option<usize>,
        index: usize,
        before: String,
        after: String,
    },
}

enum Action {
    /// Cell changes (row, col, old, new), plus style changes (row, col, old id, new id).
    Cells {
        sheet: usize,
        changes: Vec<(u32, u32, Cell, Cell)>,
        styles: Vec<(u32, u32, u16, u16)>,
    },
    Sort {
        sheet: usize,
        start: usize,
        perm: Vec<u32>,
        /// Formulas the sort broke (a reference pushed off the sheet became #REF!), as
        /// (row within the sorted block before the sort, col, original text). Undo puts them back.
        broken: Vec<(u32, u32, Arc<str>)>,
    },
    AddSheet {
        index: usize,
        sheet: Option<Box<Sheet>>,
    },
    DeleteSheet {
        index: usize,
        sheet: Option<Box<Sheet>>,
    },
    RenameSheet {
        index: usize,
        old: String,
        new: String,
        edits: Vec<FormulaEdit>,
    },
}

const UNDO_LIMIT: usize = 500;

pub struct Workbook {
    pub sheets: Vec<Sheet>,
    pub path: Option<PathBuf>,
    pub format: FileFormat,
    pub csv_delimiter: u8,
    pub modified: bool,
    /// Cell formats; sheets refer to them by index. Index 0 is the default format.
    pub styles: Vec<CellStyle>,
    /// Formats applied by conditional formatting rules.
    pub dxfs: Vec<CellStyle>,
    /// Workbook-wide defined names. Sheet-local ones are in [`Sheet::names`].
    pub names: Vec<DefinedName>,
    /// Macros from an .xlsm file.
    pub vba: Option<VbaProject>,
    /// Parts of the opened file that SpeedSheet cannot keep (charts, data validation, ...):
    /// saving over the original file would remove them. Empty when nothing would be lost.
    pub losses: Vec<String>,
    undo: Vec<Action>,
    redo: Vec<Action>,
    pub(crate) deps: Option<DepIndex>,
}

impl Default for Workbook {
    fn default() -> Self {
        Workbook::new()
    }
}

/// Where the cursor should go after an undo/redo.
#[derive(Clone, Debug, Serialize, Default)]
pub struct Change {
    pub sheet: usize,
    /// Data rows/cols touched (top-left); the UI maps them to view rows.
    pub row: usize,
    pub col: usize,
    pub structure: bool,
    pub recalculated: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Match {
    pub sheet: usize,
    pub row: usize,
    pub view_row: usize,
    pub col: usize,
}

impl Workbook {
    pub fn new() -> Workbook {
        Workbook::from_sheets(vec![Sheet::new("Sheet1")], FileFormat::New)
    }

    pub fn from_sheets(sheets: Vec<Sheet>, format: FileFormat) -> Workbook {
        Workbook::from_sheets_named(sheets, format, Vec::new())
    }

    /// Like [`Workbook::from_sheets`], with workbook-wide defined names. Formulas without a
    /// cached value are computed once the names are known.
    pub fn from_sheets_named(sheets: Vec<Sheet>, format: FileFormat, names: Vec<DefinedName>) -> Workbook {
        let mut wb = Workbook {
            sheets: if sheets.is_empty() { vec![Sheet::new("Sheet1")] } else { sheets },
            path: None,
            format,
            csv_delimiter: b',',
            modified: false,
            styles: vec![CellStyle::default()],
            dxfs: Vec::new(),
            names,
            vba: None,
            losses: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            deps: None,
        };
        wb.calc_missing();
        wb
    }

    /// Formulas without a cached value (e.g. files written by scripts) are computed on load.
    fn calc_missing(&mut self) {
        let mut missing = HashSet::new();
        for (si, s) in self.sheets.iter().enumerate() {
            for (r, row) in s.rows.iter().enumerate() {
                for (c, cell) in row.iter().enumerate() {
                    if let Cell::Formula(f) = cell {
                        if f.value.is_empty() {
                            missing.insert((si as u32, r as u32, c as u32));
                        }
                    }
                }
            }
        }
        if !missing.is_empty() {
            formula::recalc(self, missing);
        }
    }

    /// The definition of a defined name as seen from `sheet`: its sheet-local name if it has
    /// one, else the workbook-wide one. Names compare case-insensitively.
    pub fn name_formula(&self, name: &str, sheet: usize) -> Option<&str> {
        let find = |list: &'_ [DefinedName]| list.iter().position(|n| n.name.eq_ignore_ascii_case(name));
        if let Some(sh) = self.sheets.get(sheet) {
            if let Some(i) = find(&sh.names) {
                return Some(&sh.names[i].formula);
            }
        }
        find(&self.names).map(|i| self.names[i].formula.as_str())
    }

    pub(crate) fn has_names(&self) -> bool {
        !self.names.is_empty() || self.sheets.iter().any(|s| !s.names.is_empty())
    }

    pub fn sheet_index_for(&self, name: Option<&str>, current: usize) -> Option<usize> {
        match name {
            None => Some(current),
            Some(n) => self.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(n)),
        }
    }

    fn sheet_checked(&self, i: usize) -> Result<&Sheet> {
        self.sheets.get(i).ok_or(Error::NoSuchSheet(i))
    }

    fn deps(&mut self) -> &DepIndex {
        if self.deps.is_none() {
            self.deps = Some(DepIndex::build(self));
        }
        self.deps.as_ref().unwrap()
    }

    fn push_undo(&mut self, a: Action) {
        self.undo.push(a);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.modified = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Write raw cells and recalculate dependents. Returns (old cells, recalculated count).
    fn write_cells(&mut self, sheet: usize, cells: Vec<(u32, u32, Cell)>) -> (Vec<(u32, u32, Cell, Cell)>, usize) {
        let mut changes = Vec::with_capacity(cells.len());
        let mut structure_changed = false;
        let mut changed: Vec<Pos> = Vec::with_capacity(cells.len());
        for (r, c, new) in cells {
            let old_ref = self.sheets[sheet].cell(r as usize, c as usize);
            if *old_ref == new {
                continue;
            }
            if matches!(old_ref, Cell::Formula(_)) || matches!(new, Cell::Formula(_)) {
                structure_changed = true;
            }
            let old = self.sheets[sheet].set(r as usize, c as usize, new.clone());
            changes.push((r, c, old, new));
            changed.push((sheet as u32, r, c));
        }
        if structure_changed {
            self.deps = None;
        }
        let n = self.recalc_after(&changed);
        (changes, n)
    }

    fn recalc_after(&mut self, changed: &[Pos]) -> usize {
        if changed.is_empty() {
            return 0;
        }
        let dirty = {
            self.deps();
            let idx = self.deps.as_ref().unwrap();
            if idx.all().is_empty() {
                return 0;
            }
            formula::dirty_set(idx, changed, self)
        };
        formula::recalc(self, dirty)
    }

    /// After rows were reordered: recompute everything except row-local formulas,
    /// plus whatever depends on what was recomputed.
    fn recalc_after_reorder(&mut self, extra: &[Pos]) -> usize {
        self.deps = None;
        self.deps();
        let idx = self.deps.as_ref().unwrap();
        let mut seeds = idx.non_row_local();
        seeds.extend_from_slice(extra);
        if seeds.is_empty() {
            return 0;
        }
        let dirty = formula::dirty_set(idx, &seeds, self);
        formula::recalc(self, dirty)
    }

    /// Recompute every formula from scratch. Tests use it as an oracle: after any operation the
    /// incrementally maintained values must equal what a full recalculation gives.
    pub fn recalc_everything(&mut self) -> usize {
        self.recalc_all()
    }

    fn recalc_all(&mut self) -> usize {
        self.deps = None;
        self.deps();
        let all: HashSet<Pos> = self.deps.as_ref().unwrap().all().iter().copied().collect();
        formula::recalc(self, all)
    }

    /// Set a block of cells from user input strings, starting at a *view* row (filter-aware).
    pub fn set_inputs(&mut self, sheet: usize, view_row: usize, col: usize, grid: &[Vec<String>]) -> Result<Change> {
        let cells: Vec<Vec<Cell>> = grid.iter().map(|row| row.iter().map(|s| crate::input::parse_input(s)).collect()).collect();
        self.set_cells(sheet, view_row, col, cells)
    }

    /// Set a block of parsed cells starting at a view row.
    pub fn set_cells(&mut self, sheet: usize, view_row: usize, col: usize, grid: Vec<Vec<Cell>>) -> Result<Change> {
        let sh = self.sheet_checked(sheet)?;
        let mut flat = Vec::new();
        for (dr, row) in grid.into_iter().enumerate() {
            let Some(r) = sh.data_row(view_row + dr) else { break };
            if r >= crate::MAX_ROWS {
                break;
            }
            for (dc, cell) in row.into_iter().enumerate() {
                if col + dc >= crate::MAX_COLS {
                    break;
                }
                flat.push((r as u32, (col + dc) as u32, cell));
            }
        }
        let first = flat.first().map(|c| (c.0 as usize, c.1 as usize)).unwrap_or((view_row, col));
        let last_row = flat.iter().map(|c| c.0 as usize).max();
        let (changes, n) = self.write_cells(sheet, flat);
        if let Some(r) = last_row {
            self.sheets[sheet].grow_autofilter(r);
        }
        if !changes.is_empty() {
            self.push_undo(Action::Cells { sheet, changes, styles: Vec::new() });
        }
        Ok(Change { sheet, row: first.0, col: first.1, structure: false, recalculated: n })
    }

    /// Clear a rectangle given in view rows (inclusive).
    pub fn clear(&mut self, sheet: usize, vr0: usize, c0: usize, vr1: usize, c1: usize) -> Result<Change> {
        let sh = self.sheet_checked(sheet)?;
        let mut flat = Vec::new();
        let last_row = sh.view_len().min(vr1 + 1);
        for vr in vr0..last_row {
            let Some(r) = sh.data_row(vr) else { break };
            let row_len = sh.rows.get(r).map(|x| x.len()).unwrap_or(0);
            for c in c0..=c1.min(row_len.saturating_sub(1)) {
                if !sh.cell(r, c).is_empty() {
                    flat.push((r as u32, c as u32, Cell::Empty));
                }
            }
        }
        let (changes, n) = self.write_cells(sheet, flat);
        if !changes.is_empty() {
            self.push_undo(Action::Cells { sheet, changes, styles: Vec::new() });
        }
        Ok(Change { sheet, row: vr0, col: c0, structure: false, recalculated: n })
    }

    /// Fill handle: extend `src` (view rows, inclusive) to cover `dst`, which contains `src` and
    /// grows it in one direction. Series continue, formulas shift, formats are copied.
    pub fn fill(&mut self, sheet: usize, src: (usize, usize, usize, usize), dst: (usize, usize, usize, usize)) -> Result<Change> {
        let sh = self.sheet_checked(sheet)?;
        let (sr0, sc0, sr1, sc1) = src;
        let (dr0, dc0, dr1, dc1) = dst;
        let vertical = dr0 < sr0 || dr1 > sr1;
        let mut flat: Vec<(u32, u32, Cell)> = Vec::new();
        let mut styles: Vec<(u32, u32, u16, u16)> = Vec::new();
        let data = |vr: usize| sh.data_row(vr).filter(|&r| r < crate::MAX_ROWS);
        if vertical {
            let src_rows: Vec<usize> = (sr0..=sr1).filter_map(data).collect();
            let targets: Vec<(i64, usize)> =
                (dr0..sr0).chain(sr1 + 1..=dr1).filter_map(|vr| data(vr).map(|r| (vr as i64 - sr0 as i64, r))).collect();
            if src_rows.is_empty() || targets.is_empty() {
                return Ok(Change { sheet, row: sr0, col: sc0, ..Default::default() });
            }
            let n = src_rows.len();
            for c in sc0..=sc1.min(crate::MAX_COLS - 1) {
                let cells: Vec<&Cell> = src_rows.iter().map(|&r| sh.cell(r, c)).collect();
                let delta = |k: i64, i: usize| -> (i64, i64) {
                    let tr = targets.iter().find(|t| t.0 == k).map(|t| t.1).unwrap_or(0);
                    (tr as i64 - src_rows[i] as i64, 0)
                };
                let out = crate::fill::fill_line(&cells, targets.iter().map(|t| t.0), &delta);
                for ((k, r), cell) in targets.iter().zip(out) {
                    flat.push((*r as u32, c as u32, cell));
                    let sid = sh.style_id(src_rows[k.rem_euclid(n as i64) as usize], c);
                    let old = sh.style_id(*r, c);
                    if old != sid {
                        styles.push((*r as u32, c as u32, old, sid));
                    }
                }
            }
        } else {
            let src_cols: Vec<usize> = (sc0..=sc1).collect();
            let targets: Vec<(i64, usize)> =
                (dc0..sc0).chain(sc1 + 1..=dc1.min(crate::MAX_COLS - 1)).map(|c| (c as i64 - sc0 as i64, c)).collect();
            if targets.is_empty() {
                return Ok(Change { sheet, row: sr0, col: sc0, ..Default::default() });
            }
            let n = src_cols.len();
            for vr in sr0..=sr1 {
                let Some(r) = data(vr) else { continue };
                let cells: Vec<&Cell> = src_cols.iter().map(|&c| sh.cell(r, c)).collect();
                let delta = |k: i64, i: usize| -> (i64, i64) { (0, k - i as i64) };
                let out = crate::fill::fill_line(&cells, targets.iter().map(|t| t.0), &delta);
                for ((k, c), cell) in targets.iter().zip(out) {
                    flat.push((r as u32, *c as u32, cell));
                    let sid = sh.style_id(r, src_cols[k.rem_euclid(n as i64) as usize]);
                    let old = sh.style_id(r, *c);
                    if old != sid {
                        styles.push((r as u32, *c as u32, old, sid));
                    }
                }
            }
        }
        let sh = &mut self.sheets[sheet];
        for (r, c, _, new) in &styles {
            sh.set_style_id(*r as usize, *c as usize, *new);
        }
        let (changes, n) = self.write_cells(sheet, flat);
        if !changes.is_empty() || !styles.is_empty() {
            self.push_undo(Action::Cells { sheet, changes, styles });
            self.modified = true;
        }
        Ok(Change { sheet, row: dr0, col: dc0, structure: false, recalculated: n })
    }

    /// Double-click on the fill handle: fill `src` down as far as the column next to it has data.
    pub fn fill_down_auto(&mut self, sheet: usize, src: (usize, usize, usize, usize)) -> Result<Option<(usize, Change)>> {
        let sh = self.sheet_checked(sheet)?;
        let (sr0, sc0, sr1, sc1) = src;
        let filled = |c: usize, vr: usize| sh.data_row(vr).is_some_and(|r| !sh.cell(r, c).is_empty());
        let len = sh.view_len();
        let run = |c: usize| (sr1 + 1..len).take_while(|&vr| filled(c, vr)).count();
        let mut n = if sc0 > 0 { run(sc0 - 1) } else { 0 };
        if n == 0 {
            n = run(sc1 + 1);
        }
        if n == 0 {
            return Ok(None);
        }
        let end = sr1 + n;
        let ch = self.fill(sheet, src, (sr0, sc0, end, sc1))?;
        Ok(Some((end, ch)))
    }

    /// Sort data rows by one column. Formulas move with their row and their relative
    /// references are shifted, like Excel. Empty cells always go last.
    pub fn sort(&mut self, sheet: usize, col: usize, ascending: bool, header: bool) -> Result<Change> {
        let sh = self.sheet_checked(sheet)?;
        let start = sh.data_start(header);
        let n = sh.data_end().saturating_sub(start);
        if n < 2 {
            return Ok(Change { sheet, ..Default::default() });
        }
        let keys: Vec<&Value> = (start..start + n).map(|r| sh.value(r, col)).collect();
        let mut perm: Vec<u32> = (0..n as u32).collect();
        perm.sort_by(|&a, &b| {
            let (va, vb) = (keys[a as usize], keys[b as usize]);
            match (va.is_empty(), vb.is_empty()) {
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => {
                    let o = va.sort_cmp(vb);
                    if ascending {
                        o
                    } else {
                        o.reverse()
                    }
                }
            }
        });
        if perm.iter().enumerate().all(|(i, &p)| i as u32 == p) {
            return Ok(Change { sheet, ..Default::default() });
        }
        let broken = self.apply_perm(sheet, start, &perm);
        // A formula that turned into #REF! may have no references left: recompute it explicitly.
        let seeds: Vec<Pos> = broken.iter().map(|b| (sheet as u32, (start + b.1 as usize) as u32, b.2)).collect();
        let broken: Vec<(u32, u32, Arc<str>)> = broken.into_iter().map(|(p, _, c, src)| (p, c, src)).collect();
        let recalculated = self.recalc_after_reorder(&seeds);
        self.refresh_filter(sheet);
        self.push_undo(Action::Sort { sheet, start, perm, broken });
        Ok(Change { sheet, row: 0, col, structure: true, recalculated })
    }

    /// rows[start + i] = old_rows[start + perm[i]]. Returns the formulas whose shifted
    /// references fell off the sheet, as (p, i, col, original text), so undo can restore them.
    fn apply_perm(&mut self, sheet: usize, start: usize, perm: &[u32]) -> Vec<(u32, u32, u32, Arc<str>)> {
        let mut broken = Vec::new();
        let sh = &mut self.sheets[sheet];
        // Trailing empty rows are trimmed by later edits; undoing a sort after that must still
        // find its rows (found by tests/invariants.rs: sort, fill, undo, undo panicked).
        sh.rows.resize_with(sh.rows.len().max(start + perm.len()), Vec::new);
        let mut old: Vec<Option<Vec<Cell>>> =
            sh.rows[start..start + perm.len()].iter_mut().map(|r| Some(std::mem::take(r))).collect();
        for (i, &p) in perm.iter().enumerate() {
            let mut row = old[p as usize].take().unwrap_or_default();
            let dr = i as i64 - p as i64;
            if dr != 0 {
                for (c, cell) in row.iter_mut().enumerate() {
                    if let Cell::Formula(f) = cell {
                        let shifted = shift_formula(&f.src, dr, 0);
                        // Shifting is exact unless a reference left the sheet: keep the original.
                        if shifted.matches("#REF!").count() > f.src.matches("#REF!").count() {
                            broken.push((p, i as u32, c as u32, f.src.clone()));
                        }
                        f.src = shifted.into();
                    }
                }
            }
            sh.rows[start + i] = row;
        }
        while sh.rows.last().is_some_and(|r| r.is_empty()) {
            sh.rows.pop();
        }
        // Cell formatting moves with the rows, like in Excel.
        if sh.styles.len() > start {
            if sh.styles.len() < start + perm.len() {
                sh.styles.resize_with(start + perm.len(), Vec::new);
            }
            let mut old: Vec<Vec<u16>> = sh.styles[start..start + perm.len()].iter_mut().map(std::mem::take).collect();
            for (i, &p) in perm.iter().enumerate() {
                sh.styles[start + i] = std::mem::take(&mut old[p as usize]);
            }
            while sh.styles.last().is_some_and(|r| r.is_empty()) {
                sh.styles.pop();
            }
        }
        self.deps = None;
        broken
    }

    fn invert(perm: &[u32]) -> Vec<u32> {
        let mut inv = vec![0u32; perm.len()];
        for (i, &p) in perm.iter().enumerate() {
            inv[p as usize] = i as u32;
        }
        inv
    }

    pub fn set_filter(&mut self, sheet: usize, filter: Option<Filter>) -> Result<usize> {
        self.sheet_checked(sheet)?;
        let sh = &mut self.sheets[sheet];
        match filter {
            Some(f) if !f.conds.is_empty() => {
                sh.view = Some(compute_view(sh, &f));
                sh.filter = Some(f);
            }
            _ => {
                sh.filter = None;
                sh.view = None;
            }
        }
        Ok(sh.view_len())
    }

    /// Turn the filter buttons on for `area` (its first row holds the headers) or off. Turning
    /// them off also clears any active filter, like Excel's Data > Filter.
    pub fn set_autofilter(&mut self, sheet: usize, area: Option<Area>) -> Result<()> {
        self.sheet_checked(sheet)?;
        let sh = &mut self.sheets[sheet];
        sh.filter = None;
        sh.view = None;
        sh.autofilter = area;
        self.modified = true;
        Ok(())
    }

    /// Re-apply the current filter (after sort/undo).
    pub fn refresh_filter(&mut self, sheet: usize) {
        let sh = &mut self.sheets[sheet];
        if let Some(f) = &sh.filter {
            sh.view = Some(compute_view(sh, f));
        }
    }

    pub fn add_sheet(&mut self, name: Option<String>) -> usize {
        let name = name.unwrap_or_else(|| self.unique_sheet_name("Sheet"));
        let index = self.sheets.len();
        self.sheets.push(Sheet::new(name));
        self.deps = None;
        // Formulas that referred to a sheet of this name (deleted or never there) now resolve.
        self.recalc_all();
        self.push_undo(Action::AddSheet { index, sheet: None });
        index
    }

    fn unique_sheet_name(&self, base: &str) -> String {
        (1..).map(|i| format!("{base}{i}")).find(|n| self.sheet_index_for(Some(n), 0).is_none()).unwrap()
    }

    pub fn rename_sheet(&mut self, index: usize, name: String) -> Result<()> {
        self.sheet_checked(index)?;
        let name = name.trim().to_string();
        if name.is_empty() || name.len() > 31 || name.contains(['[', ']', ':', '*', '?', '/', '\\']) {
            return Err(Error::InvalidName(name));
        }
        if self.sheets.iter().enumerate().any(|(i, s)| i != index && s.name.eq_ignore_ascii_case(&name)) {
            return Err(Error::InvalidName(name));
        }
        let old = self.sheets[index].name.clone();
        if old == name {
            return Ok(());
        }
        // Like Excel, formulas and names that point at the sheet follow it to its new name.
        let edits = self.rewrite_sheet_refs(&old, &name);
        self.sheets[index].name = name.clone();
        self.recalc_after_rename(&old, &name, &edits);
        self.push_undo(Action::RenameSheet { index, old, new: name, edits });
        Ok(())
    }

    /// Rewrite references to sheet `from` so they say `to`. Returns what changed.
    fn rewrite_sheet_refs(&mut self, from: &str, to: &str) -> Vec<FormulaEdit> {
        let mut edits = Vec::new();
        for (si, sh) in self.sheets.iter_mut().enumerate() {
            for (r, row) in sh.rows.iter_mut().enumerate() {
                for (c, cell) in row.iter_mut().enumerate() {
                    let Cell::Formula(f) = cell else { continue };
                    if let Some(new) = rename_sheet_in_formula(&f.src, from, to) {
                        let after: Arc<str> = new.into();
                        let before = std::mem::replace(&mut f.src, after.clone());
                        edits.push(FormulaEdit::Cell { sheet: si as u32, row: r as u32, col: c as u32, before, after });
                    }
                }
            }
            for (i, n) in sh.names.iter_mut().enumerate() {
                if let Some(new) = rename_sheet_in_formula(&n.formula, from, to) {
                    let before = std::mem::replace(&mut n.formula, new.clone());
                    edits.push(FormulaEdit::Name { scope: Some(si), index: i, before, after: new });
                }
            }
        }
        for (i, n) in self.names.iter_mut().enumerate() {
            if let Some(new) = rename_sheet_in_formula(&n.formula, from, to) {
                let before = std::mem::replace(&mut n.formula, new.clone());
                edits.push(FormulaEdit::Name { scope: None, index: i, before, after: new });
            }
        }
        edits
    }

    /// Put recorded formula texts back (undo) or apply them again (redo).
    fn apply_formula_edits(&mut self, edits: &[FormulaEdit], undo: bool) {
        for e in edits {
            match e {
                FormulaEdit::Cell { sheet, row, col, before, after } => {
                    let sh = &mut self.sheets[*sheet as usize];
                    if let Some(Cell::Formula(f)) = sh.cell_mut(*row as usize, *col as usize) {
                        f.src = if undo { before.clone() } else { after.clone() };
                    }
                }
                FormulaEdit::Name { scope, index, before, after } => {
                    let list = match scope {
                        Some(s) => &mut self.sheets[*s].names,
                        None => &mut self.names,
                    };
                    if let Some(n) = list.get_mut(*index) {
                        n.formula = if undo { before.clone() } else { after.clone() };
                    }
                }
            }
        }
    }

    /// After a sheet was renamed from `a` to `b` (or back): recompute every formula that names
    /// either sheet (a reference to a missing sheet may now resolve, or stop resolving).
    fn recalc_after_rename(&mut self, a: &str, b: &str, edits: &[FormulaEdit]) -> usize {
        self.deps = None;
        if self.has_names() {
            return self.recalc_all();
        }
        let mut seeds: Vec<Pos> = edits
            .iter()
            .filter_map(|e| match e {
                FormulaEdit::Cell { sheet, row, col, .. } => Some((*sheet, *row, *col)),
                FormulaEdit::Name { .. } => None,
            })
            .collect();
        for (si, sh) in self.sheets.iter().enumerate() {
            for (r, row) in sh.rows.iter().enumerate() {
                for (c, cell) in row.iter().enumerate() {
                    if let Cell::Formula(f) = cell {
                        if mentions_sheet(&f.src, &[a, b]) {
                            seeds.push((si as u32, r as u32, c as u32));
                        }
                    }
                }
            }
        }
        seeds.sort_unstable();
        seeds.dedup();
        self.recalc_after(&seeds)
    }

    pub fn delete_sheet(&mut self, index: usize) -> Result<()> {
        self.sheet_checked(index)?;
        if self.sheets.len() == 1 {
            return Err(Error::LastSheet);
        }
        let sheet = self.sheets.remove(index);
        self.deps = None;
        self.recalc_all();
        self.push_undo(Action::DeleteSheet { index, sheet: Some(Box::new(sheet)) });
        Ok(())
    }

    pub fn undo(&mut self) -> Option<Change> {
        let a = self.undo.pop()?;
        let (inverse, change) = self.apply_action(a, true);
        self.redo.push(inverse);
        self.modified = true;
        Some(change)
    }

    pub fn redo(&mut self) -> Option<Change> {
        let a = self.redo.pop()?;
        let (inverse, change) = self.apply_action(a, false);
        self.undo.push(inverse);
        self.modified = true;
        Some(change)
    }

    /// Apply an action in reverse (undo=true) or forward, returning its mirror for the other stack.
    fn apply_action(&mut self, a: Action, undo: bool) -> (Action, Change) {
        match a {
            Action::Cells { sheet, changes, styles } => {
                let cells =
                    changes.iter().map(|(r, c, old, new)| (*r, *c, if undo { old.clone() } else { new.clone() })).collect();
                let first = changes.first().or(None).map(|x| (x.0 as usize, x.1 as usize));
                let first = first.or(styles.first().map(|x| (x.0 as usize, x.1 as usize))).unwrap_or_default();
                for (r, c, old, new) in &styles {
                    self.sheets[sheet].set_style_id(*r as usize, *c as usize, if undo { *old } else { *new });
                }
                let (_, n) = self.write_cells(sheet, cells);
                (
                    Action::Cells { sheet, changes, styles },
                    Change { sheet, row: first.0, col: first.1, structure: false, recalculated: n },
                )
            }
            Action::Sort { sheet, start, perm, broken } => {
                let seeds: Vec<Pos> = if undo {
                    self.apply_perm(sheet, start, &Self::invert(&perm));
                    // Back in their original rows: give broken formulas their original text.
                    let sh = &mut self.sheets[sheet];
                    for (p, c, src) in &broken {
                        if let Some(Cell::Formula(f)) = sh.cell_mut(start + *p as usize, *c as usize) {
                            f.src = src.clone();
                        }
                    }
                    broken.iter().map(|(p, c, _)| (sheet as u32, (start + *p as usize) as u32, *c)).collect()
                } else {
                    let again = self.apply_perm(sheet, start, &perm);
                    again.iter().map(|b| (sheet as u32, (start + b.1 as usize) as u32, b.2)).collect()
                };
                let n = self.recalc_after_reorder(&seeds);
                self.refresh_filter(sheet);
                (Action::Sort { sheet, start, perm, broken }, Change { sheet, row: 0, col: 0, structure: true, recalculated: n })
            }
            Action::AddSheet { index, sheet } => {
                let (sheet, change) = self.toggle_sheet(index, sheet);
                (Action::AddSheet { index, sheet }, change)
            }
            Action::DeleteSheet { index, sheet } => {
                let (sheet, change) = self.toggle_sheet(index, sheet);
                (Action::DeleteSheet { index, sheet }, change)
            }
            Action::RenameSheet { index, old, new, edits } => {
                self.apply_formula_edits(&edits, undo);
                self.sheets[index].name = if undo { old.clone() } else { new.clone() };
                let n = self.recalc_after_rename(&old, &new, &edits);
                (
                    Action::RenameSheet { index, old, new, edits },
                    Change { sheet: index, structure: true, recalculated: n, ..Default::default() },
                )
            }
        }
    }

    /// None: remove the sheet at index and return it. Some: insert it back.
    fn toggle_sheet(&mut self, index: usize, sheet: Option<Box<Sheet>>) -> (Option<Box<Sheet>>, Change) {
        let (out, focus) = match sheet {
            None => (Some(Box::new(self.sheets.remove(index))), index.saturating_sub(1)),
            Some(s) => {
                self.sheets.insert(index, *s);
                (None, index)
            }
        };
        self.deps = None;
        let n = self.recalc_all();
        (out, Change { sheet: focus, structure: true, recalculated: n, ..Default::default() })
    }

    /// Find the next cell whose displayed value (or formula) contains `query`.
    /// Starts after (view_row, col) on `sheet`, wraps around and continues on other sheets.
    pub fn find(
        &self,
        query: &str,
        sheet: usize,
        view_row: usize,
        col: usize,
        backwards: bool,
        all_sheets: bool,
    ) -> Option<Match> {
        if query.is_empty() {
            return None;
        }
        let needle = query.to_lowercase();
        // Numbers and dates only display digits and a few symbols; skip formatting them otherwise.
        let numeric_possible = needle.chars().all(|c| c.is_ascii_digit() || "-+.:e ".contains(c));
        let matches = |c: &Cell| -> bool {
            match c {
                Cell::Empty => false,
                Cell::Value(Value::Text(t)) => contains_ci(t, &needle),
                Cell::Value(Value::Number(_) | Value::Date(_)) if !numeric_possible => false,
                Cell::Value(v) => contains_ci(&v.display(), &needle),
                Cell::Formula(f) => {
                    contains_ci(&f.src, &needle)
                        || match &f.value {
                            Value::Number(_) | Value::Date(_) if !numeric_possible => false,
                            v => contains_ci(&v.display(), &needle),
                        }
                }
            }
        };
        let n_sheets = self.sheets.len();
        let order: Vec<usize> = if all_sheets {
            (0..=n_sheets)
                .map(|i| if backwards { (sheet + n_sheets * 2 - i) % n_sheets } else { (sheet + i) % n_sheets })
                .collect()
        } else {
            vec![sheet, sheet]
        };
        for (k, &si) in order.iter().enumerate() {
            let sh = &self.sheets[si];
            let len = sh.view_len();
            let first = k == 0;
            let rows: Box<dyn Iterator<Item = usize>> = if backwards {
                let top = if first { view_row.min(len.saturating_sub(1)) } else { len.saturating_sub(1) };
                if len == 0 {
                    Box::new(std::iter::empty())
                } else {
                    Box::new((0..=top).rev())
                }
            } else {
                Box::new((if first { view_row } else { 0 })..len)
            };
            for vr in rows {
                let Some(r) = sh.data_row(vr) else { continue };
                let Some(row) = sh.rows.get(r) else { continue };
                let cols: Box<dyn Iterator<Item = usize>> = if backwards {
                    let hi = if first && vr == view_row { col.min(row.len()) } else { row.len() };
                    Box::new((0..hi).rev())
                } else {
                    let lo = if first && vr == view_row { col + 1 } else { 0 };
                    Box::new(lo..row.len())
                };
                for c in cols {
                    if matches(&row[c]) {
                        return Some(Match { sheet: si, row: r, view_row: vr, col: c });
                    }
                }
            }
        }
        None
    }

    /// Evaluate a formula against this workbook without storing it.
    pub fn evaluate(&self, sheet: usize, src: &str) -> Value {
        formula::evaluate_standalone(self, sheet, src.strip_prefix('=').unwrap_or(src))
    }
}
