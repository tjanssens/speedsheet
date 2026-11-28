//! Excel's fill handle: extend a selection down, up, right or left, continuing series
//! (1, 2, 3 · dates · "Item 1" · weekdays and months) and shifting formulas.

use std::sync::Arc;

use crate::formula::parser::shift_formula;
use crate::sheet::Cell;
use crate::value::Value;

/// Day and month names that continue as a series, in English and Dutch.
const LISTS: &[&[&str]] = &[
    &["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"],
    &["mon", "tue", "wed", "thu", "fri", "sat", "sun"],
    &["maandag", "dinsdag", "woensdag", "donderdag", "vrijdag", "zaterdag", "zondag"],
    &["ma", "di", "wo", "do", "vr", "za", "zo"],
    &["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"],
    &["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"],
    &["januari", "februari", "maart", "april", "mei", "juni", "juli", "augustus", "september", "oktober", "november", "december"],
    &["jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec"],
];

/// How a line of source cells continues.
enum Series {
    /// Repeat the source cells (formulas shifted).
    Copy,
    /// a + b·k, as numbers or dates.
    Linear { a: f64, b: f64, date: bool },
    /// prefix + (a + b·k) zero-padded to `width` + suffix
    TextNum { prefix: String, suffix: String, a: f64, b: f64, width: usize },
    /// Position in a named list.
    Named { list: usize, start: i64, step: i64, case: Case },
}

#[derive(Clone, Copy)]
enum Case {
    Lower,
    Upper,
    Title,
}

fn case_of(s: &str) -> Case {
    if s.chars().all(|c| !c.is_lowercase()) {
        Case::Upper
    } else if s.chars().next().is_some_and(|c| c.is_uppercase()) {
        Case::Title
    } else {
        Case::Lower
    }
}

fn apply_case(s: &str, case: Case) -> String {
    match case {
        Case::Lower => s.to_string(),
        Case::Upper => s.to_uppercase(),
        Case::Title => {
            let mut c = s.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

fn find_in_lists(s: &str) -> Option<(usize, i64)> {
    let low = s.trim().to_lowercase();
    LISTS.iter().enumerate().find_map(|(i, l)| l.iter().position(|x| *x == low).map(|p| (i, p as i64)))
}

/// Split "Item 12b" into ("Item ", 12, "b", width 2). Uses the last group of digits.
fn split_num(s: &str) -> Option<(String, f64, String, usize)> {
    let b = s.as_bytes();
    let end = b.iter().rposition(|c| c.is_ascii_digit())? + 1;
    let start = b[..end].iter().rposition(|c| !c.is_ascii_digit()).map(|p| p + 1).unwrap_or(0);
    let digits = s.get(start..end)?;
    let n: f64 = digits.parse().ok()?;
    let width = if digits.starts_with('0') && digits.len() > 1 { digits.len() } else { 0 };
    Some((s.get(..start)?.to_string(), n, s.get(end..)?.to_string(), width))
}

/// Least-squares line through (k, y_k).
fn linear(ys: &[f64]) -> (f64, f64) {
    let n = ys.len() as f64;
    if ys.len() < 2 {
        return (ys.first().copied().unwrap_or(0.0), 0.0);
    }
    let mx = (n - 1.0) / 2.0;
    let my = ys.iter().sum::<f64>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (k, y) in ys.iter().enumerate() {
        let dx = k as f64 - mx;
        sxy += dx * (y - my);
        sxx += dx * dx;
    }
    let b = if sxx == 0.0 { 0.0 } else { sxy / sxx };
    (my - b * mx, b)
}

fn detect(src: &[&Cell]) -> Series {
    if src.iter().any(|c| matches!(c, Cell::Formula(_) | Cell::Empty)) {
        return Series::Copy;
    }
    let vals: Vec<&Value> = src.iter().map(|c| c.value()).collect();
    // Numbers: a single number is copied (like Excel), two or more continue the trend.
    if vals.iter().all(|v| matches!(v, Value::Number(_))) {
        if vals.len() < 2 {
            return Series::Copy;
        }
        let ys: Vec<f64> = vals.iter().filter_map(|v| v.as_number().ok()).collect();
        let (a, b) = linear(&ys);
        return Series::Linear { a, b, date: false };
    }
    // Dates: a single date counts up by days.
    if vals.iter().all(|v| matches!(v, Value::Date(_))) {
        let ys: Vec<f64> = vals.iter().filter_map(|v| v.as_number().ok()).collect();
        if ys.len() == 1 {
            return Series::Linear { a: ys[0], b: 1.0, date: true };
        }
        let (a, b) = linear(&ys);
        return Series::Linear { a, b, date: true };
    }
    let texts: Vec<&str> = vals.iter().filter_map(|v| if let Value::Text(t) = v { Some(t.as_ref()) } else { None }).collect();
    if texts.len() != vals.len() {
        return Series::Copy;
    }
    // Day and month names.
    let named: Vec<(usize, i64)> = texts.iter().map(|t| find_in_lists(t)).collect::<Option<_>>().unwrap_or_default();
    if let Some(&(list, first)) = named.first() {
        if named.iter().all(|n| n.0 == list) {
            let step = named.get(1).map_or(1, |n| n.1 - first);
            let len = LISTS[list].len() as i64;
            let consistent = named.windows(2).all(|w| (w[1].1 - w[0].1 - step).rem_euclid(len) == 0);
            if consistent {
                return Series::Named { list, start: first, step, case: case_of(texts[0]) };
            }
        }
    }
    // Text with a number in it: "Item 1", "Q1", "Week 05".
    let parts: Vec<(String, f64, String, usize)> = texts.iter().map(|t| split_num(t)).collect::<Option<_>>().unwrap_or_default();
    if let Some((prefix, _, suffix, width)) = parts.first() {
        if parts.iter().all(|p| &p.0 == prefix && &p.2 == suffix) {
            let ys: Vec<f64> = parts.iter().map(|p| p.1).collect();
            let (a, b) = if ys.len() == 1 { (ys[0], 1.0) } else { linear(&ys) };
            return Series::TextNum { prefix: prefix.clone(), suffix: suffix.clone(), a, b, width: *width };
        }
    }
    Series::Copy
}

/// Produce the cell at position `k` (0-based from the first source cell, may be negative when
/// filling up/left) of a line, given the source cells and how far each step moves (dr, dc).
fn produce(series: &Series, src: &[&Cell], k: i64, delta: &dyn Fn(i64, usize) -> (i64, i64)) -> Cell {
    let n = src.len() as i64;
    match series {
        Series::Copy => {
            let i = k.rem_euclid(n);
            let cell = src[i as usize];
            match cell {
                Cell::Formula(f) => {
                    let (dr, dc) = delta(k, i as usize);
                    let src = shift_formula(&f.src, dr, dc);
                    Cell::formula(&src, Value::Empty)
                }
                c => c.clone(),
            }
        }
        Series::Linear { a, b, date } => {
            let v = a + b * k as f64;
            // Avoid 0.30000000000000004 style noise.
            let v = (v * 1e10).round() / 1e10;
            Cell::from_value(if *date { Value::Date(v) } else { Value::Number(v) })
        }
        Series::TextNum { prefix, suffix, a, b, width } => {
            let v = (a + b * k as f64).round();
            let num = if v < 0.0 { format!("{}", v as i64) } else { format!("{:0width$}", v as i64, width = *width) };
            Cell::from_value(Value::Text(Arc::from(format!("{prefix}{num}{suffix}"))))
        }
        Series::Named { list, start, step: s, case } => {
            let l = LISTS[*list];
            let i = (start + s * k).rem_euclid(l.len() as i64) as usize;
            Cell::from_value(Value::Text(Arc::from(apply_case(l[i], *case))))
        }
    }
}

/// Fill one line. `src` are the source cells in sheet order; `ks` the target positions relative
/// to the first source cell (negative when filling up or left). `delta(k, i)` is how far target
/// `k` is from source cell `i`, used to shift copied formulas.
pub(crate) fn fill_line(src: &[&Cell], ks: impl Iterator<Item = i64>, delta: &dyn Fn(i64, usize) -> (i64, i64)) -> Vec<Cell> {
    let series = detect(src);
    ks.map(|k| produce(&series, src, k, delta)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(inputs: &[&str]) -> Vec<Cell> {
        inputs.iter().map(|s| crate::input::parse_input(s)).collect()
    }

    fn fill(inputs: &[&str], ks: std::ops::Range<i64>) -> Vec<String> {
        let c = cells(inputs);
        let refs: Vec<&Cell> = c.iter().collect();
        fill_line(&refs, ks, &|k, i| (k - i as i64, 0)).iter().map(|c| c.input_text()).collect()
    }

    #[test]
    fn series() {
        assert_eq!(fill(&["1"], 1..4), ["1", "1", "1"]);
        assert_eq!(fill(&["1", "2"], 2..5), ["3", "4", "5"]);
        assert_eq!(fill(&["10", "20", "30"], 3..5), ["40", "50"]);
        assert_eq!(fill(&["0.1", "0.2"], 2..4), ["0.3", "0.4"]);
        assert_eq!(fill(&["1", "2"], -2..0), ["-1", "0"]);
        assert_eq!(fill(&["Item 1"], 1..3), ["Item 2", "Item 3"]);
        assert_eq!(fill(&["Week 08"], 1..3), ["Week 09", "Week 10"]);
        assert_eq!(fill(&["Q1", "Q3"], 2..4), ["Q5", "Q7"]);
        assert_eq!(fill(&["Maandag"], 1..4), ["Dinsdag", "Woensdag", "Donderdag"]);
        assert_eq!(fill(&["sat"], 1..3), ["sun", "mon"]);
        assert_eq!(fill(&["Jan", "Mar"], 2..4), ["May", "Jul"]);
        assert_eq!(fill(&["OKT"], 1..4), ["NOV", "DEC", "JAN"]);
        assert_eq!(fill(&["a", "b"], 2..5), ["a", "b", "a"]);
        assert_eq!(fill(&["2026-01-30"], 1..3), ["2026-01-31", "2026-02-01"]);
        assert_eq!(fill(&["2026-01-01", "2026-01-08"], 2..3), ["2026-01-15"]);
    }

    #[test]
    fn formulas_shift() {
        assert_eq!(fill(&["=A1*2"], 1..3), ["=A2*2", "=A3*2"]);
        assert_eq!(fill(&["=$A$1+B1"], 1..2), ["=$A$1+B2"]);
        let c = cells(&["=A1"]);
        let refs: Vec<&Cell> = c.iter().collect();
        let right: Vec<String> = fill_line(&refs, 1..3, &|k, i| (0, k - i as i64)).iter().map(|c| c.input_text()).collect();
        assert_eq!(right, ["=B1", "=C1"]);
    }
}
