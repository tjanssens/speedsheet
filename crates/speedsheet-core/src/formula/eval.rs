//! Formula evaluation.

use std::cmp::Ordering;
use std::sync::Arc;

use super::parser::{Expr, RefTok};
use super::MAX_NAME_DEPTH;
use crate::value::{compare_text, format_date, format_number, serial_to_ymd, ymd_to_serial, CellError, Value};

/// What the evaluator needs from a workbook.
pub trait Context {
    /// Resolve a sheet name (None = the formula's own sheet, given as `current`).
    fn resolve_sheet(&self, name: Option<&str>, current: usize) -> Option<usize>;
    /// The parsed definition of a defined name as seen from `sheet` (sheet-local names first).
    fn name_expr(&mut self, name: &str, sheet: usize) -> Option<Arc<Expr>>;
    /// Value of a cell, computing it first if it is a stale formula.
    fn cell_value(&mut self, sheet: usize, row: u32, col: u32) -> Value;
    /// Used size (rows, cols) of a sheet, to clip whole-column/row references.
    fn used_size(&self, sheet: usize) -> (u32, u32);
    /// Visit every non-empty cell in the (clipped) rectangle, row by row.
    fn for_each(&mut self, rng: &Rng, f: &mut dyn FnMut(u32, u32, &Value));
    /// Current time as an Excel serial number.
    fn now(&self) -> f64;
}

/// A resolved rectangular range (inclusive).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rng {
    pub sheet: usize,
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

impl Rng {
    pub fn rows(&self) -> u32 {
        self.r1 - self.r0 + 1
    }
    pub fn cols(&self) -> u32 {
        self.c1 - self.c0 + 1
    }
}

enum Ev {
    V(Value),
    R(Rng),
}

pub struct Evaluator<'c, C: Context> {
    pub ctx: &'c mut C,
    pub sheet: usize,
    pub row: u32,
    pub col: u32,
    /// How many defined names deep the evaluation currently is (guards against name cycles).
    name_depth: u32,
}

type R<T> = Result<T, CellError>;

fn num(n: f64) -> Value {
    if n.is_finite() {
        Value::Number(n)
    } else {
        Value::Error(CellError::Num)
    }
}

impl<'c, C: Context> Evaluator<'c, C> {
    pub fn new(ctx: &'c mut C, sheet: usize, row: u32, col: u32) -> Self {
        Evaluator { ctx, sheet, row, col, name_depth: 0 }
    }

    pub fn eval_formula(&mut self, e: &Expr) -> Value {
        match self.eval(e) {
            Ev::V(v) => v,
            Ev::R(r) => self.implicit(r),
        }
    }

    fn resolve(&self, r: &RefTok) -> R<Rng> {
        let sheet = self.ctx.resolve_sheet(r.sheet.as_deref(), self.sheet).ok_or(CellError::Ref)?;
        let (r0, c0, r1, c1) = r.bounds();
        Ok(Rng { sheet, r0, c0, r1, c1 })
    }

    /// Clip a range to the used area of its sheet (keeps at least one cell).
    fn clip(&self, rng: Rng) -> Rng {
        let (rows, cols) = self.ctx.used_size(rng.sheet);
        Rng { r1: rng.r1.min(rows.max(1) - 1).max(rng.r0), c1: rng.c1.min(cols.max(1) - 1).max(rng.c0), ..rng }
    }

    fn implicit(&mut self, r: Rng) -> Value {
        if r.r0 == r.r1 && r.c0 == r.c1 {
            return self.ctx.cell_value(r.sheet, r.r0, r.c0);
        }
        if r.c0 == r.c1 && (r.r0..=r.r1).contains(&self.row) {
            return self.ctx.cell_value(r.sheet, self.row, r.c0);
        }
        if r.r0 == r.r1 && (r.c0..=r.c1).contains(&self.col) {
            return self.ctx.cell_value(r.sheet, r.r0, self.col);
        }
        Value::Error(CellError::Value)
    }

    fn eval(&mut self, e: &Expr) -> Ev {
        match e {
            Expr::Num(n) => Ev::V(Value::Number(*n)),
            Expr::Str(s) => Ev::V(Value::Text(s.clone())),
            Expr::Bool(b) => Ev::V(Value::Bool(*b)),
            Expr::Err(er) => Ev::V(Value::Error(*er)),
            Expr::Missing => Ev::V(Value::Empty),
            Expr::Name(n) => {
                if self.name_depth >= MAX_NAME_DEPTH {
                    return Ev::V(Value::Error(CellError::Name));
                }
                match self.ctx.name_expr(n, self.sheet) {
                    Some(def) => {
                        self.name_depth += 1;
                        let v = self.eval(&def);
                        self.name_depth -= 1;
                        v
                    }
                    None => Ev::V(Value::Error(CellError::Name)),
                }
            }
            Expr::Ref(r) => match self.resolve(r) {
                Ok(rng) if r.is_single_cell() => Ev::V(self.ctx.cell_value(rng.sheet, rng.r0, rng.c0)),
                Ok(rng) => Ev::R(rng),
                Err(er) => Ev::V(Value::Error(er)),
            },
            Expr::Neg(a) => Ev::V(match self.scalar(a).as_number() {
                Ok(n) => Value::Number(-n),
                Err(er) => Value::Error(er),
            }),
            Expr::Percent(a) => Ev::V(match self.scalar(a).as_number() {
                Ok(n) => Value::Number(n / 100.0),
                Err(er) => Value::Error(er),
            }),
            Expr::Bin(op, a, b) => {
                let va = self.scalar(a);
                let vb = self.scalar(b);
                Ev::V(binary(op, &va, &vb))
            }
            Expr::Call(name, args) => self.call(name, args),
        }
    }

    fn scalar(&mut self, e: &Expr) -> Value {
        match self.eval(e) {
            Ev::V(v) => v,
            Ev::R(r) => self.implicit(r),
        }
    }

    fn range_arg(&mut self, e: &Expr) -> R<Rng> {
        match e {
            Expr::Ref(r) => self.resolve(r),
            other => match self.eval(other) {
                Ev::R(r) => Ok(r),
                Ev::V(Value::Error(er)) => Err(er),
                Ev::V(_) => Err(CellError::Value),
            },
        }
    }

    fn num_arg(&mut self, e: &Expr) -> R<f64> {
        self.scalar(e).as_number()
    }

    fn text_arg(&mut self, e: &Expr) -> R<String> {
        self.scalar(e).as_text()
    }

    fn opt_num(&mut self, args: &[Expr], i: usize, default: f64) -> R<f64> {
        match args.get(i) {
            None | Some(Expr::Missing) => Ok(default),
            Some(e) => self.num_arg(e),
        }
    }

    /// Visit numbers following Excel's rules: direct arguments coerce text/bools,
    /// references only contribute real numbers. Errors propagate.
    fn numbers(&mut self, args: &[Expr], f: &mut dyn FnMut(f64)) -> R<()> {
        for a in args {
            match self.eval(a) {
                Ev::V(v) => match v {
                    Value::Empty => {}
                    Value::Error(e) => return Err(e),
                    v => {
                        if matches!(a, Expr::Ref(_) | Expr::Name(_)) {
                            if let Value::Number(n) | Value::Date(n) = v {
                                f(n)
                            }
                        } else {
                            f(v.as_number()?)
                        }
                    }
                },
                Ev::R(r) => {
                    let mut err = None;
                    let r = self.clip(r);
                    self.ctx.for_each(&r, &mut |_, _, v| match v {
                        Value::Number(n) | Value::Date(n) => f(*n),
                        Value::Error(e) if err.is_none() => err = Some(*e),
                        _ => {}
                    });
                    if let Some(e) = err {
                        return Err(e);
                    }
                }
            }
        }
        Ok(())
    }

    fn collect_numbers(&mut self, args: &[Expr]) -> R<Vec<f64>> {
        let mut v = Vec::new();
        self.numbers(args, &mut |n| v.push(n))?;
        Ok(v)
    }

    /// Visit every value (including text), for COUNTA, CONCAT and friends.
    fn values(&mut self, args: &[Expr], f: &mut dyn FnMut(&Value)) {
        for a in args {
            match self.eval(a) {
                Ev::V(v) => f(&v),
                Ev::R(r) => {
                    let r = self.clip(r);
                    self.ctx.for_each(&r, &mut |_, _, v| f(v));
                }
            }
        }
    }

    fn call(&mut self, name: &str, args: &[Expr]) -> Ev {
        let res: R<Value> = (|| -> R<Value> {
            let argc = args.len();
            let need = |min: usize, max: usize| -> R<()> {
                if argc < min || argc > max {
                    Err(CellError::Value)
                } else {
                    Ok(())
                }
            };
            Ok(match name {
                // ---- Logic
                "IF" => {
                    need(2, 3)?;
                    let c = self.scalar(&args[0]).as_bool()?;
                    return Ok(if c {
                        self.scalar(&args[1])
                    } else if argc == 3 {
                        self.scalar(&args[2])
                    } else {
                        Value::Bool(false)
                    })
                    .map(|v| if v.is_empty() { Value::Number(0.0) } else { v });
                }
                "IFS" => {
                    for pair in args.chunks(2) {
                        if pair.len() == 2 && self.scalar(&pair[0]).as_bool()? {
                            return Ok(self.scalar(&pair[1]));
                        }
                    }
                    Value::Error(CellError::NA)
                }
                "IFERROR" | "IFNA" => {
                    need(2, 2)?;
                    let v = self.scalar(&args[0]);
                    match v {
                        Value::Error(e) if name == "IFERROR" || e == CellError::NA => self.scalar(&args[1]),
                        v => v,
                    }
                }
                "AND" | "OR" | "XOR" => {
                    let mut bools = Vec::new();
                    let mut err = None;
                    self.values(args, &mut |v| match v {
                        Value::Empty | Value::Text(_) => {}
                        Value::Error(e) => {
                            err.get_or_insert(*e);
                        }
                        v => bools.push(v.as_bool().unwrap_or(false)),
                    });
                    if let Some(e) = err {
                        return Err(e);
                    }
                    if bools.is_empty() {
                        return Err(CellError::Value);
                    }
                    Value::Bool(match name {
                        "AND" => bools.iter().all(|b| *b),
                        "OR" => bools.iter().any(|b| *b),
                        _ => bools.iter().filter(|b| **b).count() % 2 == 1,
                    })
                }
                "NOT" => {
                    need(1, 1)?;
                    Value::Bool(!self.scalar(&args[0]).as_bool()?)
                }
                "TRUE" => Value::Bool(true),
                "FALSE" => Value::Bool(false),
                "NA" => Value::Error(CellError::NA),

                // ---- Aggregates
                "SUM" => {
                    let mut s = 0.0;
                    self.numbers(args, &mut |n| s += n)?;
                    num(s)
                }
                "PRODUCT" => {
                    let mut p = 1.0;
                    self.numbers(args, &mut |n| p *= n)?;
                    num(p)
                }
                "AVERAGE" => {
                    let (mut s, mut c) = (0.0, 0usize);
                    self.numbers(args, &mut |n| {
                        s += n;
                        c += 1
                    })?;
                    if c == 0 {
                        return Err(CellError::Div0);
                    }
                    num(s / c as f64)
                }
                "MIN" | "MAX" => {
                    let mut acc: Option<f64> = None;
                    let is_min = name == "MIN";
                    self.numbers(args, &mut |n| {
                        acc = Some(match acc {
                            None => n,
                            Some(a) if is_min => a.min(n),
                            Some(a) => a.max(n),
                        })
                    })?;
                    num(acc.unwrap_or(0.0))
                }
                "COUNT" => {
                    let mut c = 0usize;
                    for a in args {
                        match self.eval(a) {
                            Ev::V(v) => {
                                if matches!(v, Value::Number(_) | Value::Date(_))
                                    || (!matches!(a, Expr::Ref(_) | Expr::Name(_)) && v.as_number().is_ok() && !v.is_empty())
                                {
                                    c += 1
                                }
                            }
                            Ev::R(r) => {
                                let r = self.clip(r);
                                self.ctx.for_each(&r, &mut |_, _, v| {
                                    if matches!(v, Value::Number(_) | Value::Date(_)) {
                                        c += 1
                                    }
                                })
                            }
                        }
                    }
                    Value::Number(c as f64)
                }
                "COUNTA" => {
                    let mut c = 0usize;
                    self.values(args, &mut |v| {
                        if !v.is_empty() {
                            c += 1
                        }
                    });
                    Value::Number(c as f64)
                }
                "COUNTBLANK" => {
                    need(1, 1)?;
                    let r = self.range_arg(&args[0])?;
                    let total = r.rows() as f64 * r.cols() as f64;
                    let mut filled = 0usize;
                    let r = self.clip(r);
                    self.ctx.for_each(&r, &mut |_, _, v| {
                        if !matches!(v, Value::Text(t) if t.is_empty()) {
                            filled += 1
                        }
                    });
                    Value::Number(total - filled as f64)
                }
                "MEDIAN" => {
                    let mut v = self.collect_numbers(args)?;
                    if v.is_empty() {
                        return Err(CellError::Num);
                    }
                    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                    let m = v.len() / 2;
                    num(if v.len() % 2 == 0 { (v[m - 1] + v[m]) / 2.0 } else { v[m] })
                }
                "STDEV" | "STDEV.S" | "STDEV.P" | "STDEVP" | "VAR" | "VAR.S" | "VAR.P" | "VARP" => {
                    let v = self.collect_numbers(args)?;
                    let pop = name.ends_with('P');
                    let n = v.len() as f64;
                    if n < if pop { 1.0 } else { 2.0 } {
                        return Err(CellError::Div0);
                    }
                    let mean = v.iter().sum::<f64>() / n;
                    let ss: f64 = v.iter().map(|x| (x - mean).powi(2)).sum();
                    let var = ss / if pop { n } else { n - 1.0 };
                    num(if name.starts_with("STDEV") { var.sqrt() } else { var })
                }
                "LARGE" | "SMALL" => {
                    need(2, 2)?;
                    let mut v = self.collect_numbers(&args[..1])?;
                    let k = self.num_arg(&args[1])? as usize;
                    if k == 0 || k > v.len() {
                        return Err(CellError::Num);
                    }
                    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                    num(if name == "SMALL" { v[k - 1] } else { v[v.len() - k] })
                }
                "SUMPRODUCT" => {
                    let mut ranges = Vec::new();
                    for a in args {
                        ranges.push(self.range_arg(a)?);
                    }
                    let Some(first) = ranges.first().copied() else { return Err(CellError::Value) };
                    if ranges.iter().any(|r| r.rows() != first.rows() || r.cols() != first.cols()) {
                        return Err(CellError::Value);
                    }
                    let clipped = self.clip(first);
                    let mut total = 0.0;
                    for dr in 0..clipped.rows() {
                        for dc in 0..clipped.cols() {
                            let mut p = 1.0;
                            for r in &ranges {
                                match self.ctx.cell_value(r.sheet, r.r0 + dr, r.c0 + dc) {
                                    Value::Number(n) | Value::Date(n) => p *= n,
                                    Value::Error(e) => return Err(e),
                                    _ => p = 0.0,
                                }
                            }
                            total += p;
                        }
                    }
                    num(total)
                }
                "SUMIF" | "AVERAGEIF" | "COUNTIF" => {
                    let maxa = if name == "COUNTIF" { 2 } else { 3 };
                    need(2, maxa)?;
                    let crit_rng = self.range_arg(&args[0])?;
                    let crit = Criteria::new(&self.scalar(&args[1]));
                    let target = if argc == 3 { Some(self.range_arg(&args[2])?) } else { None };
                    self.conditional(name, &[(crit_rng, crit)], target)?
                }
                "SUMIFS" | "AVERAGEIFS" | "COUNTIFS" | "MAXIFS" | "MINIFS" => {
                    let (target, rest) = if name == "COUNTIFS" {
                        (None, args)
                    } else {
                        if argc < 3 {
                            return Err(CellError::Value);
                        }
                        (Some(self.range_arg(&args[0])?), &args[1..])
                    };
                    if rest.len() % 2 != 0 || rest.is_empty() {
                        return Err(CellError::Value);
                    }
                    let mut conds = Vec::new();
                    for pair in rest.chunks(2) {
                        let r = self.range_arg(&pair[0])?;
                        let c = Criteria::new(&self.scalar(&pair[1]));
                        conds.push((r, c));
                    }
                    self.conditional(name, &conds, target)?
                }

                // ---- Math
                "ABS" => num(self.num_arg(one(args)?)?.abs()),
                "SQRT" => {
                    let n = self.num_arg(one(args)?)?;
                    if n < 0.0 {
                        return Err(CellError::Num);
                    }
                    num(n.sqrt())
                }
                "INT" => num(self.num_arg(one(args)?)?.floor()),
                "SIGN" => num(self.num_arg(one(args)?)?.signum()),
                "EXP" => num(self.num_arg(one(args)?)?.exp()),
                "LN" => num(self.num_arg(one(args)?)?.ln()),
                "LOG10" => num(self.num_arg(one(args)?)?.log10()),
                "LOG" => {
                    need(1, 2)?;
                    let n = self.num_arg(&args[0])?;
                    let b = self.opt_num(args, 1, 10.0)?;
                    num(n.log(b))
                }
                "PI" => Value::Number(std::f64::consts::PI),
                "POWER" => {
                    need(2, 2)?;
                    num(self.num_arg(&args[0])?.powf(self.num_arg(&args[1])?))
                }
                "MOD" => {
                    need(2, 2)?;
                    let a = self.num_arg(&args[0])?;
                    let b = self.num_arg(&args[1])?;
                    if b == 0.0 {
                        return Err(CellError::Div0);
                    }
                    num(a - b * (a / b).floor())
                }
                "ROUND" | "ROUNDUP" | "ROUNDDOWN" => {
                    need(1, 2)?;
                    let n = self.num_arg(&args[0])?;
                    let d = self.opt_num(args, 1, 0.0)?.trunc() as i32;
                    let f = 10f64.powi(d);
                    let x = n * f;
                    // Nudge to avoid 2.675 -> 2.67 binary artefacts.
                    let x = if x.fract().abs() > 0.0 { (x * 1e9).round() / 1e9 } else { x };
                    let r = match name {
                        "ROUND" => x.abs().round() * x.signum(),
                        "ROUNDUP" => x.abs().ceil() * x.signum(),
                        _ => x.trunc(),
                    };
                    num(r / f)
                }
                "CEILING" | "FLOOR" | "CEILING.MATH" | "FLOOR.MATH" => {
                    need(1, 2)?;
                    let n = self.num_arg(&args[0])?;
                    let s = self.opt_num(args, 1, 1.0)?.abs();
                    if s == 0.0 {
                        return Ok(Value::Number(0.0));
                    }
                    num(if name.starts_with("CEILING") { (n / s).ceil() * s } else { (n / s).floor() * s })
                }
                "RAND" => Value::Number(pseudo_random()),
                "RANDBETWEEN" => {
                    need(2, 2)?;
                    let lo = self.num_arg(&args[0])?.ceil();
                    let hi = self.num_arg(&args[1])?.floor();
                    num((lo + (pseudo_random() * (hi - lo + 1.0)).floor()).min(hi))
                }

                // ---- Text
                "LEN" => Value::Number(self.text_arg(one(args)?)?.chars().count() as f64),
                "UPPER" => Value::text(self.text_arg(one(args)?)?.to_uppercase()),
                "LOWER" => Value::text(self.text_arg(one(args)?)?.to_lowercase()),
                "PROPER" => {
                    let s = self.text_arg(one(args)?)?;
                    let mut out = String::with_capacity(s.len());
                    let mut prev_alpha = false;
                    for c in s.chars() {
                        if prev_alpha {
                            out.extend(c.to_lowercase())
                        } else {
                            out.extend(c.to_uppercase())
                        }
                        prev_alpha = c.is_alphabetic();
                    }
                    Value::text(out)
                }
                "TRIM" => {
                    let s = self.text_arg(one(args)?)?;
                    Value::text(s.split_whitespace().collect::<Vec<_>>().join(" "))
                }
                "LEFT" | "RIGHT" => {
                    need(1, 2)?;
                    let s: Vec<char> = self.text_arg(&args[0])?.chars().collect();
                    let k = self.opt_num(args, 1, 1.0)?;
                    if k < 0.0 {
                        return Err(CellError::Value);
                    }
                    let k = (k as usize).min(s.len());
                    let part: String = if name == "LEFT" { s[..k].iter().collect() } else { s[s.len() - k..].iter().collect() };
                    Value::text(part)
                }
                "MID" => {
                    need(3, 3)?;
                    let s: Vec<char> = self.text_arg(&args[0])?.chars().collect();
                    let start = self.num_arg(&args[1])?;
                    let len = self.num_arg(&args[2])?;
                    if start < 1.0 || len < 0.0 {
                        return Err(CellError::Value);
                    }
                    let a = (start as usize - 1).min(s.len());
                    let b = (a + len as usize).min(s.len());
                    Value::text(s[a..b].iter().collect::<String>())
                }
                "CONCAT" | "CONCATENATE" => {
                    let mut out = String::new();
                    let mut err = None;
                    self.values(args, &mut |v| match v.as_text() {
                        Ok(t) => out.push_str(&t),
                        Err(e) => {
                            err.get_or_insert(e);
                        }
                    });
                    if let Some(e) = err {
                        return Err(e);
                    }
                    Value::text(out)
                }
                "TEXTJOIN" => {
                    if argc < 3 {
                        return Err(CellError::Value);
                    }
                    let sep = self.text_arg(&args[0])?;
                    let skip_empty = self.scalar(&args[1]).as_bool()?;
                    let mut parts = Vec::new();
                    let mut err = None;
                    self.values(&args[2..], &mut |v| match v.as_text() {
                        Ok(t) if !(skip_empty && t.is_empty()) => parts.push(t),
                        Ok(_) => {}
                        Err(e) => {
                            err.get_or_insert(e);
                        }
                    });
                    if let Some(e) = err {
                        return Err(e);
                    }
                    Value::text(parts.join(&sep))
                }
                "REPT" => {
                    need(2, 2)?;
                    let s = self.text_arg(&args[0])?;
                    let k = self.num_arg(&args[1])?;
                    if k < 0.0 || s.len() as f64 * k > 32767.0 {
                        return Err(CellError::Value);
                    }
                    Value::text(s.repeat(k as usize))
                }
                "FIND" | "SEARCH" => {
                    need(2, 3)?;
                    let needle = self.text_arg(&args[0])?;
                    let hay = self.text_arg(&args[1])?;
                    let start = self.opt_num(args, 2, 1.0)?;
                    if start < 1.0 {
                        return Err(CellError::Value);
                    }
                    let hay_chars: Vec<char> = hay.chars().collect();
                    let skip = start as usize - 1;
                    if skip > hay_chars.len() {
                        return Err(CellError::Value);
                    }
                    let rest: String = hay_chars[skip..].iter().collect();
                    let found = if name == "FIND" {
                        rest.split_once(needle.as_str()).map(|(before, _)| before.chars().count())
                    } else {
                        find_ignoring_case(&rest, &needle)
                    };
                    match found {
                        Some(chars) => Value::Number((chars + skip + 1) as f64),
                        None => return Err(CellError::Value),
                    }
                }
                "SUBSTITUTE" => {
                    need(3, 4)?;
                    let s = self.text_arg(&args[0])?;
                    let old = self.text_arg(&args[1])?;
                    let new = self.text_arg(&args[2])?;
                    if old.is_empty() {
                        return Ok(Value::text(s));
                    }
                    if argc == 4 {
                        let nth = self.num_arg(&args[3])? as usize;
                        let mut out = String::new();
                        let mut rest = s.as_str();
                        let mut i = 0;
                        while let Some((before, after)) = rest.split_once(old.as_str()) {
                            i += 1;
                            out.push_str(before);
                            out.push_str(if i == nth { &new } else { &old });
                            rest = after;
                        }
                        out.push_str(rest);
                        Value::text(out)
                    } else {
                        Value::text(s.replace(&old, &new))
                    }
                }
                "VALUE" => {
                    let v = self.scalar(one(args)?);
                    match v {
                        Value::Text(t) => match crate::input::parse_value(&t) {
                            Value::Number(n) | Value::Date(n) => Value::Number(n),
                            _ => return Err(CellError::Value),
                        },
                        v => Value::Number(v.as_number()?),
                    }
                }
                "TEXT" => {
                    need(2, 2)?;
                    let v = self.scalar(&args[0]);
                    let fmt = self.text_arg(&args[1])?;
                    Value::text(text_format(&v, &fmt)?)
                }
                "EXACT" => {
                    need(2, 2)?;
                    Value::Bool(self.text_arg(&args[0])? == self.text_arg(&args[1])?)
                }

                // ---- Info
                "ISBLANK" => Value::Bool(self.scalar(one(args)?).is_empty()),
                "ISNUMBER" => Value::Bool(matches!(self.scalar(one(args)?), Value::Number(_) | Value::Date(_))),
                "ISTEXT" => Value::Bool(matches!(self.scalar(one(args)?), Value::Text(_))),
                "ISLOGICAL" => Value::Bool(matches!(self.scalar(one(args)?), Value::Bool(_))),
                "ISERROR" => Value::Bool(matches!(self.scalar(one(args)?), Value::Error(_))),
                "ISERR" => Value::Bool(matches!(self.scalar(one(args)?), Value::Error(e) if e != CellError::NA)),
                "ISNA" => Value::Bool(matches!(self.scalar(one(args)?), Value::Error(CellError::NA))),
                "ROW" | "COLUMN" => {
                    if argc == 0 {
                        Value::Number(if name == "ROW" { self.row + 1 } else { self.col + 1 } as f64)
                    } else {
                        let r = self.range_arg(&args[0])?;
                        Value::Number(if name == "ROW" { r.r0 + 1 } else { r.c0 + 1 } as f64)
                    }
                }
                "ROWS" | "COLUMNS" => {
                    let r = self.range_arg(one(args)?)?;
                    Value::Number(if name == "ROWS" { r.rows() } else { r.cols() } as f64)
                }

                // ---- Date
                "TODAY" => Value::Date(self.ctx.now().floor()),
                "NOW" => Value::Date(self.ctx.now()),
                "DATE" => {
                    need(3, 3)?;
                    let y = self.num_arg(&args[0])? as i64;
                    let m = self.num_arg(&args[1])? as i64;
                    let d = self.num_arg(&args[2])? as i64;
                    let y = if y < 1900 { y + 1900 } else { y };
                    Value::Date(ymd_to_serial(y, m, d))
                }
                "YEAR" | "MONTH" | "DAY" => {
                    let n = self.num_arg(one(args)?)?;
                    let (y, m, d) = serial_to_ymd(n);
                    Value::Number(match name {
                        "YEAR" => y as f64,
                        "MONTH" => m as f64,
                        _ => d as f64,
                    })
                }
                "WEEKDAY" => {
                    need(1, 2)?;
                    let n = self.num_arg(&args[0])?.floor();
                    let t = self.opt_num(args, 1, 1.0)? as i64;
                    // Serial 1 (1900-01-01 in Excel's calendar) is a Sunday.
                    let dow = ((n as i64 - 1).rem_euclid(7)) + 1; // 1 = Sunday
                    Value::Number(match t {
                        2 => ((dow + 5) % 7 + 1) as f64,
                        3 => ((dow + 5) % 7) as f64,
                        _ => dow as f64,
                    })
                }
                "HOUR" | "MINUTE" | "SECOND" => {
                    let n = self.num_arg(one(args)?)?;
                    let secs = ((n - n.floor()) * 86400.0).round() as i64;
                    Value::Number(match name {
                        "HOUR" => secs / 3600,
                        "MINUTE" => (secs / 60) % 60,
                        _ => secs % 60,
                    } as f64)
                }
                "DAYS" => {
                    need(2, 2)?;
                    num(self.num_arg(&args[0])?.floor() - self.num_arg(&args[1])?.floor())
                }

                // ---- Lookup
                "VLOOKUP" | "HLOOKUP" => {
                    need(3, 4)?;
                    let key = self.scalar(&args[0]);
                    if let Value::Error(e) = key {
                        return Err(e);
                    }
                    let table = self.range_arg(&args[1])?;
                    let idx = self.num_arg(&args[2])? as u32;
                    let approx = match args.get(3) {
                        None | Some(Expr::Missing) => true,
                        Some(e) => self.scalar(e).as_bool()?,
                    };
                    let vertical = name == "VLOOKUP";
                    let span = if vertical { table.cols() } else { table.rows() };
                    if idx < 1 || idx > span {
                        return Err(CellError::Ref);
                    }
                    let t = self.clip(table);
                    let n = if vertical { t.rows() } else { t.cols() };
                    let at = |i: u32| if vertical { (table.r0 + i, table.c0) } else { (table.r0, table.c0 + i) };
                    let hit = self.lookup_index(&key, n, &at, if approx { 1 } else { 0 }, table.sheet)?;
                    let (r, c) = at(hit);
                    let (r, c) = if vertical { (r, c + idx - 1) } else { (r + idx - 1, c) };
                    self.ctx.cell_value(table.sheet, r, c)
                }
                "MATCH" => {
                    need(2, 3)?;
                    let key = self.scalar(&args[0]);
                    let rng = self.range_arg(&args[1])?;
                    let mode = self.opt_num(args, 2, 1.0)? as i32;
                    if rng.rows() > 1 && rng.cols() > 1 {
                        return Err(CellError::NA);
                    }
                    let t = self.clip(rng);
                    let vertical = rng.cols() == 1;
                    let n = if vertical { t.rows() } else { t.cols() };
                    let at = |i: u32| if vertical { (rng.r0 + i, rng.c0) } else { (rng.r0, rng.c0 + i) };
                    Value::Number(self.lookup_index(&key, n, &at, mode, rng.sheet)? as f64 + 1.0)
                }
                "INDEX" => {
                    need(2, 3)?;
                    let rng = self.range_arg(&args[0])?;
                    let mut r = self.num_arg(&args[1])? as u32;
                    let mut c = self.opt_num(args, 2, 0.0)? as u32;
                    if rng.rows() == 1 && argc == 2 {
                        c = r;
                        r = 1;
                    }
                    let r = r.max(1);
                    let c = c.max(1);
                    if r > rng.rows() || c > rng.cols() {
                        return Err(CellError::Ref);
                    }
                    self.ctx.cell_value(rng.sheet, rng.r0 + r - 1, rng.c0 + c - 1)
                }
                "XLOOKUP" => {
                    need(3, 6)?;
                    let key = self.scalar(&args[0]);
                    let look = self.range_arg(&args[1])?;
                    let ret = self.range_arg(&args[2])?;
                    let mode = self.opt_num(args, 4, 0.0)? as i32;
                    let vertical = look.cols() == 1;
                    let t = self.clip(look);
                    let n = if vertical { t.rows() } else { t.cols() };
                    let at = |i: u32| if vertical { (look.r0 + i, look.c0) } else { (look.r0, look.c0 + i) };
                    let found = match mode {
                        0 | 2 => self.lookup_index(&key, n, &at, 0, look.sheet),
                        -1 => self.lookup_index(&key, n, &at, 1, look.sheet),
                        _ => self.lookup_index(&key, n, &at, -1, look.sheet),
                    };
                    match found {
                        Ok(i) => {
                            if vertical {
                                self.ctx.cell_value(ret.sheet, ret.r0 + i, ret.c0)
                            } else {
                                self.ctx.cell_value(ret.sheet, ret.r0, ret.c0 + i)
                            }
                        }
                        Err(CellError::NA) if argc >= 4 && !matches!(args[3], Expr::Missing) => self.scalar(&args[3]),
                        Err(e) => return Err(e),
                    }
                }
                "CHOOSE" => {
                    if argc < 2 {
                        return Err(CellError::Value);
                    }
                    let i = self.num_arg(&args[0])? as usize;
                    if i < 1 || i >= argc {
                        return Err(CellError::Value);
                    }
                    return Ok(self.scalar(&args[i]));
                }
                _ => Value::Error(CellError::Name),
            })
        })();
        Ev::V(match res {
            Ok(v) => v,
            Err(e) => Value::Error(e),
        })
    }

    /// mode 0 = exact (wildcards for text), 1 = largest <= key, -1 = smallest >= key.
    fn lookup_index(&mut self, key: &Value, n: u32, at: &dyn Fn(u32) -> (u32, u32), mode: i32, sheet: usize) -> R<u32> {
        let crit = if mode == 0 { Some(Criteria { op: Op::Eq, val: key.clone() }) } else { None };
        let mut best: Option<(u32, Value)> = None;
        for i in 0..n {
            let (r, c) = at(i);
            let v = self.ctx.cell_value(sheet, r, c);
            if let Some(cr) = &crit {
                if cr.matches(&v) {
                    return Ok(i);
                }
                continue;
            }
            let same_type = matches!(
                (&v, key),
                (Value::Number(_) | Value::Date(_), Value::Number(_) | Value::Date(_))
                    | (Value::Text(_), Value::Text(_))
                    | (Value::Bool(_), Value::Bool(_))
            );
            if !same_type {
                continue;
            }
            let ord = v.sort_cmp(key);
            if ord == Ordering::Equal {
                return Ok(i);
            }
            let ok = if mode > 0 { ord == Ordering::Less } else { ord == Ordering::Greater };
            if ok {
                let better = match &best {
                    None => true,
                    Some((_, b)) => {
                        if mode > 0 {
                            v.sort_cmp(b) == Ordering::Greater
                        } else {
                            v.sort_cmp(b) == Ordering::Less
                        }
                    }
                };
                if better {
                    best = Some((i, v));
                }
            }
        }
        best.map(|(i, _)| i).ok_or(CellError::NA)
    }

    fn conditional(&mut self, name: &str, conds: &[(Rng, Criteria)], target: Option<Rng>) -> R<Value> {
        let (first, _) = conds[0];
        // Iterate the clipped first condition range; empty cells only match when criteria accepts empty.
        let clipped = self.clip(first);
        let accepts_empty = conds[0].1.matches(&Value::Empty);
        let mut candidates: Vec<(u32, u32)> = Vec::new();
        if accepts_empty {
            for r in 0..clipped.rows() {
                for c in 0..clipped.cols() {
                    candidates.push((r, c));
                }
            }
        } else {
            let crit = &conds[0].1;
            self.ctx.for_each(&clipped, &mut |r, c, v| {
                if crit.matches(v) {
                    candidates.push((r - first.r0, c - first.c0));
                }
            });
            // Already matched the first condition: skip it below.
        }
        let (mut sum, mut count) = (0.0, 0usize);
        let mut extreme: Option<f64> = None;
        'outer: for (dr, dc) in candidates {
            for (i, (rng, crit)) in conds.iter().enumerate() {
                if i == 0 && !accepts_empty {
                    continue;
                }
                if dr >= rng.rows() || dc >= rng.cols() {
                    continue 'outer;
                }
                let v = self.ctx.cell_value(rng.sheet, rng.r0 + dr, rng.c0 + dc);
                if !crit.matches(&v) {
                    continue 'outer;
                }
            }
            match target {
                None => count += 1,
                Some(t) => {
                    let v = self.ctx.cell_value(t.sheet, t.r0 + dr, t.c0 + dc);
                    match v {
                        Value::Number(n) | Value::Date(n) => {
                            sum += n;
                            count += 1;
                            extreme = Some(match extreme {
                                None => n,
                                Some(e) if name.starts_with("MAX") => e.max(n),
                                Some(e) => e.min(n),
                            });
                        }
                        Value::Error(e) => return Err(e),
                        _ => {}
                    }
                }
            }
        }
        Ok(match name {
            "COUNTIF" | "COUNTIFS" => Value::Number(count as f64),
            "SUMIF" | "SUMIFS" => num(sum),
            "MAXIFS" | "MINIFS" => num(extreme.unwrap_or(0.0)),
            _ => {
                if count == 0 {
                    return Err(CellError::Div0);
                }
                num(sum / count as f64)
            }
        })
    }
}

fn one(args: &[Expr]) -> R<&Expr> {
    if args.len() == 1 {
        Ok(&args[0])
    } else {
        Err(CellError::Value)
    }
}

/// SEARCH: the character position of `needle` in `hay`, ignoring case. Lowercasing can change a
/// character's length ("İ" becomes two characters), so positions are mapped back to `hay`.
fn find_ignoring_case(hay: &str, needle: &str) -> Option<usize> {
    let needle = needle.to_lowercase();
    let mut low = String::with_capacity(hay.len());
    // (byte offset in `low`, character index in `hay`) where each lowercased character starts.
    let mut starts = Vec::with_capacity(hay.len());
    for (i, ch) in hay.chars().enumerate() {
        starts.push((low.len(), i));
        low.extend(ch.to_lowercase());
    }
    let byte = low.find(&needle)?;
    // A match that starts inside a lowercased character counts from that character.
    let at = starts.partition_point(|&(b, _)| b <= byte);
    Some(starts.get(at.wrapping_sub(1)).map_or(0, |&(_, i)| i))
}

fn pseudo_random() -> f64 {
    use std::cell::Cell;
    use std::hash::{BuildHasher, Hasher};
    // A different seed in every run, without reading the clock: std seeds its hashers randomly.
    // Never 0, which xorshift would never leave.
    thread_local!(static STATE: Cell<u64> = Cell::new(std::collections::hash_map::RandomState::new().build_hasher().finish().max(1)));
    STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

fn compare(a: &Value, b: &Value) -> R<Ordering> {
    if let Value::Error(e) = a {
        return Err(*e);
    }
    if let Value::Error(e) = b {
        return Err(*e);
    }
    // Empty compares as 0, "" or FALSE depending on the other side.
    let norm = |v: &Value, other: &Value| -> Value {
        if v.is_empty() {
            match other {
                Value::Text(_) => Value::text(""),
                Value::Bool(_) => Value::Bool(false),
                _ => Value::Number(0.0),
            }
        } else {
            v.clone()
        }
    };
    let a = norm(a, b);
    let b = norm(&b.clone(), &a);
    let rank = |v: &Value| match v {
        Value::Number(_) | Value::Date(_) | Value::Empty => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
    };
    Ok(match (&a, &b) {
        (Value::Number(x) | Value::Date(x), Value::Number(y) | Value::Date(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Text(x), Value::Text(y)) => compare_text(x, y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => rank(&a).cmp(&rank(&b)),
    })
}

fn binary(op: &str, a: &Value, b: &Value) -> Value {
    let arith = |f: fn(f64, f64) -> f64| -> Value {
        match (a.as_number(), b.as_number()) {
            (Ok(x), Ok(y)) => num(f(x, y)),
            (Err(e), _) | (_, Err(e)) => Value::Error(e),
        }
    };
    match op {
        "+" => arith(|x, y| x + y),
        "-" => arith(|x, y| x - y),
        "*" => arith(|x, y| x * y),
        "/" => match (a.as_number(), b.as_number()) {
            (Ok(_), Ok(0.0)) => Value::Error(CellError::Div0),
            (Ok(x), Ok(y)) => num(x / y),
            (Err(e), _) | (_, Err(e)) => Value::Error(e),
        },
        "^" => arith(f64::powf),
        "&" => match (a.as_text(), b.as_text()) {
            (Ok(x), Ok(y)) => Value::Text(Arc::from(format!("{x}{y}").as_str())),
            (Err(e), _) | (_, Err(e)) => Value::Error(e),
        },
        _ => match compare(a, b) {
            Ok(o) => Value::Bool(match op {
                "=" => o == Ordering::Equal,
                "<>" => o != Ordering::Equal,
                "<" => o == Ordering::Less,
                ">" => o == Ordering::Greater,
                "<=" => o != Ordering::Greater,
                ">=" => o != Ordering::Less,
                _ => false,
            }),
            Err(e) => Value::Error(e),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

/// A COUNTIF-style criterion such as ">5", "<>apple", "a*".
#[derive(Clone, Debug)]
pub struct Criteria {
    op: Op,
    val: Value,
}

impl Criteria {
    pub fn new(v: &Value) -> Criteria {
        let Value::Text(t) = v else {
            return Criteria { op: Op::Eq, val: v.clone() };
        };
        let (op, rest) = if let Some(r) = t.strip_prefix("<>") {
            (Op::Ne, r)
        } else if let Some(r) = t.strip_prefix(">=") {
            (Op::Ge, r)
        } else if let Some(r) = t.strip_prefix("<=") {
            (Op::Le, r)
        } else if let Some(r) = t.strip_prefix('=') {
            (Op::Eq, r)
        } else if let Some(r) = t.strip_prefix('>') {
            (Op::Gt, r)
        } else if let Some(r) = t.strip_prefix('<') {
            (Op::Lt, r)
        } else {
            (Op::Eq, t.as_ref())
        };
        let val = if rest.is_empty() {
            Value::Empty
        } else {
            match crate::input::parse_value(rest) {
                Value::Text(_) => Value::text(rest),
                other => other,
            }
        };
        Criteria { op, val }
    }

    pub fn matches(&self, v: &Value) -> bool {
        match (&self.val, self.op) {
            (Value::Empty, Op::Eq) => return v.is_empty() || matches!(v, Value::Text(t) if t.is_empty()),
            (Value::Empty, Op::Ne) => return !v.is_empty(),
            (Value::Text(p), Op::Eq | Op::Ne) => {
                let m = matches!(v, Value::Text(t) if wildcard_match(p, t))
                    || (!matches!(v, Value::Text(_)) && !v.is_empty() && wildcard_match(p, &v.display()));
                return if self.op == Op::Eq { m } else { !m };
            }
            _ => {}
        }
        let comparable = matches!(
            (&self.val, v),
            (Value::Number(_) | Value::Date(_), Value::Number(_) | Value::Date(_))
                | (Value::Text(_), Value::Text(_))
                | (Value::Bool(_), Value::Bool(_))
                | (Value::Error(_), Value::Error(_))
        );
        if !comparable {
            return self.op == Op::Ne;
        }
        let o = match (v, &self.val) {
            (Value::Error(a), Value::Error(b)) => {
                if a == b {
                    Ordering::Equal
                } else {
                    Ordering::Less
                }
            }
            _ => v.sort_cmp(&self.val),
        };
        match self.op {
            Op::Eq => o == Ordering::Equal,
            Op::Ne => o != Ordering::Equal,
            Op::Lt => o == Ordering::Less,
            Op::Gt => o == Ordering::Greater,
            Op::Le => o != Ordering::Greater,
            Op::Ge => o != Ordering::Less,
        }
    }
}

/// Case-insensitive match with `*`, `?` and `~` escapes.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    if !p.iter().any(|c| matches!(c, '*' | '?' | '~')) {
        return p == t;
    }
    // Tokenise pattern: Some(c) literal, None for '?', and '*' marker.
    #[derive(Clone, Copy, PartialEq)]
    enum P {
        Lit(char),
        One,
        Any,
    }
    let mut toks = Vec::new();
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            '~' if i + 1 < p.len() => {
                toks.push(P::Lit(p[i + 1]));
                i += 2;
                continue;
            }
            '*' => toks.push(P::Any),
            '?' => toks.push(P::One),
            c => toks.push(P::Lit(c)),
        }
        i += 1;
    }
    let (mut ti, mut pi) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ti < t.len() {
        if pi < toks.len() && (toks[pi] == P::One || toks[pi] == P::Lit(t[ti])) {
            ti += 1;
            pi += 1;
        } else if pi < toks.len() && toks[pi] == P::Any {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < toks.len() && toks[pi] == P::Any {
        pi += 1;
    }
    pi == toks.len()
}

/// A small subset of Excel's TEXT() format codes.
fn text_format(v: &Value, fmt: &str) -> R<String> {
    let lower = fmt.to_ascii_lowercase();
    if lower.contains('y') || lower.contains('d') || (lower.contains('m') && !lower.contains('0')) {
        let n = v.as_number()?;
        let (y, m, d) = serial_to_ymd(n);
        let secs = ((n - n.floor()) * 86400.0).round() as i64;
        let mut out = lower.clone();
        let reps: [(&str, String); 9] = [
            ("yyyy", format!("{:04}", y)),
            ("yy", format!("{:02}", y % 100)),
            ("hh", format!("{:02}", secs / 3600)),
            ("ss", format!("{:02}", secs % 60)),
            ("mm", format!("{:02}", m)),
            ("dd", format!("{:02}", d)),
            ("m", m.to_string()),
            ("d", d.to_string()),
            ("nn", format!("{:02}", (secs / 60) % 60)),
        ];
        // Replace longest tokens first, using placeholders to avoid re-replacing digits.
        let mut placeholders = Vec::new();
        for (i, (k, val)) in reps.iter().enumerate() {
            let ph = format!("\u{1}{i}\u{2}");
            if out.contains(k) {
                out = out.replace(k, &ph);
                placeholders.push((ph, val.clone()));
            }
        }
        for (ph, val) in placeholders {
            out = out.replace(&ph, &val);
        }
        return Ok(out);
    }
    let n = match v {
        Value::Text(t) => return Ok(t.to_string()),
        v => v.as_number()?,
    };
    let pct = fmt.contains('%');
    let n = if pct { n * 100.0 } else { n };
    let decimals = fmt.split_once('.').map(|(_, d)| d.chars().filter(|c| *c == '0' || *c == '#').count()).unwrap_or(0);
    let grouped = fmt.contains(',');
    let mut s = format!("{:.*}", decimals, n.abs());
    if grouped {
        let (int, frac) = match s.split_once('.') {
            Some((i, f)) => (i.to_string(), Some(f.to_string())),
            None => (s.clone(), None),
        };
        let mut g = String::new();
        for (i, ch) in int.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                g.push(',');
            }
            g.push(ch);
        }
        s = g.chars().rev().collect();
        if let Some(f) = frac {
            s = format!("{s}.{f}");
        }
    }
    if n < 0.0 && s.chars().any(|c| c != '0' && c != '.' && c != ',') {
        s = format!("-{s}");
    }
    if pct {
        s.push('%');
    }
    let _ = (format_date, format_number);
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard_match("a*", "Apple"));
        assert!(wildcard_match("?pple", "apple"));
        assert!(!wildcard_match("b*", "apple"));
        assert!(wildcard_match("*~*", "a*"));
        assert!(wildcard_match("apple", "APPLE"));
    }

    #[test]
    fn criteria() {
        let c = Criteria::new(&Value::text(">5"));
        assert!(c.matches(&Value::Number(6.0)));
        assert!(!c.matches(&Value::Number(5.0)));
        assert!(!c.matches(&Value::text("x")));
        let c = Criteria::new(&Value::text("<>x"));
        assert!(c.matches(&Value::text("y")));
        assert!(!c.matches(&Value::text("X")));
    }

    #[test]
    fn text_formats() {
        assert_eq!(text_format(&Value::Number(1234.567), "#,##0.00").unwrap(), "1,234.57");
        assert_eq!(text_format(&Value::Number(0.256), "0%").unwrap(), "26%");
        assert_eq!(text_format(&Value::Number(46353.0), "dd/mm/yyyy").unwrap(), "27/11/2026");
    }
}
