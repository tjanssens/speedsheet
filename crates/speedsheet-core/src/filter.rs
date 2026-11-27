//! Non-destructive row filtering.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::sheet::Sheet;
use crate::value::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FilterOp {
    Contains {
        value: String,
    },
    NotContains {
        value: String,
    },
    Equals {
        value: String,
    },
    NotEquals {
        value: String,
    },
    Gt {
        value: String,
    },
    Lt {
        value: String,
    },
    Ge {
        value: String,
    },
    Le {
        value: String,
    },
    Empty,
    NotEmpty,
    /// Keep rows whose displayed value is one of these (Excel's checkbox list).
    In {
        values: Vec<String>,
    },
    /// Keep rows whose displayed value is none of these.
    NotIn {
        values: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterCond {
    pub col: usize,
    #[serde(flatten)]
    pub op: FilterOp,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    /// First row is a header and is always shown.
    pub header: bool,
    pub conds: Vec<FilterCond>,
}

enum Compiled {
    Contains(String, bool),
    Equals(String, bool),
    Cmp(Value, std::cmp::Ordering, bool),
    Empty(bool),
    In(HashSet<String>, bool),
}

fn compile(op: &FilterOp) -> Compiled {
    use std::cmp::Ordering::*;
    let cmp = |v: &str, o, eq| Compiled::Cmp(crate::input::parse_value(v), o, eq);
    match op {
        FilterOp::Contains { value } => Compiled::Contains(value.to_lowercase(), true),
        FilterOp::NotContains { value } => Compiled::Contains(value.to_lowercase(), false),
        FilterOp::Equals { value } => Compiled::Equals(value.to_lowercase(), true),
        FilterOp::NotEquals { value } => Compiled::Equals(value.to_lowercase(), false),
        FilterOp::Gt { value } => cmp(value, Greater, false),
        FilterOp::Ge { value } => cmp(value, Greater, true),
        FilterOp::Lt { value } => cmp(value, Less, false),
        FilterOp::Le { value } => cmp(value, Less, true),
        FilterOp::Empty => Compiled::Empty(true),
        FilterOp::NotEmpty => Compiled::Empty(false),
        FilterOp::In { values } => Compiled::In(values.iter().cloned().collect(), true),
        FilterOp::NotIn { values } => Compiled::In(values.iter().cloned().collect(), false),
    }
}

impl Compiled {
    fn matches(&self, v: &Value) -> bool {
        match self {
            Compiled::Contains(needle, want) => contains_ci(&v.display(), needle) == *want,
            Compiled::Equals(needle, want) => (v.display().to_lowercase() == *needle) == *want,
            Compiled::Cmp(target, ord, or_eq) => {
                let comparable = matches!(
                    (v, target),
                    (Value::Number(_) | Value::Date(_), Value::Number(_) | Value::Date(_)) | (Value::Text(_), Value::Text(_))
                );
                if !comparable {
                    return false;
                }
                let o = v.sort_cmp(target);
                o == *ord || (*or_eq && o == std::cmp::Ordering::Equal)
            }
            Compiled::Empty(want) => v.is_empty() == *want,
            Compiled::In(set, want) => set.contains(&v.display()) == *want,
        }
    }
}

/// Case-insensitive substring test; `needle` must already be lowercase.
pub fn contains_ci(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if hay.is_ascii() && needle.is_ascii() {
        let h = hay.as_bytes();
        let n = needle.as_bytes();
        if n.len() > h.len() {
            return false;
        }
        return h.windows(n.len()).any(|w| w.iter().zip(n).all(|(a, b)| a.to_ascii_lowercase() == *b));
    }
    hay.to_lowercase().contains(needle)
}

/// Compute visible data rows for a filter.
pub fn compute_view(sheet: &Sheet, filter: &Filter) -> Vec<u32> {
    let compiled: Vec<(usize, Compiled)> = filter.conds.iter().map(|c| (c.col, compile(&c.op))).collect();
    let mut out = Vec::with_capacity(sheet.row_count());
    for r in 0..sheet.row_count() {
        if (filter.header && r == 0) || compiled.iter().all(|(c, m)| m.matches(sheet.value(r, *c))) {
            out.push(r as u32);
        }
    }
    out
}

/// Distinct displayed values in a column with their counts, in sort order.
/// Respects other columns' filters (like Excel), but not the filter on this column.
pub fn column_values(sheet: &Sheet, col: usize, header: bool, limit: usize) -> (Vec<(String, usize)>, bool) {
    let others: Vec<(usize, Compiled)> = sheet
        .filter
        .as_ref()
        .map(|f| f.conds.iter().filter(|c| c.col != col).map(|c| (c.col, compile(&c.op))).collect())
        .unwrap_or_default();
    let mut counts: HashMap<String, usize> = HashMap::new();
    let start = if header { 1 } else { 0 };
    for r in start..sheet.row_count() {
        if others.iter().all(|(c, m)| m.matches(sheet.value(r, *c))) {
            *counts.entry(sheet.value(r, col).display()).or_default() += 1;
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| match (crate::input::parse_number(&a.0), crate::input::parse_number(&b.0)) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => crate::value::compare_text(&a.0, &b.0),
    });
    let truncated = v.len() > limit;
    v.truncate(limit);
    (v, truncated)
}
