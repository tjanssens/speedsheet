//! Cell values and display formatting.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

/// Spreadsheet error values, as in Excel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellError {
    Div0,
    NA,
    Name,
    Null,
    Num,
    Ref,
    Value,
    Cycle,
}

impl CellError {
    pub fn as_str(self) -> &'static str {
        match self {
            CellError::Div0 => "#DIV/0!",
            CellError::NA => "#N/A",
            CellError::Name => "#NAME?",
            CellError::Null => "#NULL!",
            CellError::Num => "#NUM!",
            CellError::Ref => "#REF!",
            CellError::Value => "#VALUE!",
            CellError::Cycle => "#CYCLE!",
        }
    }

    pub fn parse(s: &str) -> Option<CellError> {
        Some(match s.to_ascii_uppercase().as_str() {
            "#DIV/0!" => CellError::Div0,
            "#N/A" => CellError::NA,
            "#NAME?" => CellError::Name,
            "#NULL!" => CellError::Null,
            "#NUM!" => CellError::Num,
            "#REF!" => CellError::Ref,
            "#VALUE!" => CellError::Value,
            "#CYCLE!" => CellError::Cycle,
            _ => return None,
        })
    }
}

impl fmt::Display for CellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A computed (non-formula) value.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Empty,
    Number(f64),
    /// Excel serial date/time (days since 1899-12-30), shown as a date.
    Date(f64),
    Text(Arc<str>),
    Bool(bool),
    Error(CellError),
}

impl Value {
    pub fn text(s: impl Into<Arc<str>>) -> Value {
        Value::Text(s.into())
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }

    /// Numeric view used by arithmetic. Empty is 0, bools are 0/1, numeric text is parsed.
    pub fn as_number(&self) -> Result<f64, CellError> {
        match self {
            Value::Empty => Ok(0.0),
            Value::Number(n) | Value::Date(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Text(s) => {
                let t = s.trim();
                if t.is_empty() {
                    return Ok(0.0);
                }
                t.parse::<f64>().map_err(|_| CellError::Value)
            }
            Value::Error(e) => Err(*e),
        }
    }

    pub fn as_bool(&self) -> Result<bool, CellError> {
        match self {
            Value::Empty => Ok(false),
            Value::Number(n) | Value::Date(n) => Ok(*n != 0.0),
            Value::Bool(b) => Ok(*b),
            Value::Text(s) => match s.to_ascii_uppercase().as_str() {
                "TRUE" => Ok(true),
                "FALSE" => Ok(false),
                _ => Err(CellError::Value),
            },
            Value::Error(e) => Err(*e),
        }
    }

    /// Text view used by string functions and concatenation.
    pub fn as_text(&self) -> Result<String, CellError> {
        match self {
            Value::Error(e) => Err(*e),
            Value::Number(n) => Ok(format_number(*n)),
            other => Ok(other.display()),
        }
    }

    /// Human readable representation shown in the grid.
    pub fn display(&self) -> String {
        match self {
            Value::Empty => String::new(),
            Value::Number(n) => format_number(*n),
            Value::Date(n) => format_date(*n),
            Value::Text(s) => s.to_string(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Value::Error(e) => e.as_str().to_string(),
        }
    }

    /// Kind code used by the UI for alignment/colouring.
    pub fn kind(&self) -> u8 {
        match self {
            Value::Empty => 0,
            Value::Number(_) | Value::Date(_) => 1,
            Value::Text(_) => 2,
            Value::Bool(_) => 3,
            Value::Error(_) => 4,
        }
    }

    /// Ordering used by sort: numbers < text < bools < errors < empty (empty is always last).
    pub fn sort_cmp(&self, other: &Value) -> Ordering {
        fn rank(v: &Value) -> u8 {
            match v {
                Value::Number(_) | Value::Date(_) => 0,
                Value::Text(_) => 1,
                Value::Bool(_) => 2,
                Value::Error(_) => 3,
                Value::Empty => 4,
            }
        }
        match (self, other) {
            (Value::Number(a) | Value::Date(a), Value::Number(b) | Value::Date(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Value::Text(a), Value::Text(b)) => compare_text(a, b),
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            _ => rank(self).cmp(&rank(other)),
        }
    }
}

/// Case-insensitive comparison with a cheap ASCII fast path.
pub fn compare_text(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        let ord = a.bytes().map(|c| c.to_ascii_lowercase()).cmp(b.bytes().map(|c| c.to_ascii_lowercase()));
        return ord;
    }
    a.to_lowercase().cmp(&b.to_lowercase())
}

/// Format a number like a spreadsheet's "General" format: up to ~11 significant digits.
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return CellError::Num.as_str().to_string();
    }
    if n.is_infinite() {
        return CellError::Div0.as_str().to_string();
    }
    if n == 0.0 {
        return "0".to_string();
    }
    let abs = n.abs();
    if n.fract() == 0.0 && abs < 1e15 {
        return format!("{}", n as i64);
    }
    if !(1e-9..1e15).contains(&abs) {
        let s = format!("{:.5E}", n);
        // Trim mantissa zeros: 1.50000E3 -> 1.5E3
        if let Some((m, e)) = s.split_once('E') {
            let m = m.trim_end_matches('0').trim_end_matches('.');
            let exp: i32 = e.parse().unwrap_or(0);
            return format!("{}E{}{:02}", m, if exp < 0 { '-' } else { '+' }, exp.abs());
        }
        return s;
    }
    // Number of decimals so that total significant digits is about 11.
    let int_digits = if abs >= 1.0 { abs.log10().floor() as i32 + 1 } else { 0 };
    let decimals = (11 - int_digits).clamp(0, 10) as usize;
    let s = format!("{:.*}", decimals, n);
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// Convert an Excel serial date into (year, month, day).
pub fn serial_to_ymd(serial: f64) -> (i64, u32, u32) {
    // Excel serial 25569 is 1970-01-01. Serial 60 is the fictional 1900-02-29; ignore it.
    let days = serial.floor() as i64 - 25569;
    civil_from_days(days)
}

/// Convert (year, month, day) into an Excel serial date.
pub fn ymd_to_serial(y: i64, m: i64, d: i64) -> f64 {
    // Normalise month overflow like Excel's DATE(2024, 14, 1).
    let total_months = y * 12 + (m - 1);
    let y2 = total_months.div_euclid(12);
    let m2 = total_months.rem_euclid(12) + 1;
    (days_from_civil(y2, m2 as u32, 1) + (d - 1) + 25569) as f64
}

// Algorithms from Howard Hinnant's date library (public domain).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn format_date(serial: f64) -> String {
    let (y, m, d) = serial_to_ymd(serial);
    let frac = serial - serial.floor();
    if frac.abs() < 1e-9 {
        format!("{:04}-{:02}-{:02}", y, m, d)
    } else {
        let secs = (frac * 86400.0).round() as i64;
        let (h, mi, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
        if serial < 1.0 {
            format!("{:02}:{:02}:{:02}", h, mi, s)
        } else {
            format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, h, mi, s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_format_like_general() {
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(-12.5), "-12.5");
        assert_eq!(format_number(1.0 / 3.0), "0.3333333333");
        assert_eq!(format_number(1e20), "1E+20");
    }

    #[test]
    fn dates_round_trip() {
        let s = ymd_to_serial(2026, 11, 27);
        assert_eq!(s, 46353.0);
        assert_eq!(serial_to_ymd(s), (2026, 11, 27));
        assert_eq!(format_date(45292.5), "2024-01-01 12:00:00");
    }
}
