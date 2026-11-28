//! Time the core operations on a file: `cargo run --release -p speedsheet-core --example bench -- testdata/large-500k.xlsx`
//!
//! CI compares the output with `bench/budget.txt` and with the base branch (`scripts/bench_check.py`).
//! Keep the `name  <number> ms` line format: the checker parses it.

use std::time::Instant;

use speedsheet_core::{io, Filter, FilterCond, FilterOp, Workbook};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "testdata/large-500k.xlsx".into());
    let t = Instant::now();
    let mut wb = io::open(path.as_ref())?;
    let s = &wb.sheets[0];
    println!("open      {:>8.0} ms  ({} rows x {} cols)", t.elapsed().as_secs_f64() * 1e3, s.row_count(), s.col_count());

    let t = Instant::now();
    wb.set_inputs(0, 1, 5, &[vec!["1".into()]])?;
    println!("edit+calc {:>8.1} ms", t.elapsed().as_secs_f64() * 1e3);

    let t = Instant::now();
    wb.sort(0, 5, false, true)?;
    println!("sort      {:>8.0} ms", t.elapsed().as_secs_f64() * 1e3);

    let t = Instant::now();
    let n = wb.set_filter(
        0,
        Some(Filter { header: true, conds: vec![FilterCond { col: 3, op: FilterOp::Equals { value: "Gent".into() } }] }),
    )?;
    println!("filter    {:>8.0} ms  ({} rows visible)", t.elapsed().as_secs_f64() * 1e3, n);
    wb.set_filter(0, None)?;

    let t = Instant::now();
    let m = wb.find("nothing-here", 0, 0, 0, false, true);
    println!("find miss {:>8.0} ms  ({:?})", t.elapsed().as_secs_f64() * 1e3, m.map(|m| m.row));

    let t = Instant::now();
    wb.undo();
    println!("undo sort {:>8.0} ms", t.elapsed().as_secs_f64() * 1e3);

    let out = std::env::temp_dir().join("speedsheet-bench.xlsx");
    let t = Instant::now();
    io::save(&mut wb, &out, 0)?;
    println!("save xlsx {:>8.0} ms", t.elapsed().as_secs_f64() * 1e3);
    chain()?;
    Ok(())
}

/// A 50,000-cell dependency chain (A2=A1+1, ...): entering it, then changing its first cell,
/// which recalculates every link.
fn chain() -> Result<(), Box<dyn std::error::Error>> {
    let n = 50_000;
    let mut grid = vec![vec!["1".to_string()]];
    grid.extend((1..n).map(|r| vec![format!("=A{r}+1")]));
    let mut wb = Workbook::new();
    let t = Instant::now();
    wb.set_inputs(0, 0, 0, &grid)?;
    println!("chain in  {:>8.0} ms  ({n} formulas)", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let ch = wb.set_inputs(0, 0, 0, &[vec!["2".into()]])?;
    println!("chain calc{:>8.1} ms  ({} recalculated)", t.elapsed().as_secs_f64() * 1e3, ch.recalculated);
    Ok(())
}
