//! Formula lexer and parser (Excel A1 syntax).

use std::sync::Arc;

use crate::sheet::{col_index, col_name};
use crate::value::CellError;
use crate::{MAX_COLS, MAX_ROWS};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RefPart {
    Cell { row: u32, col: u32, abs_row: bool, abs_col: bool },
    Col { col: u32, abs: bool },
    Row { row: u32, abs: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RefTok {
    pub sheet: Option<Arc<str>>,
    pub a: RefPart,
    pub b: Option<RefPart>,
}

impl RefTok {
    /// Inclusive bounds (r0, c0, r1, c1).
    pub fn bounds(&self) -> (u32, u32, u32, u32) {
        let lo_hi = |p: &RefPart| match *p {
            RefPart::Cell { row, col, .. } => (row, col, row, col),
            RefPart::Col { col, .. } => (0, col, MAX_ROWS as u32 - 1, col),
            RefPart::Row { row, .. } => (row, 0, row, MAX_COLS as u32 - 1),
        };
        let (r0, c0, r1, c1) = lo_hi(&self.a);
        match &self.b {
            None => (r0, c0, r1, c1),
            Some(b) => {
                let (r2, c2, r3, c3) = lo_hi(b);
                (r0.min(r2), c0.min(c2), r1.max(r3), c1.max(c3))
            }
        }
    }

    pub fn is_single_cell(&self) -> bool {
        self.b.is_none() && matches!(self.a, RefPart::Cell { .. })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64),
    Str(String),
    Bool(bool),
    Err(CellError),
    Ref(RefTok),
    Func(String),
    Name(String),
    Op(&'static str),
    LParen,
    RParen,
    Sep,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    Str(Arc<str>),
    Bool(bool),
    Err(CellError),
    Ref(RefTok),
    Name(String),
    Neg(Box<Expr>),
    Percent(Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Missing,
}

pub type ParseError = String;

/// `src[a..b]`, or "" when that is not a valid range. Positions come from the lexer, so they are
/// character boundaries; this keeps a lexer bug from becoming a crash.
fn sub(src: &str, a: usize, b: usize) -> &str {
    src.get(a..b).unwrap_or_default()
}

fn parse_cell_ref(s: &str) -> Option<RefPart> {
    let b = s.as_bytes();
    let mut i = 0;
    let abs_col = b.first() == Some(&b'$');
    if abs_col {
        i += 1;
    }
    let cs = i;
    while i < b.len() && b[i].is_ascii_alphabetic() {
        i += 1;
    }
    let letters = s.get(cs..i)?;
    let abs_row = b.get(i) == Some(&b'$');
    if abs_row {
        i += 1;
    }
    let rs = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i != b.len() || rs == i || letters.is_empty() {
        return None;
    }
    let col = col_index(letters)?;
    let row: usize = s.get(rs..)?.parse().ok()?;
    if row == 0 || row > MAX_ROWS {
        return None;
    }
    Some(RefPart::Cell { row: row as u32 - 1, col: col as u32, abs_row, abs_col })
}

fn parse_col_ref(s: &str) -> Option<RefPart> {
    let (abs, letters) = match s.strip_prefix('$') {
        Some(r) => (true, r),
        None => (false, s),
    };
    if !letters.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    Some(RefPart::Col { col: col_index(letters)? as u32, abs })
}

fn parse_row_ref(s: &str) -> Option<RefPart> {
    let (abs, digits) = match s.strip_prefix('$') {
        Some(r) => (true, r),
        None => (false, s),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row: usize = digits.parse().ok()?;
    if row == 0 || row > MAX_ROWS {
        return None;
    }
    Some(RefPart::Row { row: row as u32 - 1, abs })
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '$' || c == '\\'
}

/// Split a formula into tokens. `src` is without the leading '='.
pub fn tokenize(src: &str) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let n = chars.len();
    let pos = |i: usize| if i < n { chars[i].0 } else { src.len() };
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let (start, c) = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // Strings
        if c == '"' {
            let mut s = String::new();
            i += 1;
            loop {
                if i >= n {
                    return Err("unterminated string".into());
                }
                let ch = chars[i].1;
                if ch == '"' {
                    if i + 1 < n && chars[i + 1].1 == '"' {
                        s.push('"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                s.push(ch);
                i += 1;
            }
            out.push(Token { tok: Tok::Str(s), start, end: pos(i) });
            continue;
        }
        // Error literals
        if c == '#' {
            let rest = src.get(start..).unwrap_or_default();
            let mut found = None;
            for e in ["#DIV/0!", "#N/A", "#NAME?", "#NULL!", "#NUM!", "#REF!", "#VALUE!", "#CYCLE!"] {
                if rest.get(..e.len()).is_some_and(|p| p.eq_ignore_ascii_case(e)) {
                    found = Some(e);
                    break;
                }
            }
            let e = found.ok_or("unknown error literal")?;
            let end = start + e.len();
            while i < n && chars[i].0 < end {
                i += 1;
            }
            out.push(Token { tok: Tok::Err(CellError::parse(e).ok_or("unknown error literal")?), start, end });
            continue;
        }
        // Quoted sheet name: 'My Sheet'!A1
        if c == '\'' {
            let mut name = String::new();
            i += 1;
            loop {
                if i >= n {
                    return Err("unterminated sheet name".into());
                }
                let ch = chars[i].1;
                if ch == '\'' {
                    if i + 1 < n && chars[i + 1].1 == '\'' {
                        name.push('\'');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                name.push(ch);
                i += 1;
            }
            if i >= n || chars[i].1 != '!' {
                return Err("expected ! after sheet name".into());
            }
            i += 1;
            let (r, ni) = lex_ref_after_sheet(src, &chars, i, Some(Arc::from(name.as_str())))?;
            i = ni;
            out.push(Token { tok: Tok::Ref(r), start, end: pos(i) });
            continue;
        }
        // Numbers (and row ranges like 1:3)
        if c.is_ascii_digit() || (c == '.' && i + 1 < n && chars[i + 1].1.is_ascii_digit()) {
            let mut j = i;
            while j < n && (chars[j].1.is_ascii_digit() || chars[j].1 == '.') {
                j += 1;
            }
            // Row range?
            if j < n && chars[j].1 == ':' {
                let a = sub(src, start, pos(j));
                let mut k = j + 1;
                while k < n && (chars[k].1.is_ascii_digit() || chars[k].1 == '$') {
                    k += 1;
                }
                if let (Some(pa), Some(pb)) = (parse_row_ref(a), parse_row_ref(sub(src, pos(j + 1), pos(k)))) {
                    out.push(Token { tok: Tok::Ref(RefTok { sheet: None, a: pa, b: Some(pb) }), start, end: pos(k) });
                    i = k;
                    continue;
                }
            }
            if j < n && (chars[j].1 == 'e' || chars[j].1 == 'E') {
                let mut k = j + 1;
                if k < n && (chars[k].1 == '+' || chars[k].1 == '-') {
                    k += 1;
                }
                if k < n && chars[k].1.is_ascii_digit() {
                    while k < n && chars[k].1.is_ascii_digit() {
                        k += 1;
                    }
                    j = k;
                }
            }
            let text = sub(src, start, pos(j));
            let v: f64 = text.parse().map_err(|_| format!("bad number {text}"))?;
            out.push(Token { tok: Tok::Num(v), start, end: pos(j) });
            i = j;
            continue;
        }
        if c == '$' || is_word_char(c) {
            let mut j = i;
            while j < n && is_word_char(chars[j].1) {
                j += 1;
            }
            let word = sub(src, start, pos(j));
            // Sheet prefix
            if j < n && chars[j].1 == '!' {
                let (r, ni) = lex_ref_after_sheet(src, &chars, j + 1, Some(Arc::from(word)))?;
                out.push(Token { tok: Tok::Ref(r), start, end: pos(ni) });
                i = ni;
                continue;
            }
            // Function call
            let mut k = j;
            while k < n && chars[k].1 == ' ' {
                k += 1;
            }
            if k < n && chars[k].1 == '(' && !word.contains('$') {
                let mut name = word.to_ascii_uppercase();
                for p in ["_XLFN._XLWS.", "_XLFN.", "_XLWS."] {
                    if let Some(rest) = name.strip_prefix(p) {
                        name = rest.to_string();
                    }
                }
                out.push(Token { tok: Tok::Func(name), start, end: pos(j) });
                i = j;
                continue;
            }
            // Reference or range
            let (r, ni) = lex_ref_after_sheet(src, &chars, i, None)
                .unwrap_or((RefTok { sheet: None, a: RefPart::Row { row: 0, abs: false }, b: None }, usize::MAX));
            if ni != usize::MAX && ni >= j {
                out.push(Token { tok: Tok::Ref(r), start, end: pos(ni) });
                i = ni;
                continue;
            }
            let up = word.to_ascii_uppercase();
            let tok = match up.as_str() {
                "TRUE" => Tok::Bool(true),
                "FALSE" => Tok::Bool(false),
                _ => Tok::Name(word.to_string()),
            };
            out.push(Token { tok, start, end: pos(j) });
            i = j;
            continue;
        }
        let two = if i + 1 < n { Some((c, chars[i + 1].1)) } else { None };
        let op2 = match two {
            Some(('<', '>')) => Some("<>"),
            Some(('<', '=')) => Some("<="),
            Some(('>', '=')) => Some(">="),
            _ => None,
        };
        if let Some(op) = op2 {
            out.push(Token { tok: Tok::Op(op), start, end: pos(i + 2) });
            i += 2;
            continue;
        }
        let tok = match c {
            '+' => Tok::Op("+"),
            '-' => Tok::Op("-"),
            '*' => Tok::Op("*"),
            '/' => Tok::Op("/"),
            '^' => Tok::Op("^"),
            '&' => Tok::Op("&"),
            '=' => Tok::Op("="),
            '<' => Tok::Op("<"),
            '>' => Tok::Op(">"),
            '%' => Tok::Op("%"),
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            ',' | ';' => Tok::Sep,
            _ => return Err(format!("unexpected character '{c}'")),
        };
        out.push(Token { tok, start, end: pos(i + 1) });
        i += 1;
    }
    Ok(out)
}

/// Lex `A1`, `A1:B2`, `A:B` or `1:2` starting at char index `i`.
fn lex_ref_after_sheet(
    src: &str,
    chars: &[(usize, char)],
    i: usize,
    sheet: Option<Arc<str>>,
) -> Result<(RefTok, usize), ParseError> {
    let n = chars.len();
    let pos = |i: usize| if i < n { chars[i].0 } else { src.len() };
    let read_part = |mut j: usize| {
        let s = j;
        while j < n && (chars[j].1.is_ascii_alphanumeric() || chars[j].1 == '$') {
            j += 1;
        }
        (s, j)
    };
    let (s, j) = read_part(i);
    let first = sub(src, pos(s), pos(j));
    let a_cell = parse_cell_ref(first);
    let a_col = parse_col_ref(first);
    let a_row = parse_row_ref(first);
    if j < n && chars[j].1 == ':' {
        let (s2, j2) = read_part(j + 1);
        let second = sub(src, pos(s2), pos(j2));
        let pair = match (a_cell, a_col, a_row) {
            (Some(a), _, _) => parse_cell_ref(second).map(|b| (a, b)),
            (_, Some(a), _) => parse_col_ref(second).map(|b| (a, b)),
            (_, _, Some(a)) => parse_row_ref(second).map(|b| (a, b)),
            _ => None,
        };
        if let Some((a, b)) = pair {
            return Ok((RefTok { sheet, a, b: Some(b) }, j2));
        }
    }
    match a_cell {
        Some(a) => Ok((RefTok { sheet, a, b: None }, j)),
        None => Err(format!("bad reference {first}")),
    }
}

struct Parser {
    toks: Vec<Token>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|t| &t.tok)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).map(|t| t.tok.clone());
        self.i += 1;
        t
    }
    fn peek_op(&self) -> Option<&'static str> {
        match self.peek() {
            Some(Tok::Op(o)) => Some(o),
            _ => None,
        }
    }

    fn expr(&mut self) -> Result<Expr, ParseError> {
        self.comparison()
    }

    fn comparison(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.concat()?;
        while let Some(op @ ("=" | "<>" | "<" | ">" | "<=" | ">=")) = self.peek_op() {
            self.i += 1;
            let rhs = self.concat()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn concat(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.additive()?;
        while let Some("&") = self.peek_op() {
            self.i += 1;
            let rhs = self.additive()?;
            lhs = Expr::Bin("&", Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn additive(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.multiplicative()?;
        while let Some(op @ ("+" | "-")) = self.peek_op() {
            self.i += 1;
            let rhs = self.multiplicative()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.power()?;
        while let Some(op @ ("*" | "/")) = self.peek_op() {
            self.i += 1;
            let rhs = self.power()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn power(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.unary()?;
        while let Some("^") = self.peek_op() {
            self.i += 1;
            let rhs = self.unary()?;
            lhs = Expr::Bin("^", Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek_op() {
            Some("-") => {
                self.i += 1;
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Some("+") => {
                self.i += 1;
                self.unary()
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.primary()?;
        while let Some("%") = self.peek_op() {
            self.i += 1;
            e = Expr::Percent(Box::new(e));
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::Str(s)) => Ok(Expr::Str(Arc::from(s.as_str()))),
            Some(Tok::Bool(b)) => Ok(Expr::Bool(b)),
            Some(Tok::Err(e)) => Ok(Expr::Err(e)),
            Some(Tok::Ref(r)) => Ok(Expr::Ref(r)),
            Some(Tok::Name(n)) => Ok(Expr::Name(n)),
            Some(Tok::LParen) => {
                let e = self.expr()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err("expected )".into()),
                }
            }
            Some(Tok::Func(name)) => {
                match self.next() {
                    Some(Tok::LParen) => {}
                    _ => return Err("expected (".into()),
                }
                let mut args = Vec::new();
                if let Some(Tok::RParen) = self.peek() {
                    self.i += 1;
                    return Ok(Expr::Call(name, args));
                }
                loop {
                    match self.peek() {
                        Some(Tok::Sep) | Some(Tok::RParen) => args.push(Expr::Missing),
                        _ => args.push(self.expr()?),
                    }
                    match self.next() {
                        Some(Tok::Sep) => continue,
                        Some(Tok::RParen) => break,
                        _ => return Err("expected , or )".into()),
                    }
                }
                Ok(Expr::Call(name, args))
            }
            Some(t) => Err(format!("unexpected {:?}", t)),
            None => Err("unexpected end of formula".into()),
        }
    }
}

/// Parse a formula (without leading '=').
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return Err("empty formula".into());
    }
    let mut p = Parser { toks, i: 0 };
    let e = p.expr()?;
    if p.i < p.toks.len() {
        return Err("unexpected trailing input".into());
    }
    Ok(e)
}

/// Collect every reference in an expression.
pub fn collect_refs<'a>(e: &'a Expr, out: &mut Vec<&'a RefTok>) {
    match e {
        Expr::Ref(r) => out.push(r),
        Expr::Neg(a) | Expr::Percent(a) => collect_refs(a, out),
        Expr::Bin(_, a, b) => {
            collect_refs(a, out);
            collect_refs(b, out);
        }
        Expr::Call(_, args) => args.iter().for_each(|a| collect_refs(a, out)),
        _ => {}
    }
}

/// Collect every defined name an expression uses.
pub fn collect_names<'a>(e: &'a Expr, out: &mut Vec<&'a str>) {
    match e {
        Expr::Name(n) => out.push(n),
        Expr::Neg(a) | Expr::Percent(a) => collect_names(a, out),
        Expr::Bin(_, a, b) => {
            collect_names(a, out);
            collect_names(b, out);
        }
        Expr::Call(_, args) => args.iter().for_each(|a| collect_names(a, out)),
        _ => {}
    }
}

pub fn has_volatile(e: &Expr) -> bool {
    match e {
        Expr::Call(name, args) => {
            matches!(name.as_str(), "NOW" | "TODAY" | "RAND" | "RANDBETWEEN") || args.iter().any(has_volatile)
        }
        Expr::Neg(a) | Expr::Percent(a) => has_volatile(a),
        Expr::Bin(_, a, b) => has_volatile(a) || has_volatile(b),
        _ => false,
    }
}

fn part_text(p: &RefPart) -> String {
    match *p {
        RefPart::Cell { row, col, abs_row, abs_col } => {
            format!("{}{}{}{}", if abs_col { "$" } else { "" }, col_name(col as usize), if abs_row { "$" } else { "" }, row + 1)
        }
        RefPart::Col { col, abs } => format!("{}{}", if abs { "$" } else { "" }, col_name(col as usize)),
        RefPart::Row { row, abs } => format!("{}{}", if abs { "$" } else { "" }, row + 1),
    }
}

fn shift_part(p: &RefPart, dr: i64, dc: i64) -> Option<RefPart> {
    let sh = |v: u32, d: i64, max: usize| {
        let n = v as i64 + d;
        if n < 0 || n >= max as i64 {
            None
        } else {
            Some(n as u32)
        }
    };
    Some(match *p {
        RefPart::Cell { row, col, abs_row, abs_col } => RefPart::Cell {
            row: if abs_row { row } else { sh(row, dr, MAX_ROWS)? },
            col: if abs_col { col } else { sh(col, dc, MAX_COLS)? },
            abs_row,
            abs_col,
        },
        RefPart::Col { col, abs } => RefPart::Col { col: if abs { col } else { sh(col, dc, MAX_COLS)? }, abs },
        RefPart::Row { row, abs } => RefPart::Row { row: if abs { row } else { sh(row, dr, MAX_ROWS)? }, abs },
    })
}

fn sheet_prefix(s: &str) -> String {
    let plain =
        s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') && !s.chars().next().is_some_and(|c| c.is_ascii_digit());
    if plain {
        format!("{s}!")
    } else {
        format!("'{}'!", s.replace('\'', "''"))
    }
}

/// Move relative references by (dr, dc), as Excel does when copying a formula.
/// References pushed off the sheet become #REF!.
pub fn shift_formula(src: &str, dr: i64, dc: i64) -> String {
    let Ok(toks) = tokenize(src) else { return src.to_string() };
    let mut out = String::with_capacity(src.len() + 8);
    let mut last = 0;
    for t in &toks {
        if let Tok::Ref(r) = &t.tok {
            out.push_str(sub(src, last, t.start));
            let a = shift_part(&r.a, dr, dc);
            let b = r.b.as_ref().map(|b| shift_part(b, dr, dc));
            match (a, b) {
                (Some(a), None) => {
                    if let Some(s) = &r.sheet {
                        out.push_str(&sheet_prefix(s));
                    }
                    out.push_str(&part_text(&a));
                }
                (Some(a), Some(Some(b))) => {
                    if let Some(s) = &r.sheet {
                        out.push_str(&sheet_prefix(s));
                    }
                    out.push_str(&part_text(&a));
                    out.push(':');
                    out.push_str(&part_text(&b));
                }
                _ => out.push_str("#REF!"),
            }
            last = t.end;
        }
    }
    out.push_str(src.get(last..).unwrap_or_default());
    out
}

/// Point references to sheet `from` at sheet `to` instead (sheet names compare
/// case-insensitively, like Excel). Returns None when nothing refers to `from`.
pub fn rename_sheet_in_formula(src: &str, from: &str, to: &str) -> Option<String> {
    if !src.contains('!') {
        return None;
    }
    let toks = tokenize(src).ok()?;
    let mut out = String::with_capacity(src.len() + to.len());
    let mut last = 0;
    for t in &toks {
        let Tok::Ref(r) = &t.tok else { continue };
        if !r.sheet.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(from)) {
            continue;
        }
        let text = sub(src, t.start, t.end);
        let Some((_, after)) = text.rsplit_once('!') else { continue };
        out.push_str(sub(src, last, t.start));
        out.push_str(&sheet_prefix(to));
        out.push_str(after);
        last = t.end;
    }
    if last == 0 {
        return None;
    }
    out.push_str(src.get(last..).unwrap_or_default());
    Some(out)
}

/// Does the formula refer to a sheet with one of these names?
pub fn mentions_sheet(src: &str, names: &[&str]) -> bool {
    if !src.contains('!') {
        return false;
    }
    let Ok(toks) = tokenize(src) else { return false };
    toks.iter().any(|t| match &t.tok {
        Tok::Ref(RefTok { sheet: Some(s), .. }) => names.iter().any(|n| s.eq_ignore_ascii_case(n)),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic() {
        let e = parse("SUM(A1:B2, 3) * -2 ^ 2").unwrap();
        assert!(matches!(e, Expr::Bin("*", _, _)));
        assert!(parse("'My Sheet'!A1 + Sheet2!$B$3").is_ok());
        assert!(parse("SUM(A:A)").is_ok());
        assert!(parse("SUM(1:3)").is_ok());
        assert!(parse("IF(A1>=10;\"big\";\"small\")").is_ok());
        assert!(parse("_xlfn.XLOOKUP(1,A:A,B:B)").is_ok());
        assert!(parse("SUM(A1").is_err());
        // Found by tests/fuzz.rs: a multi-byte character after '#' used to panic.
        assert!(parse("#\u{11918}").is_err());
        assert!(parse("#NAMé").is_err());
    }

    #[test]
    fn shifts_relative_refs() {
        assert_eq!(shift_formula("A1+$B$2+C$3+$D4", 1, 1), "B2+$B$2+D$3+$D5");
        assert_eq!(shift_formula("SUM(A1:A10)", 2, 0), "SUM(A3:A12)");
        assert_eq!(shift_formula("A1", -1, 0), "#REF!");
        assert_eq!(shift_formula("'My Sheet'!A1&\"A1\"", 0, 1), "'My Sheet'!B1&\"A1\"");
    }

    #[test]
    fn renames_sheet_refs() {
        assert_eq!(rename_sheet_in_formula("Sheet1!A1*2", "sheet1", "Data").as_deref(), Some("Data!A1*2"));
        assert_eq!(
            rename_sheet_in_formula("SUM('Sheet1'!$A$1:B2)+Other!A1+\"Sheet1!A1\"", "Sheet1", "My Data").as_deref(),
            Some("SUM('My Data'!$A$1:B2)+Other!A1+\"Sheet1!A1\"")
        );
        assert_eq!(rename_sheet_in_formula("A1+Other!B2", "Sheet1", "Data"), None);
        assert_eq!(rename_sheet_in_formula("Sheet1!A1", "Sheet1", "it's").as_deref(), Some("'it''s'!A1"));
        assert!(mentions_sheet("'data'!A1", &["Data"]));
        assert!(!mentions_sheet("A1+\"Data!A1\"", &["Data"]));
    }
}
