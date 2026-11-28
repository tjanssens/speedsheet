//! Parsing of what the user types (or pastes) into a cell.

use crate::sheet::Cell;
use crate::value::{ymd_to_serial, CellError, Value};

/// Turn user input into a cell. Formulas start with '='; a leading apostrophe forces text.
pub fn parse_input(s: &str) -> Cell {
    if s.is_empty() {
        return Cell::Empty;
    }
    if let Some(rest) = s.strip_prefix('\'') {
        return Cell::from_value(Value::text(rest));
    }
    if let Some(src) = s.strip_prefix('=').filter(|f| !f.is_empty()) {
        // Value is filled in by recalculation.
        return Cell::formula(src.trim(), Value::Empty);
    }
    Cell::from_value(parse_value(s))
}

/// Parse a literal (non-formula) value.
pub fn parse_value(s: &str) -> Value {
    let t = s.trim();
    if t.is_empty() {
        return Value::text(s);
    }
    let first = t.as_bytes()[0];
    if first.is_ascii_digit() || matches!(first, b'-' | b'+' | b'.' | b',') {
        if let Some(n) = parse_number(t) {
            return Value::Number(n);
        }
        if first.is_ascii_digit() {
            if let Some(d) = parse_date(t) {
                return Value::Date(d);
            }
        }
        return Value::text(s);
    }
    if first == b'#' {
        if let Some(e) = CellError::parse(t) {
            return Value::Error(e);
        }
    }
    if (4..=6).contains(&t.len()) {
        match t.to_ascii_uppercase().as_str() {
            "TRUE" | "WAAR" => return Value::Bool(true),
            "FALSE" | "ONWAAR" => return Value::Bool(false),
            _ => {}
        }
    }
    Value::text(s)
}

/// Numbers: "12", "-3.5", "3,5" (decimal comma), "1e3", "50%", "1 000".
pub fn parse_number(t: &str) -> Option<f64> {
    let (body, pct) = match t.strip_suffix('%') {
        Some(b) => (b.trim_end(), true),
        None => (t, false),
    };
    if body.is_empty() {
        return None;
    }
    let first = body.as_bytes()[0];
    if !(first.is_ascii_digit() || first == b'-' || first == b'+' || first == b'.' || first == b',') {
        return None;
    }
    let mut n = body.parse::<f64>().ok();
    if n.is_none() {
        // Decimal comma: exactly one comma and no dot, e.g. 3,5
        let commas = body.matches(',').count();
        if commas == 1 && !body.contains('.') {
            n = body.replace(',', ".").parse::<f64>().ok();
        } else if body.contains(' ') || body.contains('\u{a0}') {
            let cleaned: String = body.chars().filter(|c| *c != ' ' && *c != '\u{a0}').collect();
            n = cleaned.replace(',', ".").parse::<f64>().ok();
        }
    }
    let n = n.filter(|v| v.is_finite())?;
    // Reject things like "inf"/"nan" that Rust parses.
    if body.chars().any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E') {
        return None;
    }
    Some(if pct { n / 100.0 } else { n })
}

/// Dates: ISO "2026-11-27" (optionally with " HH:MM[:SS]") and European "27/11/2026" or "27-11-2026".
pub fn parse_date(t: &str) -> Option<f64> {
    let (date_part, time_part) = match t.split_once(' ') {
        Some((d, tm)) => (d, Some(tm.trim())),
        None => (t, None),
    };
    let parts: Vec<&str> = date_part.split(['-', '/', '.']).collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let nums: Vec<i64> = parts.iter().map(|p| p.parse().unwrap_or(0)).collect();
    let (y, m, d) = if parts[0].len() == 4 {
        (nums[0], nums[1], nums[2])
    } else if parts[2].len() == 4 {
        (nums[2], nums[1], nums[0])
    } else {
        return None;
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1900..=9999).contains(&y) {
        return None;
    }
    let mut serial = ymd_to_serial(y, m, d);
    if let Some(tm) = time_part {
        let tp: Vec<&str> = tm.split(':').collect();
        if !(2..=3).contains(&tp.len()) {
            return None;
        }
        let h: f64 = tp[0].parse().ok()?;
        let mi: f64 = tp[1].parse().ok()?;
        let s: f64 = if tp.len() == 3 { tp[2].parse().ok()? } else { 0.0 };
        serial += (h * 3600.0 + mi * 60.0 + s) / 86400.0;
    }
    Some(serial)
}

/// True when this text, typed back in, would not stay text.
pub fn would_parse_as_non_text(t: &str) -> bool {
    t.starts_with('=') || t.starts_with('\'') || !matches!(parse_value(t), Value::Text(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inputs() {
        assert_eq!(parse_value("12"), Value::Number(12.0));
        assert_eq!(parse_value("3,5"), Value::Number(3.5));
        assert_eq!(parse_value("-1.25e2"), Value::Number(-125.0));
        assert_eq!(parse_value("50%"), Value::Number(0.5));
        assert_eq!(parse_value("true"), Value::Bool(true));
        assert_eq!(parse_value("inf"), Value::text("inf"));
        assert_eq!(parse_value("27/11/2026"), Value::Date(46353.0));
        assert_eq!(parse_value("2026-11-27"), Value::Date(46353.0));
        assert_eq!(parse_value("hello"), Value::text("hello"));
        assert!(matches!(parse_input("=A1+1"), Cell::Formula(_)));
        assert_eq!(parse_input("'12"), Cell::Value(Value::text("12")));
    }
}
