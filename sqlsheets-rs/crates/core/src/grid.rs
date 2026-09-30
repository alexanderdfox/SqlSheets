//! Spreadsheet grid model over a SQLite table snapshot.

use crate::db::Database;
use crate::formula::{recalculate, CellValue};
use rusqlite::types::Value as SqlValue;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellRef {
    pub row: usize, // 0-based data row
    pub col: usize, // 0-based column
}

/// Per-cell formatting (persisted in `_sqlsheets_fmt`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellFmt {
    /// Hex `#RRGGBB` or empty
    pub text_color: String,
    /// Hex `#RRGGBB` or empty
    pub bg_color: String,
    /// Relative font size: 0 = normal, positive = larger steps
    pub font_size: i8,
    pub bold: bool,
}

impl CellFmt {
    pub fn is_empty(&self) -> bool {
        self.text_color.is_empty()
            && self.bg_color.is_empty()
            && self.font_size == 0
            && !self.bold
    }

    /// Parse `#RRGGBB` or `RRGGBB` into (r,g,b) 0–255.
    pub fn parse_rgb(hex: &str) -> Option<(u8, u8, u8)> {
        let h = hex.trim().trim_start_matches('#');
        if h.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&h[0..2], 16).ok()?;
        let g = u8::from_str_radix(&h[2..4], 16).ok()?;
        let b = u8::from_str_radix(&h[4..6], 16).ok()?;
        Some((r, g, b))
    }
}

/// Convert 0-based column index to Excel letter(s): 0→A, 25→Z, 26→AA
pub fn col_letter(idx: usize) -> String {
    let mut n = idx;
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    s
}

/// Parse Excel column letters to 0-based index. A→0, Z→25, AA→26
pub fn col_index(letters: &str) -> Option<usize> {
    let letters = letters.to_uppercase();
    if letters.is_empty() || !letters.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut n: usize = 0;
    for c in letters.chars() {
        n = n * 26 + (c as u8 - b'A') as usize + 1;
    }
    Some(n - 1)
}

#[derive(Debug, Clone)]
pub struct GridModel {
    pub table_name: String,
    pub col_names: Vec<String>,
    pub rowids: Vec<i64>,
    /// Raw cell text as stored in SQLite (formulas kept as-is).
    pub raw: Vec<Vec<String>>,
    /// Evaluated display values after recalculate.
    pub eval_cache: HashMap<(usize, usize), CellValue>,
    /// Cell formatting keyed by (row_index, col_index).
    pub fmt: HashMap<(usize, usize), CellFmt>,
    /// Total rows in the SQLite table (may exceed loaded `raw.len()` when virtualized).
    pub total_rows: usize,
    /// Offset of first loaded row in the full table (for windowed loading).
    pub row_offset: usize,
}

impl GridModel {
    pub fn empty() -> Self {
        Self {
            table_name: String::new(),
            col_names: vec![],
            rowids: vec![],
            raw: vec![],
            eval_cache: HashMap::new(),
            fmt: HashMap::new(),
            total_rows: 0,
            row_offset: 0,
        }
    }

    pub fn from_data(col_names: Vec<String>, rowids: Vec<i64>, rows: Vec<Vec<String>>) -> Self {
        let n = rows.len();
        Self {
            table_name: String::new(),
            col_names,
            rowids,
            raw: rows,
            eval_cache: HashMap::new(),
            fmt: HashMap::new(),
            total_rows: n,
            row_offset: 0,
        }
    }

    pub fn from_db(db: &Database, table: &str, limit: Option<usize>) -> crate::Result<Self> {
        Self::from_db_window(db, table, 0, limit)
    }

    /// Load a window of rows starting at `offset` (for virtualized / large tables).
    pub fn from_db_window(
        db: &Database,
        table: &str,
        offset: usize,
        limit: Option<usize>,
    ) -> crate::Result<Self> {
        let _ = db.ensure_fmt_table();
        let total = db.table_row_count(table).unwrap_or(0) as usize;
        let (rowids, cols, sql_rows) = db.fetch_table_window(table, offset, limit)?;
        let raw: Vec<Vec<String>> = sql_rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|v| match v {
                        SqlValue::Null => String::new(),
                        SqlValue::Integer(i) => i.to_string(),
                        SqlValue::Real(f) => {
                            if f.fract() == 0.0 && f.abs() < 1e15 {
                                format!("{:.0}", f)
                            } else {
                                format!("{}", f)
                            }
                        }
                        SqlValue::Text(s) => s,
                        SqlValue::Blob(_) => String::new(),
                    })
                    .collect()
            })
            .collect();
        let mut g = Self {
            table_name: table.to_string(),
            col_names: cols,
            rowids,
            raw,
            eval_cache: HashMap::new(),
            fmt: HashMap::new(),
            total_rows: total,
            row_offset: offset,
        };
        g.load_fmt(db)?;
        recalculate(&mut g, Some(db));
        Ok(g)
    }

    pub fn load_fmt(&mut self, db: &Database) -> crate::Result<()> {
        self.fmt.clear();
        let map = db.load_fmt_for_table(&self.table_name)?;
        // map is keyed by (sqlite_rowid, col_name)
        let col_index: HashMap<&str, usize> = self
            .col_names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        let rowid_index: HashMap<i64, usize> = self
            .rowids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect();
        for ((rowid, col), fmt) in map {
            if let (Some(&ri), Some(&ci)) = (rowid_index.get(&rowid), col_index.get(col.as_str())) {
                self.fmt.insert((ri, ci), fmt);
            }
        }
        Ok(())
    }

    pub fn row_count(&self) -> usize {
        self.raw.len()
    }

    pub fn col_count(&self) -> usize {
        self.col_names.len()
    }

    pub fn raw_at(&self, row: usize, col: usize) -> Option<String> {
        self.raw.get(row)?.get(col).cloned()
    }

    pub fn display_at(&self, row: usize, col: usize) -> String {
        if let Some(v) = self.eval_cache.get(&(row, col)) {
            return v.display();
        }
        self.raw_at(row, col).unwrap_or_default()
    }

    pub fn is_formula(&self, row: usize, col: usize) -> bool {
        self.raw_at(row, col)
            .map(|s| {
                let t = s.trim();
                t.starts_with('=')
                    || t.starts_with("sql=")
                    || t.starts_with("rust=")
                    || t.starts_with("script=")
                    || t.starts_with("python=")
            })
            .unwrap_or(false)
    }

    pub fn formula_kind(&self, row: usize, col: usize) -> Option<&'static str> {
        let t = self.raw_at(row, col)?;
        let t = t.trim();
        if t.starts_with("rust=") || t.starts_with("script=") {
            Some("rust")
        } else if t.starts_with("python=") {
            Some("python")
        } else if t.starts_with("sql=") {
            Some("sql")
        } else if t.starts_with('=') {
            Some("fx")
        } else {
            None
        }
    }

    pub fn set_raw(&mut self, row: usize, col: usize, value: String) {
        if let Some(r) = self.raw.get_mut(row) {
            if let Some(c) = r.get_mut(col) {
                *c = value;
            }
        }
    }

    pub fn rowid_at(&self, row: usize) -> Option<i64> {
        self.rowids.get(row).copied()
    }

    pub fn col_name(&self, col: usize) -> Option<&str> {
        self.col_names.get(col).map(|s| s.as_str())
    }

    pub fn fmt_at(&self, row: usize, col: usize) -> CellFmt {
        self.fmt.get(&(row, col)).cloned().unwrap_or_default()
    }

    pub fn set_fmt(&mut self, row: usize, col: usize, fmt: CellFmt) {
        if fmt.is_empty() {
            self.fmt.remove(&(row, col));
        } else {
            self.fmt.insert((row, col), fmt);
        }
    }


    /// Absolute 0-based row index for a local window row.
    pub fn abs_row(&self, local_row: usize) -> usize {
        self.row_offset + local_row
    }

    /// Map absolute 0-based row → local index if inside the loaded window.
    pub fn local_row(&self, abs_row: usize) -> Option<usize> {
        if abs_row >= self.row_offset && abs_row < self.row_offset + self.raw.len() {
            Some(abs_row - self.row_offset)
        } else {
            None
        }
    }

    /// Raw text at absolute 0-based row (from window or live DB fetch).
    pub fn raw_at_abs(&self, abs_row: usize, col: usize, db: Option<&Database>) -> Option<String> {
        if let Some(local) = self.local_row(abs_row) {
            return self.raw_at(local, col);
        }
        let db = db?;
        let col_name = self.col_name(col)?;
        db.fetch_cell_raw(&self.table_name, abs_row, col_name).ok()
    }

    /// Suggested window size for virtualization (keeps memory bounded at 100k+ rows).
    pub const WINDOW_SIZE: usize = 512;

    /// Compute a window offset that keeps `abs_row` near the middle of the window.
    pub fn window_offset_for(abs_row: usize, total: usize) -> usize {
        let w = Self::WINDOW_SIZE;
        if total <= w {
            return 0;
        }
        let half = w / 3; // bias toward having more rows ahead while scrolling down
        abs_row.saturating_sub(half).min(total.saturating_sub(w))
    }

    /// True when the absolute row is near the edge of the loaded window (should reload).
    pub fn needs_window_shift(&self, abs_row: usize) -> bool {
        if self.total_rows <= self.raw.len() {
            return false;
        }
        let local = match self.local_row(abs_row) {
            Some(l) => l,
            None => return true,
        };
        let margin = 32.min(self.raw.len() / 4);
        local < margin || local + margin >= self.raw.len()
    }

    /// Persist formatting for one cell to the database.
    pub fn save_fmt_cell(&self, db: &Database, row: usize, col: usize) -> crate::Result<()> {
        let rowid = match self.rowid_at(row) {
            Some(id) => id,
            None => return Ok(()),
        };
        let col_name = match self.col_name(col) {
            Some(n) => n,
            None => return Ok(()),
        };
        let fmt = self.fmt_at(row, col);
        db.set_cell_fmt(&self.table_name, rowid, col_name, &fmt)
    }

    pub fn a1(&self, row: usize, col: usize) -> String {
        format!("{}{}", col_letter(col), self.row_offset + row + 1)
    }
}
