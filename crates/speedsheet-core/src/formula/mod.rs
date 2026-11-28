//! Formula engine: parsing, evaluation and incremental recalculation.

pub mod eval;
pub mod parser;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use eval::{Context, Evaluator, Rng};
use parser::{collect_names, collect_refs, has_volatile, parse, Expr, RefPart, RefTok};

use crate::sheet::Cell;
use crate::value::{CellError, Value};
use crate::workbook::Workbook;

/// (sheet, row, col)
pub type Pos = (u32, u32, u32);

const SHORT_ROWS: u32 = 1024;
const SHORT_COLS: u32 = 64;
/// Recursion limit for on-demand evaluation. Recalculation runs in dependency order, so a
/// formula's inputs are normally computed before it and this is only a safety net.
const MAX_DEPTH: usize = 20_000;
/// How deep defined names may refer to other names.
pub(crate) const MAX_NAME_DEPTH: u32 = 32;

/// Parsed definitions of defined names, looked up case-insensitively; sheet-local names win.
#[derive(Default)]
pub(crate) struct NameCache {
    map: HashMap<(usize, String), Option<Arc<Expr>>>,
}

impl NameCache {
    pub(crate) fn get(&mut self, wb: &Workbook, name: &str, sheet: usize) -> Option<Arc<Expr>> {
        let key = (sheet, name.to_ascii_uppercase());
        if let Some(e) = self.map.get(&key) {
            return e.clone();
        }
        let e = wb.name_formula(name, sheet).and_then(|src| parse(src).ok()).map(Arc::new);
        self.map.insert(key, e.clone());
        e
    }
}

/// References a formula reads through the defined names it uses (recursively).
/// Returns whether any name is volatile.
fn name_refs(wb: &Workbook, names: &mut NameCache, expr: &Expr, sheet: usize, depth: u32, out: &mut Vec<RefTok>) -> bool {
    let mut used = Vec::new();
    collect_names(expr, &mut used);
    let mut volatile = false;
    for n in used {
        if depth >= MAX_NAME_DEPTH {
            break;
        }
        let Some(def) = names.get(wb, n, sheet) else { continue };
        volatile |= has_volatile(&def);
        let mut refs = Vec::new();
        collect_refs(&def, &mut refs);
        out.extend(refs.into_iter().cloned());
        volatile |= name_refs(wb, names, &def, sheet, depth + 1, out);
    }
    volatile
}

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
        let mut names = NameCache::default();
        let has_names = wb.has_names();
        for (si, sheet) in wb.sheets.iter().enumerate() {
            for (r, row) in sheet.rows.iter().enumerate() {
                for (c, cell) in row.iter().enumerate() {
                    let Cell::Formula(f) = cell else { continue };
                    let fi = idx.formulas.len() as u32;
                    idx.formulas.push((si as u32, r as u32, c as u32));
                    idx.row_local.push(false);
                    let Ok(expr) = parse(&f.src) else { continue };
                    let mut volatile = has_volatile(&expr);
                    let mut refs = Vec::new();
                    collect_refs(&expr, &mut refs);
                    // References reached through defined names (`=Rate*100`).
                    let mut via_names = Vec::new();
                    if has_names {
                        volatile |= name_refs(wb, &mut names, &expr, si, 0, &mut via_names);
                    }
                    if volatile {
                        idx.volatile.push(fi);
                    }
                    refs.extend(via_names.iter());
                    // Only relative row references move with the row: `=$B$2` in row 2 still reads
                    // row 2 after the sort moved it elsewhere, and so do names (found by tests/invariants.rs).
                    let relative_row = |p: &RefPart| matches!(p, RefPart::Cell { abs_row: false, .. });
                    idx.row_local[fi as usize] = !volatile
                        && via_names.is_empty()
                        && refs.iter().all(|rt| {
                            let (r0, _, r1, _) = rt.bounds();
                            r0 == r as u32
                                && r1 == r as u32
                                && relative_row(&rt.a)
                                && rt.b.as_ref().is_none_or(relative_row)
                                && wb.sheet_index_for(rt.sheet.as_deref(), si) == Some(si)
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

/// Order in which to compute `dirty`: every formula comes after the dirty formulas it reads
/// (Kahn's algorithm, no recursion, so chains of any length and direction work). Formulas on
/// a circular reference are returned separately; whatever depends on them is still ordered.
fn plan(idx: &DepIndex, dirty: &HashSet<Pos>) -> (Vec<Pos>, Vec<Pos>) {
    let mut nodes: Vec<Pos> = dirty.iter().copied().collect();
    nodes.sort_unstable();
    let n = nodes.len();
    let id: HashMap<Pos, u32> = nodes.iter().enumerate().map(|(i, p)| (*p, i as u32)).collect();
    // Edges p -> formulas reading p, in compressed rows.
    let mut start = Vec::with_capacity(n + 1);
    let mut adj: Vec<u32> = Vec::with_capacity(n);
    let mut indeg = vec![0u32; n];
    start.push(0);
    for &p in &nodes {
        idx.dependents_of(p, &mut |d| {
            if let Some(&j) = id.get(&d) {
                adj.push(j);
                indeg[j as usize] += 1;
            }
        });
        start.push(adj.len());
    }
    let succ = |u: u32| &adj[start[u as usize]..start[u as usize + 1]];
    let mut order: Vec<u32> = (0..n as u32).filter(|&i| indeg[i as usize] == 0).collect();
    let mut done = vec![false; n];
    let mut head = 0;
    let mut cyclic = Vec::new();
    loop {
        while head < order.len() {
            let u = order[head];
            head += 1;
            done[u as usize] = true;
            for &v in succ(u) {
                indeg[v as usize] -= 1;
                if indeg[v as usize] == 0 {
                    order.push(v);
                }
            }
        }
        if order.len() + cyclic.len() == n {
            break;
        }
        // Stuck: what is left lies on or behind a cycle. Take out the cycles and go on.
        let on_cycle = cycles(n, &done, &succ);
        for &u in &on_cycle {
            done[u as usize] = true;
        }
        for &u in &on_cycle {
            for &v in succ(u) {
                if !done[v as usize] {
                    indeg[v as usize] -= 1;
                    if indeg[v as usize] == 0 {
                        order.push(v);
                    }
                }
            }
        }
        cyclic.extend(on_cycle.iter().map(|&u| nodes[u as usize]));
        if on_cycle.is_empty() {
            break; // cannot happen; never loop forever
        }
    }
    (order.into_iter().map(|u| nodes[u as usize]).collect(), cyclic)
}

/// Nodes not yet `done` that lie on a cycle: strongly connected components with more than one
/// node or with a self-loop (iterative Tarjan).
fn cycles<'a>(n: usize, done: &[bool], succ: &dyn Fn(u32) -> &'a [u32]) -> Vec<u32> {
    const UNSEEN: u32 = u32::MAX;
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut out = Vec::new();
    let mut next = 0u32;
    for root in 0..n as u32 {
        if done[root as usize] || index[root as usize] != UNSEEN {
            continue;
        }
        // (node, position in its successor list)
        let mut call: Vec<(u32, usize)> = vec![(root, 0)];
        index[root as usize] = next;
        low[root as usize] = next;
        next += 1;
        stack.push(root);
        on_stack[root as usize] = true;
        while let Some(&mut (u, ref mut i)) = call.last_mut() {
            let s = succ(u);
            if *i < s.len() {
                let v = s[*i];
                *i += 1;
                if done[v as usize] {
                    continue;
                }
                if index[v as usize] == UNSEEN {
                    index[v as usize] = next;
                    low[v as usize] = next;
                    next += 1;
                    stack.push(v);
                    on_stack[v as usize] = true;
                    call.push((v, 0));
                } else if on_stack[v as usize] {
                    low[u as usize] = low[u as usize].min(index[v as usize]);
                }
                continue;
            }
            call.pop();
            if let Some(&(parent, _)) = call.last() {
                low[parent as usize] = low[parent as usize].min(low[u as usize]);
            }
            if low[u as usize] == index[u as usize] {
                let mut comp = Vec::new();
                loop {
                    let w = stack.pop().expect("tarjan stack");
                    on_stack[w as usize] = false;
                    comp.push(w);
                    if w == u {
                        break;
                    }
                }
                if comp.len() > 1 || succ(u).contains(&u) {
                    out.extend(comp);
                }
            }
        }
    }
    out
}

struct Recalc<'a> {
    wb: &'a Workbook,
    dirty: HashSet<Pos>,
    results: HashMap<Pos, Value>,
    in_progress: HashSet<Pos>,
    depth: usize,
    now: f64,
    names: NameCache,
}

impl<'a> Recalc<'a> {
    fn new(wb: &'a Workbook, dirty: HashSet<Pos>) -> Recalc<'a> {
        Recalc {
            wb,
            dirty,
            results: HashMap::new(),
            in_progress: HashSet::new(),
            depth: 0,
            now: now_serial(),
            names: NameCache::default(),
        }
    }

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
                let mut ev = Evaluator::new(self, p.0 as usize, p.1, p.2);
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

    fn name_expr(&mut self, name: &str, sheet: usize) -> Option<Arc<Expr>> {
        self.names.get(self.wb, name, sheet)
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
        for (r, row) in sh.rows.iter().enumerate().take(r_end).skip(rng.r0 as usize) {
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

/// Excel serial number for a moment given as Unix seconds plus the local UTC offset in seconds.
pub fn serial_from_unix(unix_secs: f64, utc_offset_secs: i32) -> f64 {
    25569.0 + (unix_secs + utc_offset_secs as f64) / 86400.0
}

/// The current local time as an Excel serial number (Excel's NOW() is local time).
pub(crate) fn now_serial() -> f64 {
    let now = crate::clock::local_now();
    let unix = now.timestamp() as f64 + now.timestamp_subsec_nanos() as f64 / 1e9;
    serial_from_unix(unix, now.offset().local_minus_utc())
}

/// Recompute the given formulas in dependency order, using fresh values for anything they
/// read that is also dirty. Formulas on a circular reference get #CYCLE!.
/// Returns the number of formulas recomputed.
pub(crate) fn recalc(wb: &mut Workbook, dirty: HashSet<Pos>) -> usize {
    if dirty.is_empty() {
        return 0;
    }
    let idx = wb.deps.take().unwrap_or_else(|| DepIndex::build(wb));
    let results = {
        let wb_ref: &Workbook = wb;
        let idx = &idx;
        let run = move || {
            let (order, cyclic) = plan(idx, &dirty);
            let mut rc = Recalc::new(wb_ref, dirty);
            for p in cyclic {
                rc.dirty.remove(&p);
                rc.results.insert(p, Value::Error(CellError::Cycle));
            }
            for p in order {
                if rc.dirty.contains(&p) {
                    rc.compute(p);
                }
            }
            rc.results
        };
        // Chains no longer recurse, but one very long formula still nests deeply while it is
        // evaluated; a big (lazily committed) stack keeps that safe.
        std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(256 * 1024 * 1024)
                .spawn_scoped(s, run)
                .expect("spawn recalc thread")
                .join()
                .unwrap_or_default()
        })
    };
    wb.deps = Some(idx);
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
    let mut rc = Recalc::new(wb, HashSet::new());
    let mut ev = Evaluator::new(&mut rc, sheet, 0, 0);
    ev.eval_formula(&expr)
}
