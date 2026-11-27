//! Excel number format codes: enough of them to display values the way Excel does.
//!
//! Supports sections (`pos;neg;zero;text`), literals (`"text"`, `\x`, `_x`, `*x`), colors and
//! conditions in brackets (ignored), currency tags like `[$€-813]`, digit placeholders `0 # ?`,
//! thousands separators and scaling commas, percent, scientific notation, `@`, and dates/times
//! (`yyyy mm dd hh:mm:ss AM/PM [h]` and friends).

use crate::value::{format_number, serial_to_ymd, Value};

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Format a value with a number format code. Text and other non-numbers are shown as-is
/// unless the format has a text section.
pub fn format_value(v: &Value, code: &str) -> String {
    match v {
        Value::Number(n) | Value::Date(n) => format_num(*n, code, matches!(v, Value::Date(_))),
        Value::Text(t) => {
            let sections = split_sections(code);
            match sections.iter().find(|s| s.contains('@')).or(if sections.len() >= 4 { sections.get(3) } else { None }) {
                Some(sec) => render_literal(sec, Some(t)),
                None => t.to_string(),
            }
        }
        other => other.display(),
    }
}

fn split_sections(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = code.as_bytes();
    let mut start = 0;
    let mut i = 0;
    let mut quoted = false;
    while i < b.len() {
        match b[i] {
            b'"' => quoted = !quoted,
            b'\\' if !quoted => i += 1,
            b';' if !quoted => {
                out.push(&code[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&code[start..]);
    out
}

/// A format section broken into literal text and the placeholder pattern.
enum Tok {
    Lit(String),
    /// Digit placeholders, '.', ',', 'E+'/'E-' as written.
    Num(char),
    Percent,
    Text,
    Date(String),
    AmPm(bool),
    Elapsed(char, usize),
}

fn tokenize(sec: &str) -> Vec<Tok> {
    let chars: Vec<char> = sec.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let lit = |out: &mut Vec<Tok>, s: &str| {
        if let Some(Tok::Lit(prev)) = out.last_mut() {
            prev.push_str(s);
        } else {
            out.push(Tok::Lit(s.to_string()));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let mut s = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    s.push(chars[i]);
                    i += 1;
                }
                lit(&mut out, &s);
            }
            '\\' => {
                if let Some(&n) = chars.get(i + 1) {
                    lit(&mut out, &n.to_string());
                }
                i += 1;
            }
            '_' => {
                // Space as wide as the next character.
                lit(&mut out, " ");
                i += 1;
            }
            '*' => {
                // Repeat-to-fill: ignored.
                i += 1;
            }
            '[' => {
                let end = chars[i..].iter().position(|&x| x == ']').map(|p| i + p).unwrap_or(chars.len() - 1);
                let inner: String = chars[i + 1..end].iter().collect();
                if let Some(cur) = inner.strip_prefix('$') {
                    // [$€-813] -> €
                    let sym = cur.split('-').next().unwrap_or("");
                    lit(&mut out, sym);
                } else {
                    let low = inner.to_ascii_lowercase();
                    if !low.is_empty() && low.chars().all(|x| x == 'h' || x == 'm' || x == 's') {
                        out.push(Tok::Elapsed(low.chars().next().unwrap_or('h'), low.len()));
                    }
                    // Colors and conditions are ignored.
                }
                i = end;
            }
            '0' | '#' | '?' | '.' | ',' => out.push(Tok::Num(c)),
            'E' | 'e' if matches!(chars.get(i + 1), Some('+') | Some('-')) => {
                out.push(Tok::Num('E'));
                out.push(Tok::Num(chars[i + 1]));
                i += 1;
            }
            '%' => out.push(Tok::Percent),
            '@' => out.push(Tok::Text),
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                let lc = c.to_ascii_lowercase();
                let mut s = String::new();
                while i < chars.len() && chars[i].to_ascii_lowercase() == lc {
                    s.push(lc);
                    i += 1;
                }
                out.push(Tok::Date(s));
                continue;
            }
            'A' | 'a' => {
                let rest: String = chars[i..].iter().take(5).collect::<String>().to_ascii_uppercase();
                if rest.starts_with("AM/PM") {
                    out.push(Tok::AmPm(true));
                    i += 5;
                    continue;
                } else if rest.starts_with("A/P") {
                    out.push(Tok::AmPm(false));
                    i += 3;
                    continue;
                }
                lit(&mut out, &c.to_string());
            }
            'G' | 'g' if chars[i..].iter().take(7).collect::<String>().eq_ignore_ascii_case("general") => {
                out.push(Tok::Num('G'));
                i += 7;
                continue;
            }
            _ => lit(&mut out, &c.to_string()),
        }
        i += 1;
    }
    out
}

fn render_literal(sec: &str, text: Option<&str>) -> String {
    let mut s = String::new();
    for t in tokenize(sec) {
        match t {
            Tok::Lit(l) => s.push_str(&l),
            Tok::Text => s.push_str(text.unwrap_or("")),
            _ => {}
        }
    }
    s
}

fn is_date_section(toks: &[Tok]) -> bool {
    toks.iter().any(|t| matches!(t, Tok::Date(_) | Tok::Elapsed(..) | Tok::AmPm(_)))
}

fn format_num(n: f64, code: &str, is_date: bool) -> String {
    if !n.is_finite() {
        return format_number(n);
    }
    let sections = split_sections(code);
    let (sec, val, neg_sign) = match sections.len() {
        1 => (sections[0], n, n < 0.0),
        2 => {
            if n < 0.0 {
                (sections[1], -n, false)
            } else {
                (sections[0], n, false)
            }
        }
        _ => {
            if n > 0.0 {
                (sections[0], n, false)
            } else if n < 0.0 {
                (sections[1], -n, false)
            } else {
                (sections[2], n, false)
            }
        }
    };
    let toks = tokenize(sec);
    if is_date_section(&toks) {
        return format_date(n, &toks);
    }
    let has_num = toks.iter().any(|t| matches!(t, Tok::Num(c) if *c != '.' && *c != ','));
    if !has_num {
        let lit = render_literal(sec, None);
        if toks.iter().any(|t| matches!(t, Tok::Num('G'))) || (lit.is_empty() && sections.len() == 1) {
            let shown = if is_date { crate::value::format_date(n) } else { format_number(val) };
            let general = if neg_sign { format!("-{}", shown.trim_start_matches('-')) } else { shown };
            let mut s = String::new();
            let mut done = false;
            for t in &toks {
                match t {
                    Tok::Lit(l) => s.push_str(l),
                    Tok::Num('G') if !done => {
                        s.push_str(&general);
                        done = true;
                    }
                    _ => {}
                }
            }
            if !done {
                s.push_str(&general);
            }
            return s;
        }
        return lit;
    }
    let mut s = format_digits(val.abs(), &toks);
    if neg_sign && s.chars().any(|c| c.is_ascii_digit() && c != '0') {
        s.insert(0, '-');
    }
    s
}

fn format_digits(mut v: f64, toks: &[Tok]) -> String {
    // Layout of the pattern.
    let first = toks.iter().position(|t| matches!(t, Tok::Num(c) if matches!(c, '0' | '#' | '?' | '.')));
    let last = toks.iter().rposition(|t| matches!(t, Tok::Num(_)));
    let (Some(first), Some(last)) = (first, last) else { return format_number(v) };
    let pattern: String = toks[first..=last].iter().filter_map(|t| if let Tok::Num(c) = t { Some(*c) } else { None }).collect();
    let percents = toks.iter().filter(|t| matches!(t, Tok::Percent)).count();
    for _ in 0..percents {
        v *= 100.0;
    }
    let (mantissa, exp_part) = match pattern.find('E') {
        Some(p) => (&pattern[..p], Some(&pattern[p + 1..])),
        None => (pattern.as_str(), None),
    };
    // Trailing commas scale by 1000.
    let mut mant = mantissa.to_string();
    while mant.ends_with(',') {
        mant.pop();
        if exp_part.is_none() {
            v /= 1000.0;
        }
    }
    let (int_pat, frac_pat) = match mant.find('.') {
        Some(p) => (&mant[..p], &mant[p + 1..]),
        None => (mant.as_str(), ""),
    };
    let grouping = int_pat.contains(',');
    let int_min = int_pat.chars().filter(|&c| c == '0').count();
    let frac_max = frac_pat.chars().filter(|&c| matches!(c, '0' | '#' | '?')).count();
    let frac_min = frac_pat.chars().filter(|&c| c == '0').count();

    let mut exp_str = String::new();
    if let Some(ep) = exp_part {
        let exp = if v == 0.0 { 0 } else { v.abs().log10().floor() as i32 - (int_min.max(1) as i32 - 1) };
        v /= 10f64.powi(exp);
        let sign = if exp < 0 {
            "-"
        } else if ep.starts_with('+') {
            "+"
        } else {
            ""
        };
        let digits = ep.chars().filter(|c| matches!(c, '0' | '#')).count().max(1);
        exp_str = format!("E{}{:0width$}", sign, exp.abs(), width = digits);
    }

    let rounded = format!("{:.*}", frac_max, v);
    let (ip, fp) = match rounded.split_once('.') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (rounded.clone(), String::new()),
    };
    let mut ip = ip.trim_start_matches('0').to_string();
    while ip.len() < int_min {
        ip.insert(0, '0');
    }
    if grouping && ip.len() > 3 {
        let mut g = String::new();
        for (i, ch) in ip.chars().enumerate() {
            if i > 0 && (ip.len() - i) % 3 == 0 {
                g.push(',');
            }
            g.push(ch);
        }
        ip = g;
    }
    let mut fp = fp;
    while fp.len() > frac_min && fp.ends_with('0') {
        fp.pop();
    }
    let mut number = ip;
    if frac_max > 0 && (!fp.is_empty() || frac_pat.contains('?') || frac_min > 0 || mant.contains('.')) {
        if !fp.is_empty() || mant.contains('.') {
            number.push('.');
        }
        number.push_str(&fp);
    }
    if mant.contains('.') && frac_max == 0 {
        number.push('.');
    }
    number.push_str(&exp_str);

    let mut s = String::new();
    let mut placed = false;
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Lit(l) => s.push_str(l),
            Tok::Percent => s.push('%'),
            Tok::Num(_) if i >= first && i <= last && !placed => {
                s.push_str(&number);
                placed = true;
            }
            _ => {}
        }
    }
    s
}

fn format_date(serial: f64, toks: &[Tok]) -> String {
    let has_ampm = toks.iter().any(|t| matches!(t, Tok::AmPm(_)));
    // Seconds precision from a ".0" right after seconds.
    let total_secs = (serial * 86400.0).round() as i64;
    let day = total_secs.div_euclid(86400) as f64;
    let secs = total_secs.rem_euclid(86400);
    let (y, mo, d) = serial_to_ymd(day);
    let (h, mi, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    let weekday = ((day as i64 + 6).rem_euclid(7)) as usize; // serial 1 = Sunday 1900-01-01
    let mut out = String::new();
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Lit(l) => out.push_str(l),
            Tok::Num(c) => {
                if *c == '.' {
                    out.push('.');
                } else if *c == '0' {
                    out.push('0');
                } else if *c == ',' {
                    out.push(',');
                }
            }
            Tok::Percent => out.push('%'),
            Tok::Text => {}
            Tok::AmPm(full) => {
                let pm = h >= 12;
                out.push_str(match (full, pm) {
                    (true, false) => "AM",
                    (true, true) => "PM",
                    (false, false) => "A",
                    (false, true) => "P",
                });
            }
            Tok::Elapsed(unit, width) => {
                let v = match unit {
                    'h' => total_secs / 3600,
                    'm' => total_secs / 60,
                    _ => total_secs,
                };
                out.push_str(&format!("{:0width$}", v, width = *width));
            }
            Tok::Date(p) => {
                let minutes = p.starts_with('m') && p.len() <= 2 && is_minute(toks, i);
                match p.as_str() {
                    "yy" | "y" => out.push_str(&format!("{:02}", y.rem_euclid(100))),
                    p if p.starts_with('y') => out.push_str(&format!("{:04}", y)),
                    "m" if minutes => out.push_str(&mi.to_string()),
                    "mm" if minutes => out.push_str(&format!("{:02}", mi)),
                    "m" => out.push_str(&mo.to_string()),
                    "mm" => out.push_str(&format!("{:02}", mo)),
                    "mmm" => out.push_str(&MONTHS[(mo as usize + 11) % 12][..3]),
                    "mmmmm" => out.push_str(&MONTHS[(mo as usize + 11) % 12][..1]),
                    p if p.starts_with('m') => out.push_str(MONTHS[(mo as usize + 11) % 12]),
                    "d" => out.push_str(&d.to_string()),
                    "dd" => out.push_str(&format!("{:02}", d)),
                    "ddd" => out.push_str(&DAYS[weekday][..3]),
                    p if p.starts_with('d') => out.push_str(DAYS[weekday]),
                    p if p.starts_with('h') => {
                        let hh = if has_ampm {
                            let x = h % 12;
                            if x == 0 {
                                12
                            } else {
                                x
                            }
                        } else {
                            h
                        };
                        if p.len() >= 2 {
                            out.push_str(&format!("{:02}", hh));
                        } else {
                            out.push_str(&hh.to_string());
                        }
                    }
                    "s" => out.push_str(&s.to_string()),
                    p if p.starts_with('s') => out.push_str(&format!("{:02}", s)),
                    _ => {}
                }
            }
        }
    }
    out
}

/// `m`/`mm` means minutes right after an hour or right before seconds.
fn is_minute(toks: &[Tok], i: usize) -> bool {
    let prev = toks[..i].iter().rev().find(|t| !matches!(t, Tok::Lit(_)));
    let next = toks[i + 1..].iter().find(|t| !matches!(t, Tok::Lit(_)));
    matches!(prev, Some(Tok::Date(p)) if p.starts_with('h'))
        || matches!(prev, Some(Tok::Elapsed('h', _)))
        || matches!(next, Some(Tok::Date(p)) if p.starts_with('s'))
}

/// Whether a format code displays dates or times.
pub fn is_date_format(code: &str) -> bool {
    let sec = split_sections(code)[0];
    is_date_section(&tokenize(sec))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: f64, code: &str) -> String {
        format_value(&Value::Number(v), code)
    }

    #[test]
    fn numbers() {
        assert_eq!(n(1234.567, "0.00"), "1234.57");
        assert_eq!(n(1234.567, "#,##0.00"), "1,234.57");
        assert_eq!(n(1234567.0, "#,##0"), "1,234,567");
        assert_eq!(n(-1234.5, "#,##0.00"), "-1,234.50");
        assert_eq!(n(-1234.5, "#,##0.00;[Red]-#,##0.00"), "-1,234.50");
        assert_eq!(n(-5.0, "0;(0)"), "(5)");
        assert_eq!(n(0.0, "0;-0;\"zero\""), "zero");
        assert_eq!(n(0.256, "0%"), "26%");
        assert_eq!(n(0.256, "0.0%"), "25.6%");
        assert_eq!(n(12.5, "[$€-813] #,##0.00"), "€ 12.50");
        assert_eq!(n(12.5, "#,##0.00 \"EUR\""), "12.50 EUR");
        assert_eq!(n(7.0, "000"), "007");
        assert_eq!(n(0.5, "#.##"), ".5");
        assert_eq!(n(12345.0, "0.00E+00"), "1.23E+04");
        assert_eq!(n(1500000.0, "#,##0,,\"M\""), "2M");
        assert_eq!(n(3.0, "General"), "3");
        assert_eq!(n(-3.5, "General"), "-3.5");
        assert_eq!(n(42.0, "_-* #,##0_-;-* #,##0_-;_-* \"-\"_-;_-@_-"), " 42 ");
        assert_eq!(n(0.0, "_-* #,##0_-;-* #,##0_-;_-* \"-\"_-;_-@_-"), " - ");
    }

    #[test]
    fn dates() {
        let d = crate::value::ymd_to_serial(2026, 3, 7) + 0.5 + 15.0 / 1440.0;
        let f = |code| format_value(&Value::Date(d), code);
        assert_eq!(f("dd/mm/yyyy"), "07/03/2026");
        assert_eq!(f("d-mmm-yy"), "7-Mar-26");
        assert_eq!(f("dddd d mmmm yyyy"), "Saturday 7 March 2026");
        assert_eq!(f("hh:mm"), "12:15");
        assert_eq!(f("h:mm AM/PM"), "12:15 PM");
        assert_eq!(f("yyyy-mm-dd hh:mm:ss"), "2026-03-07 12:15:00");
        assert_eq!(format_value(&Value::Number(1.5), "[h]:mm"), "36:00");
        assert!(is_date_format("dd/mm/yyyy"));
        assert!(!is_date_format("#,##0.00"));
    }

    #[test]
    fn text() {
        assert_eq!(format_value(&Value::text("abc"), "@"), "abc");
        assert_eq!(format_value(&Value::text("abc"), "0;0;0;\"[\"@\"]\""), "[abc]");
        assert_eq!(format_value(&Value::text("abc"), "0.00"), "abc");
    }
}
