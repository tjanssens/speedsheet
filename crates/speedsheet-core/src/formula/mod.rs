//! Formula engine: parsing, evaluation and incremental recalculation.

pub mod eval;
pub mod parser;

use std::collections::{HashMap, HashSet};

use eval::{Context, Evaluator, Rng};
use parser::{collect_refs, has_volatile, parse};

use crate::sheet::Cell;
use crate::value::{CellError, Value};
use crate::workbook::Workbook;

/// (sheet, row, col)
pub type Pos = (u32, u32, u32);

const SHORT_ROWS: u32 = 1024;
const SHORT_COLS: u32 = 64;
const MAX_DEPTH: usize = 20_000;

/// (r0, r1, formula index)
type RowSpan = (u32, u32, u32);

/// Reverse dependency index: which formulas read a given cell.
#[derive(Default)]
pub(crate) struct DepIndex {
    formulas: Vec<Pos>,
    /// (sheet, col) -> (r0, r1, formula) sorted by r0, for ranges of at most SHORT_ROWS rows.
    short: HashMap<(u32, u32), Vec<RowSpan>>,
    long: Vec<(Rng, u32)>,
    volatile: Vec<u32>,
    /// Formulas whose references all point into their own row on their own sheet
    /// (like `=F2*G2`). Their value survives a row sort unchanged.
    row_local: Vec<bool>,
}

impl DepIndex {
    pub(crate) fn build(wb: &Workbook) -> DepIndex {
        let mut idx = DepIndex::default();
        for (si, sheet) in wb.sheets.iter().enumerate() {
            for (r, row) in sheet.rows.iter().enumerate() {
                for (c, cell) in row.iter().enumerate() {
                    let Cell::Formula(f) = cell else { continue };
                    let fi = idx.formulas.len() as u32;
                    idx.formulas.push((si as u32, r as u32, c as u32));
                    idx.row_local.push(false);
                    let Ok(expr) = parse(&f.src) else { continue };
                    let volatile = has_volatile(&expr);
                    if volatile {
                        idx.volatile.push(fi);
                    }
                    let mut refs = Vec::new();
                    collect_refs(&expr, &mut refs);
                    idx.row_local[fi as usize] = !volatile
                        && refs.iter().all(|rt| {
                            let (r0, _, r1, _) = rt.bounds();
                            r0 == r as u32 && r1 == r as u32 && wb.sheet_index_for(rt.sheet.as_deref(), si) == Some(si)
                        });
                    for rt in refs {
                        let Some(s) = wb.sheet_index_for(rt.sheet.as_deref(), si) else { continue };
                        let (r0, c0, r1, c1) = rt.bounds();
                        if r1 - r0 < SHORT_ROWS && c1 - c0 < SHORT_COLS {
                            for col in c0..=c1 {
                                idx.short.entry((s as u32, col)).or_default().push((r0, r1, fi));
                            }
                        } else {
                            idx.long.push((Rng { sheet: s, r0, c0, r1, c1 }, fi));
                        }
                    }
                }
            }
        }
        for v in idx.short.values_mut() {
            v.sort_unstable();
        }
        idx
    }

    fn dependents_of(&self, (s, r, c): Pos, out: &mut dyn FnMut(Pos)) {
        if let Some(list) = self.short.get(&(s, c)) {
            let end = list.partition_point(|e| e.0 <= r);
            for e in list[..end].iter().rev() {
                if e.0 + SHORT_ROWS <= r {
                    break;
                }
                if e.1 >= r {
                    out(self.formulas[e.2 as usize]);
                }
            }
        }
        for (rng, fi) in &self.long {
            if rng.sheet as u32 == s && (rng.r0..=rng.r1).contains(&r) && (rng.c0..=rng.c1).contains(&c) {
                out(self.formulas[*fi as usize]);
            }
        }
    }

    pub(crate) fn all(&self) -> &[Pos] {
        &self.formulas
    }

    /// Formulas that are not row-local, i.e. that may change when rows are reordered.
    pub(crate) fn non_row_local(&self) -> Vec<Pos> {
        self.formulas.iter().zip(&self.row_local).filter(|(_, l)| !**l).map(|(p, _)| *p).collect()
    }
}

/// Collect every formula that must be recomputed after `changed` cells changed.
pub(crate) fn dirty_set(idx: &DepIndex, changed: &[Pos], wb: &Workbook) -> HashSet<Pos> {
    let mut dirty: HashSet<Pos> = HashSet::new();
    let mut queue: Vec<Pos> = Vec::with_capacity(changed.len());
    for &p in changed {
        if matches!(wb.sheets[p.0 as usize].cell(p.1 as usize, p.2 as usize), Cell::Formula(_)) {
            dirty.insert(p);
        }
        queue.push(p);
    }
    for &fi in &idx.volatile {
        let p = idx.formulas[fi as usize];
        if dirty.insert(p) {
            queue.push(p);
        }
    }
    while let Some(p) = queue.pop() {
        idx.dependents_of(p, &mut |d| {
            if dirty.insert(d) {
                queue.push(d);
            }
        });
    }
    dirty
}

struct Recalc<'a> {
    wb: &'a Workbook,
    dirty: HashSet<Pos>,
    results: HashMap<Pos, Value>,
    in_progress: HashSet<Pos>,
    depth: usize,
    now: f64,
}

impl Recalc<'_> {
    fn compute(&mut self, p: Pos) -> Value {
        if let Some(v) = self.results.get(&p) {
            return v.clone();
        }
        let Cell::Formula(f) = self.wb.sheets[p.0 as usize].cell(p.1 as usize, p.2 as usize) else {
            return self.wb.sheets[p.0 as usize].value(p.1 as usize, p.2 as usize).clone();
        };
        if self.in_progress.contains(&p) || self.depth > MAX_DEPTH {
            return Value::Error(CellError::Cycle);
        }
        self.in_progress.insert(p);
        self.depth += 1;
        let v = match parse(&f.src) {
            Ok(expr) => {
                let mut ev = Evaluator { ctx: self, sheet: p.0 as usize, row: p.1, col: p.2 };
                ev.eval_formula(&expr)
            }
            Err(_) => Value::Error(CellError::Name),
        };
        self.depth -= 1;
        self.in_progress.remove(&p);
        self.dirty.remove(&p);
        // An empty result displays as 0, like Excel.
        let v = if v.is_empty() { Value::Number(0.0) } else { v };
        self.results.insert(p, v.clone());
        v
    }
}

impl Context for Recalc<'_> {
    fn resolve_sheet(&self, name: Option<&str>, current: usize) -> Option<usize> {
        self.wb.sheet_index_for(name, current)
    }

    fn cell_value(&mut self, sheet: usize, row: u32, col: u32) -> Value {
        let Some(sh) = self.wb.sheets.get(sheet) else { return Value::Error(CellError::Ref) };
        match sh.cell(row as usize, col as usize) {
            Cell::Formula(f) => {
                let p = (sheet as u32, row, col);
                if self.dirty.contains(&p) || self.results.contains_key(&p) {
                    self.compute(p)
                } else {
                    f.value.clone()
                }
            }
            c => c.value().clone(),
        }
    }

    fn used_size(&self, sheet: usize) -> (u32, u32) {
        self.wb.sheets.get(sheet).map(|s| (s.row_count() as u32, s.col_count() as u32)).unwrap_or((0, 0))
    }

    fn for_each(&mut self, rng: &Rng, f: &mut dyn FnMut(u32, u32, &Value)) {
        let wb = self.wb;
        let Some(sh) = wb.sheets.get(rng.sheet) else { return };
        let r_end = (rng.r1 as usize + 1).min(sh.rows.len());
        for r in rng.r0 as usize..r_end {
            let row = &sh.rows[r];
            let c_end = (rng.c1 as usize + 1).min(row.len());
            for (c, cell) in row.iter().enumerate().take(c_end).skip(rng.c0 as usize) {
                match cell {
                    Cell::Empty => {}
                    Cell::Value(v) => f(r as u32, c as u32, v),
                    Cell::Formula(fc) => {
                        let p = (rng.sheet as u32, r as u32, c as u32);
                        if self.dirty.contains(&p) || self.results.contains_key(&p) {
                            let v = self.compute(p);
                            f(r as u32, c as u32, &v)
                        } else {
                            f(r as u32, c as u32, &fc.value)
                        }
                    }
                }
            }
        }
    }

    fn now(&self) -> f64 {
        self.now
    }
}

pub(crate) fn now_serial() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    // Local offset is not known without a tz database; UTC is a reasonable default.
    25569.0 + secs / 86400.0
}

/// Recompute the given formulas (and use fresh values for anything they read that is also dirty).
/// Returns the number of formulas recomputed.
pub(crate) fn recalc(wb: &mut Workbook, dirty: HashSet<Pos>) -> usize {
    if dirty.is_empty() {
        return 0;
    }
    let mut order: Vec<Pos> = dirty.iter().copied().collect();
    order.sort_unstable();
    let results = {
        let wb_ref: &Workbook = wb;
        let run = move || {
            let mut rc =
                Recalc { wb: wb_ref, dirty, results: HashMap::new(), in_progress: HashSet::new(), depth: 0, now: now_serial() };
            for p in order {
                if rc.dirty.contains(&p) {
                    rc.compute(p);
                }
            }
            rc.results
        };
        // Deep dependency chains recurse; give them a big (lazily committed) stack.
        std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(512 * 1024 * 1024)
                .spawn_scoped(s, run)
                .expect("spawn recalc thread")
                .join()
                .unwrap_or_default()
        })
    };
    let n = results.len();
    for ((s, r, c), v) in results {
        if let Some(Cell::Formula(f)) = wb.sheets[s as usize].cell_mut(r as usize, c as usize) {
            f.value = v;
        }
    }
    n
}

/// Evaluate a formula without storing it, e.g. for a status bar.
pub fn evaluate_standalone(wb: &Workbook, sheet: usize, src: &str) -> Value {
    let Ok(expr) = parse(src) else { return Value::Error(CellError::Name) };
    let mut rc =
        Recalc { wb, dirty: HashSet::new(), results: HashMap::new(), in_progress: HashSet::new(), depth: 0, now: now_serial() };
    let mut ev = Evaluator { ctx: &mut rc, sheet, row: 0, col: 0 };
    ev.eval_formula(&expr)
}
