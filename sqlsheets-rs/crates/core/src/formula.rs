//! Minimal Excel-style formula engine + sql= cell expressions.
//!
//! Supports: numbers, cell refs (A1), ranges (A1:B3), + - * / ( ),
//! SUM, AVERAGE, MIN, MAX, COUNT, IF, AND, OR, NOT, ABS, ROUND,
//! and string concat with &.
//!
//! Cells starting with `sql=` run a SQL expression against the DB
//! (scalar result). `rust=` runs Rhai (Rust-like); `python=` runs
//! system Python 3 via a sandboxed helper with `cell("A1")`.

use crate::db::{sql_value_to_f64, value_to_display, Database};
use crate::grid::{col_index, GridModel};
use rusqlite::types::Value as SqlValue;
use std::collections::HashSet;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum FormulaError {
    #[error("#REF!")]
    Ref,
    #[error("#VALUE!")]
    Value,
    #[error("#DIV/0!")]
    Div0,
    #[error("#NAME?")]
    Name,
    #[error("#CIRC!")]
    Circ,
    #[error("#SQL! {0}")]
    Sql(String),
    #[error("{0}")]
    Msg(String),
}

#[derive(Debug, Clone)]
pub enum CellValue {
    Empty,
    Number(f64),
    Text(String),
    Error(FormulaError),
}

impl CellValue {
    pub fn display(&self) -> String {
        match self {
            CellValue::Empty => String::new(),
            CellValue::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{:.0}", n)
                } else {
                    format!("{}", n)
                }
            }
            CellValue::Text(s) => s.clone(),
            CellValue::Error(e) => e.to_string(),
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            CellValue::Number(n) => Some(*n),
            CellValue::Text(s) => s.trim().parse().ok(),
            CellValue::Empty => Some(0.0),
            CellValue::Error(_) => None,
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            CellValue::Empty => false,
            CellValue::Number(n) => *n != 0.0,
            CellValue::Text(s) => !s.is_empty(),
            CellValue::Error(_) => false,
        }
    }
}

/// Evaluate a single raw cell string in the context of the grid and optional DB.
pub fn eval_cell(
    raw: &str,
    grid: &GridModel,
    row: usize,
    col: usize,
    db: Option<&Database>,
    visiting: &mut HashSet<(usize, usize)>,
) -> CellValue {
    let raw = raw.trim();
    if raw.is_empty() {
        return CellValue::Empty;
    }

    // sql= expression → run as scalar SQL
    if let Some(rest) = raw.strip_prefix("sql=") {
        return eval_sql(rest.trim(), db);
    }
    // rust= → Rhai (Rust-like); python= → system Python 3
    if let Some(rest) = raw.strip_prefix("rust=").or_else(|| raw.strip_prefix("script=")) {
        return crate::script::eval_script_cell(
            crate::script::ScriptKind::Rhai,
            rest,
            grid,
            row,
            col,
            db,
            visiting,
        );
    }
    if let Some(rest) = raw.strip_prefix("python=") {
        return crate::script::eval_script_cell(
            crate::script::ScriptKind::Python,
            rest,
            grid,
            row,
            col,
            db,
            visiting,
        );
    }

    if !raw.starts_with('=') {
        // Plain value: try number, else text
        if let Ok(n) = raw.parse::<f64>() {
            return CellValue::Number(n);
        }
        return CellValue::Text(raw.to_string());
    }

    let expr = &raw[1..];
    if visiting.contains(&(row, col)) {
        return CellValue::Error(FormulaError::Circ);
    }
    visiting.insert((row, col));
    let result = eval_expr(expr, grid, db, visiting);
    visiting.remove(&(row, col));
    result
}

fn eval_sql(sql: &str, db: Option<&Database>) -> CellValue {
    let Some(db) = db else {
        return CellValue::Error(FormulaError::Sql("no database".into()));
    };
    match db.connection().query_row(sql, [], |row| {
        let v: SqlValue = row.get(0)?;
        Ok(v)
    }) {
        Ok(v) => match sql_value_to_f64(&v) {
            Some(n) => CellValue::Number(n),
            None => CellValue::Text(value_to_display(&v)),
        },
        Err(e) => CellValue::Error(FormulaError::Sql(e.to_string())),
    }
}

// ── Tokenizer / recursive-descent parser ───────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Cell(usize, usize), // (col, row) 0-based
    Range(usize, usize, usize, usize), // c1,r1,c2,r2
    Op(char),
    Comma,
    LParen,
    RParen,
    Amp, // &
}

fn tokenize(input: &str) -> Result<Vec<Tok>, FormulaError> {
    let mut toks = Vec::new();
    let mut i = 0;
    let bytes = input.as_bytes();
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();

    while i < n {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == ',' {
            toks.push(Tok::Comma);
            i += 1;
            continue;
        }
        if c == '(' {
            toks.push(Tok::LParen);
            i += 1;
            continue;
        }
        if c == ')' {
            toks.push(Tok::RParen);
            i += 1;
            continue;
        }
        if c == '&' {
            toks.push(Tok::Amp);
            i += 1;
            continue;
        }
        if matches!(c, '+' | '-' | '*' | '/' | '=' | '<' | '>' | '^') {
            // two-char ops
            if i + 1 < n {
                let two: String = chars[i..i + 2].iter().collect();
                if matches!(two.as_str(), "<=" | ">=" | "<>") {
                    // treat comparison as Ident-like for IF; we keep simple ops only
                    // for v1 stick to arithmetic; comparisons handled in IF args as numbers
                }
            }
            toks.push(Tok::Op(c));
            i += 1;
            continue;
        }
        if c == '"' {
            i += 1;
            let mut s = String::new();
            while i < n && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < n {
                    i += 1;
                }
                s.push(chars[i]);
                i += 1;
            }
            if i < n {
                i += 1; // closing "
            }
            toks.push(Tok::Str(s));
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && i + 1 < n && chars[i + 1].is_ascii_digit()) {
            let start = i;
            while i < n && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let num_str: String = chars[start..i].iter().collect();
            let num: f64 = num_str.parse().map_err(|_| FormulaError::Value)?;
            toks.push(Tok::Num(num));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            // Maybe cell ref: letters then digits, optional : range
            let ident: String = chars[start..i].iter().collect();
            // Check if this is A1-style
            if let Some((col, row)) = parse_a1(&ident) {
                // Look ahead for :B2
                if i < n && chars[i] == ':' {
                    i += 1;
                    let rstart = i;
                    while i < n && (chars[i].is_ascii_alphanumeric()) {
                        i += 1;
                    }
                    let ident2: String = chars[rstart..i].iter().collect();
                    if let Some((c2, r2)) = parse_a1(&ident2) {
                        toks.push(Tok::Range(col, row, c2, r2));
                        continue;
                    }
                    return Err(FormulaError::Ref);
                }
                toks.push(Tok::Cell(col, row));
                continue;
            }
            toks.push(Tok::Ident(ident.to_uppercase()));
            continue;
        }
        return Err(FormulaError::Value);
    }
    let _ = bytes; // silence
    Ok(toks)
}

fn parse_a1(s: &str) -> Option<(usize, usize)> {
    let s = s.to_uppercase();
    let mut col_part = String::new();
    let mut row_part = String::new();
    for c in s.chars() {
        if c.is_ascii_alphabetic() {
            if !row_part.is_empty() {
                return None;
            }
            col_part.push(c);
        } else if c.is_ascii_digit() {
            row_part.push(c);
        } else {
            return None;
        }
    }
    if col_part.is_empty() || row_part.is_empty() {
        return None;
    }
    let col = col_index(&col_part)?;
    let row: usize = row_part.parse().ok()?;
    if row == 0 {
        return None;
    }
    Some((col, row - 1)) // 0-based row
}

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    grid: &'a GridModel,
    db: Option<&'a Database>,
    visiting: &'a mut HashSet<(usize, usize)>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        if self.pos < self.toks.len() {
            let t = self.toks[self.pos].clone();
            self.pos += 1;
            Some(t)
        } else {
            None
        }
    }
    fn parse(&mut self) -> Result<CellValue, FormulaError> {
        self.parse_concat()
    }

    fn parse_concat(&mut self) -> Result<CellValue, FormulaError> {
        let mut left = self.parse_add()?;
        while matches!(self.peek(), Some(Tok::Amp)) {
            self.next();
            let right = self.parse_add()?;
            left = CellValue::Text(format!("{}{}", left.display(), right.display()));
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<CellValue, FormulaError> {
        let mut left = self.parse_mul()?;
        loop {
            match self.peek() {
                Some(Tok::Op('+')) => {
                    self.next();
                    let r = self.parse_mul()?;
                    left = bin_num(left, r, |a, b| a + b)?;
                }
                Some(Tok::Op('-')) => {
                    self.next();
                    let r = self.parse_mul()?;
                    left = bin_num(left, r, |a, b| a - b)?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<CellValue, FormulaError> {
        let mut left = self.parse_unary()?;
        loop {
            match self.peek() {
                Some(Tok::Op('*')) => {
                    self.next();
                    let r = self.parse_unary()?;
                    left = bin_num(left, r, |a, b| a * b)?;
                }
                Some(Tok::Op('/')) => {
                    self.next();
                    let r = self.parse_unary()?;
                    let bn = r.as_number().ok_or(FormulaError::Value)?;
                    if bn == 0.0 {
                        return Err(FormulaError::Div0);
                    }
                    left = bin_num(left, r, |a, b| a / b)?;
                }
                Some(Tok::Op('^')) => {
                    self.next();
                    let r = self.parse_unary()?;
                    left = bin_num(left, r, |a, b| a.powf(b))?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<CellValue, FormulaError> {
        if matches!(self.peek(), Some(Tok::Op('-'))) {
            self.next();
            let v = self.parse_unary()?;
            let n = v.as_number().ok_or(FormulaError::Value)?;
            return Ok(CellValue::Number(-n));
        }
        if matches!(self.peek(), Some(Tok::Op('+'))) {
            self.next();
            return self.parse_unary();
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<CellValue, FormulaError> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(CellValue::Number(n)),
            Some(Tok::Str(s)) => Ok(CellValue::Text(s)),
            Some(Tok::Cell(c, r)) => self.resolve_cell(c, r),
            Some(Tok::Range(c1, r1, c2, r2)) => {
                // Ranges only make sense as function args; bare range → #VALUE!
                let _ = (c1, r1, c2, r2);
                Err(FormulaError::Value)
            }
            Some(Tok::Ident(name)) => self.parse_func(name),
            Some(Tok::LParen) => {
                let v = self.parse()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(v),
                    _ => Err(FormulaError::Value),
                }
            }
            _ => Err(FormulaError::Value),
        }
    }

    fn resolve_cell(&mut self, col: usize, row: usize) -> Result<CellValue, FormulaError> {
        // `row` from A1 is absolute 0-based. Map into the loaded window when possible
        // so recursion uses consistent local indices; otherwise fetch from DB.
        let (local_row, raw) = if let Some(local) = self.grid.local_row(row) {
            (
                local,
                self.grid.raw_at(local, col).unwrap_or_default(),
            )
        } else {
            // Outside window: fetch raw from SQLite and evaluate without local cache index
            let raw = self
                .grid
                .raw_at_abs(row, col, self.db)
                .unwrap_or_default();
            // Use a synthetic local index that will not collide with the window
            // (visiting is keyed by the absolute row via a large offset tag).
            // We pass `row` as the "local" for visiting uniqueness outside window.
            (row.saturating_add(1_000_000_000), raw)
        };
        Ok(eval_cell(
            &raw,
            self.grid,
            local_row,
            col,
            self.db,
            self.visiting,
        ))
    }

    fn parse_func(&mut self, name: String) -> Result<CellValue, FormulaError> {
        if !matches!(self.next(), Some(Tok::LParen)) {
            return Err(FormulaError::Name);
        }
        let mut args: Vec<Arg> = Vec::new();
        if !matches!(self.peek(), Some(Tok::RParen)) {
            loop {
                args.push(self.parse_arg()?);
                match self.peek() {
                    Some(Tok::Comma) => {
                        self.next();
                    }
                    Some(Tok::RParen) => break,
                    _ => break,
                }
            }
        }
        if !matches!(self.next(), Some(Tok::RParen)) {
            return Err(FormulaError::Value);
        }
        call_func(&name, &args, self.grid, self.db, self.visiting)
    }

    fn parse_arg(&mut self) -> Result<Arg, FormulaError> {
        // Peek for range token
        if let Some(Tok::Range(c1, r1, c2, r2)) = self.peek().cloned() {
            self.next();
            return Ok(Arg::Range(c1, r1, c2, r2));
        }
        // Single cell becomes a 1-cell range conceptually, but evaluate as value
        if let Some(Tok::Cell(c, r)) = self.peek().cloned() {
            self.next();
            let v = self.resolve_cell(c, r)?;
            return Ok(Arg::Value(v));
        }
        let v = self.parse_concat()?;
        Ok(Arg::Value(v))
    }
}

#[derive(Debug, Clone)]
enum Arg {
    Value(CellValue),
    Range(usize, usize, usize, usize),
}

fn bin_num(
    a: CellValue,
    b: CellValue,
    f: impl Fn(f64, f64) -> f64,
) -> Result<CellValue, FormulaError> {
    let na = a.as_number().ok_or(FormulaError::Value)?;
    let nb = b.as_number().ok_or(FormulaError::Value)?;
    Ok(CellValue::Number(f(na, nb)))
}

fn collect_range_nums(
    c1: usize,
    r1: usize,
    c2: usize,
    r2: usize,
    grid: &GridModel,
    db: Option<&Database>,
    visiting: &mut HashSet<(usize, usize)>,
) -> Result<Vec<f64>, FormulaError> {
    // r1/r2 are absolute 0-based from A1 parsing
    let (min_c, max_c) = if c1 <= c2 { (c1, c2) } else { (c2, c1) };
    let (min_r, max_r) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
    // Cap range size to avoid scanning 100k cells in one formula
    const MAX_RANGE_CELLS: usize = 50_000;
    let height = max_r.saturating_sub(min_r).saturating_add(1);
    let width = max_c.saturating_sub(min_c).saturating_add(1);
    if height.saturating_mul(width) > MAX_RANGE_CELLS {
        return Err(FormulaError::Msg("#RANGE! too large".into()));
    }
    let mut nums = Vec::new();
    for abs_r in min_r..=max_r {
        for c in min_c..=max_c {
            let (local, raw) = if let Some(local) = grid.local_row(abs_r) {
                (local, grid.raw_at(local, c).unwrap_or_default())
            } else {
                (
                    abs_r.saturating_add(1_000_000_000),
                    grid.raw_at_abs(abs_r, c, db).unwrap_or_default(),
                )
            };
            let v = eval_cell(&raw, grid, local, c, db, visiting);
            if let Some(n) = v.as_number() {
                nums.push(n);
            }
        }
    }
    Ok(nums)
}

fn arg_nums(
    arg: &Arg,
    grid: &GridModel,
    db: Option<&Database>,
    visiting: &mut HashSet<(usize, usize)>,
) -> Result<Vec<f64>, FormulaError> {
    match arg {
        Arg::Value(v) => Ok(v.as_number().into_iter().collect()),
        Arg::Range(c1, r1, c2, r2) => collect_range_nums(*c1, *r1, *c2, *r2, grid, db, visiting),
    }
}

fn call_func(
    name: &str,
    args: &[Arg],
    grid: &GridModel,
    db: Option<&Database>,
    visiting: &mut HashSet<(usize, usize)>,
) -> Result<CellValue, FormulaError> {
    match name {
        "SUM" => {
            let mut total = 0.0;
            for a in args {
                for n in arg_nums(a, grid, db, visiting)? {
                    total += n;
                }
            }
            Ok(CellValue::Number(total))
        }
        "AVERAGE" | "AVG" => {
            let mut nums = Vec::new();
            for a in args {
                nums.extend(arg_nums(a, grid, db, visiting)?);
            }
            if nums.is_empty() {
                return Err(FormulaError::Div0);
            }
            Ok(CellValue::Number(nums.iter().sum::<f64>() / nums.len() as f64))
        }
        "MIN" => {
            let mut nums = Vec::new();
            for a in args {
                nums.extend(arg_nums(a, grid, db, visiting)?);
            }
            nums
                .into_iter()
                .min_by(|a, b| a.partial_cmp(b).unwrap())
                .map(CellValue::Number)
                .ok_or(FormulaError::Value)
        }
        "MAX" => {
            let mut nums = Vec::new();
            for a in args {
                nums.extend(arg_nums(a, grid, db, visiting)?);
            }
            nums
                .into_iter()
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .map(CellValue::Number)
                .ok_or(FormulaError::Value)
        }
        "COUNT" => {
            let mut n = 0usize;
            for a in args {
                n += arg_nums(a, grid, db, visiting)?.len();
            }
            Ok(CellValue::Number(n as f64))
        }
        "SQRT" => {
            let v = match args.first() {
                Some(Arg::Value(v)) => v.as_number().ok_or(FormulaError::Value)?,
                _ => return Err(FormulaError::Value),
            };
            if v < 0.0 {
                return Err(FormulaError::Value);
            }
            Ok(CellValue::Number(v.sqrt()))
        }
        "POWER" | "POW" => {
            if args.len() < 2 {
                return Err(FormulaError::Value);
            }
            let a = match &args[0] {
                Arg::Value(v) => v.as_number().ok_or(FormulaError::Value)?,
                _ => return Err(FormulaError::Value),
            };
            let b = match &args[1] {
                Arg::Value(v) => v.as_number().ok_or(FormulaError::Value)?,
                _ => return Err(FormulaError::Value),
            };
            Ok(CellValue::Number(a.powf(b)))
        }
        "MOD" => {
            if args.len() < 2 {
                return Err(FormulaError::Value);
            }
            let a = match &args[0] {
                Arg::Value(v) => v.as_number().ok_or(FormulaError::Value)?,
                _ => return Err(FormulaError::Value),
            };
            let b = match &args[1] {
                Arg::Value(v) => v.as_number().ok_or(FormulaError::Value)?,
                _ => return Err(FormulaError::Value),
            };
            if b == 0.0 {
                return Err(FormulaError::Div0);
            }
            Ok(CellValue::Number(a % b))
        }
        "COUNTA" => {
            let mut n = 0usize;
            for a in args {
                match a {
                    Arg::Value(v) => {
                        if !matches!(v, CellValue::Empty) {
                            n += 1;
                        }
                    }
                    Arg::Range(c1, r1, c2, r2) => {
                        let (min_c, max_c) = if *c1 <= *c2 { (*c1, *c2) } else { (*c2, *c1) };
                        let (min_r, max_r) = if *r1 <= *r2 { (*r1, *r2) } else { (*r2, *r1) };
                        for abs_r in min_r..=max_r {
                            for c in min_c..=max_c {
                                let raw = if let Some(local) = grid.local_row(abs_r) {
                                    grid.raw_at(local, c).unwrap_or_default()
                                } else {
                                    grid.raw_at_abs(abs_r, c, db).unwrap_or_default()
                                };
                                if !raw.trim().is_empty() {
                                    n += 1;
                                }
                            }
                        }
                    }
                }
            }
            Ok(CellValue::Number(n as f64))
        }
        "ABS" => {
            let v = match args.first() {
                Some(Arg::Value(v)) => v.as_number().ok_or(FormulaError::Value)?,
                _ => return Err(FormulaError::Value),
            };
            Ok(CellValue::Number(v.abs()))
        }
        "ROUND" => {
            let v = match args.first() {
                Some(Arg::Value(v)) => v.as_number().ok_or(FormulaError::Value)?,
                _ => return Err(FormulaError::Value),
            };
            let digits = match args.get(1) {
                Some(Arg::Value(v)) => v.as_number().unwrap_or(0.0) as i32,
                _ => 0,
            };
            let factor = 10f64.powi(digits);
            Ok(CellValue::Number((v * factor).round() / factor))
        }
        "IF" => {
            if args.len() < 2 {
                return Err(FormulaError::Value);
            }
            let cond = match &args[0] {
                Arg::Value(v) => v.is_truthy(),
                _ => return Err(FormulaError::Value),
            };
            let idx = if cond { 1 } else { 2 };
            match args.get(idx) {
                Some(Arg::Value(v)) => Ok(v.clone()),
                Some(Arg::Range(..)) => Err(FormulaError::Value),
                None => Ok(CellValue::Empty),
            }
        }
        "AND" => {
            for a in args {
                match a {
                    Arg::Value(v) if !v.is_truthy() => return Ok(CellValue::Number(0.0)),
                    Arg::Range(..) => return Err(FormulaError::Value),
                    _ => {}
                }
            }
            Ok(CellValue::Number(1.0))
        }
        "OR" => {
            for a in args {
                match a {
                    Arg::Value(v) if v.is_truthy() => return Ok(CellValue::Number(1.0)),
                    Arg::Range(..) => return Err(FormulaError::Value),
                    _ => {}
                }
            }
            Ok(CellValue::Number(0.0))
        }
        "NOT" => {
            let v = match args.first() {
                Some(Arg::Value(v)) => v,
                _ => return Err(FormulaError::Value),
            };
            Ok(CellValue::Number(if v.is_truthy() { 0.0 } else { 1.0 }))
        }
        "LEN" => {
            let s = match args.first() {
                Some(Arg::Value(v)) => v.display(),
                _ => return Err(FormulaError::Value),
            };
            Ok(CellValue::Number(s.chars().count() as f64))
        }
        "UPPER" => {
            let s = match args.first() {
                Some(Arg::Value(v)) => v.display(),
                _ => return Err(FormulaError::Value),
            };
            Ok(CellValue::Text(s.to_uppercase()))
        }
        "LOWER" => {
            let s = match args.first() {
                Some(Arg::Value(v)) => v.display(),
                _ => return Err(FormulaError::Value),
            };
            Ok(CellValue::Text(s.to_lowercase()))
        }
        "CONCAT" | "CONCATENATE" => {
            let mut s = String::new();
            for a in args {
                if let Arg::Value(v) = a {
                    s.push_str(&v.display());
                }
            }
            Ok(CellValue::Text(s))
        }
        _ => Err(FormulaError::Name),
    }
}

fn eval_expr(
    expr: &str,
    grid: &GridModel,
    db: Option<&Database>,
    visiting: &mut HashSet<(usize, usize)>,
) -> CellValue {
    match tokenize(expr) {
        Ok(toks) => {
            let mut p = Parser {
                toks,
                pos: 0,
                grid,
                db,
                visiting,
            };
            match p.parse() {
                Ok(v) => {
                    if p.pos != p.toks.len() {
                        // trailing junk
                        CellValue::Error(FormulaError::Value)
                    } else {
                        v
                    }
                }
                Err(e) => CellValue::Error(e),
            }
        }
        Err(e) => CellValue::Error(e),
    }
}

/// Recalculate all evaluated values for a grid model.
pub fn recalculate(grid: &mut GridModel, db: Option<&Database>) {
    grid.eval_cache.clear();
    let rows = grid.row_count();
    let cols = grid.col_count();
    for r in 0..rows {
        for c in 0..cols {
            let mut visiting = HashSet::new();
            let raw = grid.raw_at(r, c).unwrap_or_default();
            let val = eval_cell(&raw, grid, r, c, db, &mut visiting);
            grid.eval_cache.insert((r, c), val);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::GridModel;

    fn make_grid(cols: &[&str], data: &[Vec<&str>]) -> GridModel {
        let col_names: Vec<String> = cols.iter().map(|s| s.to_string()).collect();
        let rowids: Vec<i64> = (1..=data.len() as i64).collect();
        let rows: Vec<Vec<String>> = data
            .iter()
            .map(|r| r.iter().map(|s| s.to_string()).collect())
            .collect();
        GridModel::from_data(col_names, rowids, rows)
    }

    #[test]
    fn sum_range() {
        let mut g = make_grid(
            &["A", "B"],
            &[
                vec!["1", "2"],
                vec!["3", "4"],
                vec!["=SUM(A1:B2)", ""],
            ],
        );
        recalculate(&mut g, None);
        assert_eq!(g.display_at(2, 0), "10");
    }

    #[test]
    fn cell_ref() {
        let mut g = make_grid(&["A", "B"], &[vec!["5", "=A1*2"]]);
        recalculate(&mut g, None);
        assert_eq!(g.display_at(0, 1), "10");
    }
}
