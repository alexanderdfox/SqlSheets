//! SQLite wrapper with table listing, import/export, and row helpers.

use crate::{Error, Result, FMT_TABLE};
use rusqlite::{Connection, params, types::Value as SqlValue};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub name: String,
    pub row_count: i64,
}

pub struct Database {
    conn: Connection,
    pub file_name: String,
}

impl Database {
    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Ok(Self {
            conn,
            file_name: "sqlsheet.sqlite".into(),
        })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path)?;
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("sqlsheet.sqlite")
            .to_string();
        Ok(Self { conn, file_name })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// List user tables (exclude internal sqlite_ and _sqlsheets_ meta).
    pub fn list_tables(&self) -> Result<Vec<TableInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT name FROM sqlite_master
             WHERE type='table'
               AND name NOT LIKE 'sqlite_%'
               AND name NOT LIKE '_sqlsheets_%'
             ORDER BY name",
        )?;
        let names: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut out = Vec::with_capacity(names.len());
        for name in names {
            let count: i64 = self
                .conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {}", quote_ident(&name)),
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            out.push(TableInfo {
                name,
                row_count: count,
            });
        }
        Ok(out)
    }

    pub fn table_columns(&self, table: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({})", quote_ident(table)))?;
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(cols)
    }

    /// Returns (rowids, column_names, rows as Vec of SqlValue per column).
    pub fn fetch_table(
        &self,
        table: &str,
        limit: Option<usize>,
    ) -> Result<(Vec<i64>, Vec<String>, Vec<Vec<SqlValue>>)> {
        self.fetch_table_window(table, 0, limit)
    }

    pub fn create_table(&self, name: &str, columns: &[(String, String)]) -> Result<()> {
        if columns.is_empty() {
            return Err(Error::Msg("need at least one column".into()));
        }
        let defs: Vec<String> = columns
            .iter()
            .map(|(n, ty)| format!("{} {}", quote_ident(n), ty))
            .collect();
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS {} ({})",
            quote_ident(name),
            defs.join(", ")
        );
        self.conn.execute(&sql, [])?;
        Ok(())
    }

    pub fn drop_table(&self, name: &str) -> Result<()> {
        self.conn
            .execute(&format!("DROP TABLE IF EXISTS {}", quote_ident(name)), [])?;
        Ok(())
    }

    pub fn set_cell(&self, table: &str, rowid: i64, col: &str, value: &str) -> Result<()> {
        // Store as text; formulas are just text that starts with = / rust= / sql=
        let sql = format!(
            "UPDATE {} SET {} = ?1 WHERE rowid = ?2",
            quote_ident(table),
            quote_ident(col)
        );
        self.conn.execute(&sql, params![value, rowid])?;
        Ok(())
    }

    pub fn insert_row(&self, table: &str, cols: &[String], values: &[String]) -> Result<i64> {
        let col_list = cols
            .iter()
            .map(|c| quote_ident(c))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=values.len())
            .map(|i| format!("?{}", i))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_ident(table),
            col_list,
            placeholders
        );
        self.conn.execute(
            &sql,
            rusqlite::params_from_iter(values.iter().map(|s| s.as_str())),
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_row(&self, table: &str, rowid: i64) -> Result<()> {
        self.conn.execute(
            &format!("DELETE FROM {} WHERE rowid = ?1", quote_ident(table)),
            params![rowid],
        )?;
        Ok(())
    }

    pub fn execute_sql(&self, sql: &str) -> Result<SqlResult> {
        let sql = sql.trim();
        if sql.is_empty() {
            return Ok(SqlResult::Message("empty".into()));
        }
        let upper = sql.to_uppercase();
        let is_query = upper.starts_with("SELECT")
            || upper.starts_with("PRAGMA")
            || upper.starts_with("WITH")
            || upper.starts_with("EXPLAIN");

        if is_query {
            let mut stmt = self.conn.prepare(sql)?;
            let col_names: Vec<String> = stmt
                .column_names()
                .iter()
                .map(|s| s.to_string())
                .collect();
            let mut rows = Vec::new();
            let mut rows_iter = stmt.query([])?;
            while let Some(row) = rows_iter.next()? {
                let mut vals = Vec::with_capacity(col_names.len());
                for i in 0..col_names.len() {
                    vals.push(value_to_display(&row.get::<_, SqlValue>(i)?));
                }
                rows.push(vals);
            }
            Ok(SqlResult::Table {
                columns: col_names,
                rows,
            })
        } else {
            let n = self.conn.execute(sql, [])?;
            Ok(SqlResult::Message(format!("{} row(s) affected", n)))
        }
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<()> {
        // Use SQLite backup API for a consistent copy
        let mut dst = Connection::open(path.as_ref())?;
        let backup = rusqlite::backup::Backup::new(&self.conn, &mut dst)?;
        backup.run_to_completion(100, std::time::Duration::from_millis(10), None)?;
        Ok(())
    }

    pub fn import_csv(&self, table: &str, path: impl AsRef<Path>) -> Result<usize> {
        let mut rdr = csv::ReaderBuilder::new()
            .flexible(true)
            .from_path(path)?;
        let headers: Vec<String> = rdr
            .headers()?
            .iter()
            .map(|h| {
                let h = h.trim();
                if h.is_empty() {
                    "col".to_string()
                } else {
                    h.to_string()
                }
            })
            .collect();
        if headers.is_empty() {
            return Err(Error::Msg("CSV has no headers".into()));
        }
        // Sanitize column names
        let cols: Vec<(String, String)> = headers
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let name = sanitize_ident(h).unwrap_or_else(|| format!("col{}", i + 1));
                (name, "TEXT".to_string())
            })
            .collect();
        self.create_table(table, &cols)?;
        let col_names: Vec<String> = cols.iter().map(|(n, _)| n.clone()).collect();
        let mut count = 0usize;
        for rec in rdr.records() {
            let rec = rec?;
            let vals: Vec<String> = (0..col_names.len())
                .map(|i| rec.get(i).unwrap_or("").to_string())
                .collect();
            self.insert_row(table, &col_names, &vals)?;
            count += 1;
        }
        Ok(count)
    }

    pub fn export_csv(&self, table: &str, path: impl AsRef<Path>) -> Result<usize> {
        let (rowids, cols, rows) = self.fetch_table(table, None)?;
        let mut wtr = csv::Writer::from_path(path)?;
        wtr.write_record(&cols)?;
        for row in &rows {
            let rec: Vec<String> = row.iter().map(value_to_display).collect();
            wtr.write_record(&rec)?;
        }
        wtr.flush()?;
        Ok(rowids.len())
    }

    /// Ensure formatting meta table exists.
    pub fn ensure_fmt_table(&self) -> Result<()> {
        self.conn.execute(
            &format!(
                "CREATE TABLE IF NOT EXISTS {} (
                    table_name TEXT NOT NULL,
                    rowid INTEGER NOT NULL,
                    col_name TEXT NOT NULL,
                    text_color TEXT,
                    bg_color TEXT,
                    font_size INTEGER DEFAULT 0,
                    bold INTEGER DEFAULT 0,
                    PRIMARY KEY (table_name, rowid, col_name)
                )",
                quote_ident(FMT_TABLE)
            ),
            [],
        )?;
        // migrate older tables missing columns
        let _ = self.conn.execute(
            &format!("ALTER TABLE {} ADD COLUMN font_size INTEGER DEFAULT 0", quote_ident(FMT_TABLE)),
            [],
        );
        let _ = self.conn.execute(
            &format!("ALTER TABLE {} ADD COLUMN bold INTEGER DEFAULT 0", quote_ident(FMT_TABLE)),
            [],
        );
        Ok(())
    }

    pub fn table_row_count(&self, table: &str) -> Result<i64> {
        self.conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {}", quote_ident(table)),
                [],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    /// Fetch a window of rows (ORDER BY rowid LIMIT/OFFSET).
    pub fn fetch_table_window(
        &self,
        table: &str,
        offset: usize,
        limit: Option<usize>,
    ) -> Result<(Vec<i64>, Vec<String>, Vec<Vec<SqlValue>>)> {
        let cols = self.table_columns(table)?;
        if cols.is_empty() {
            return Ok((vec![], vec![], vec![]));
        }
        let col_list = cols
            .iter()
            .map(|c| quote_ident(c))
            .collect::<Vec<_>>()
            .join(", ");
        let lim = limit
            .map(|n| format!(" LIMIT {} OFFSET {}", n, offset))
            .unwrap_or_else(|| {
                if offset > 0 {
                    format!(" LIMIT -1 OFFSET {}", offset)
                } else {
                    String::new()
                }
            });
        let sql = format!(
            "SELECT rowid, {} FROM {} ORDER BY rowid{}",
            col_list,
            quote_ident(table),
            lim
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let col_count = cols.len();
        let mut rowids = Vec::new();
        let mut rows = Vec::new();
        let mut rows_iter = stmt.query([])?;
        while let Some(row) = rows_iter.next()? {
            let rowid: i64 = row.get(0)?;
            rowids.push(rowid);
            let mut vals = Vec::with_capacity(col_count);
            for i in 0..col_count {
                vals.push(row.get::<_, SqlValue>(i + 1)?);
            }
            rows.push(vals);
        }
        Ok((rowids, cols, rows))
    }

    pub fn load_fmt_for_table(
        &self,
        table: &str,
    ) -> Result<std::collections::HashMap<(i64, String), crate::grid::CellFmt>> {
        self.ensure_fmt_table()?;
        let mut stmt = self.conn.prepare(&format!(
            "SELECT rowid, col_name, text_color, bg_color, font_size, bold FROM {} WHERE table_name = ?1",
            quote_ident(FMT_TABLE)
        ))?;
        let mut map = std::collections::HashMap::new();
        let mut rows = stmt.query(params![table])?;
        while let Some(row) = rows.next()? {
            let rowid: i64 = row.get(0)?;
            let col: String = row.get(1)?;
            let text_color: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
            let bg_color: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
            let font_size: i8 = row.get::<_, Option<i64>>(4)?.unwrap_or(0) as i8;
            let bold: bool = row.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0;
            map.insert(
                (rowid, col),
                crate::grid::CellFmt {
                    text_color,
                    bg_color,
                    font_size,
                    bold,
                },
            );
        }
        Ok(map)
    }

    pub fn set_cell_fmt(
        &self,
        table: &str,
        rowid: i64,
        col: &str,
        fmt: &crate::grid::CellFmt,
    ) -> Result<()> {
        self.ensure_fmt_table()?;
        if fmt.is_empty() {
            self.conn.execute(
                &format!(
                    "DELETE FROM {} WHERE table_name = ?1 AND rowid = ?2 AND col_name = ?3",
                    quote_ident(FMT_TABLE)
                ),
                params![table, rowid, col],
            )?;
        } else {
            self.conn.execute(
                &format!(
                    "INSERT INTO {} (table_name, rowid, col_name, text_color, bg_color, font_size, bold)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(table_name, rowid, col_name) DO UPDATE SET
                       text_color = excluded.text_color,
                       bg_color = excluded.bg_color,
                       font_size = excluded.font_size,
                       bold = excluded.bold",
                    quote_ident(FMT_TABLE)
                ),
                params![
                    table,
                    rowid,
                    col,
                    fmt.text_color,
                    fmt.bg_color,
                    fmt.font_size as i64,
                    if fmt.bold { 1i64 } else { 0i64 },
                ],
            )?;
        }
        Ok(())
    }



    pub fn add_column(&self, table: &str, col_name: &str, col_type: &str) -> Result<()> {
        self.conn.execute(
            &format!(
                "ALTER TABLE {} ADD COLUMN {} {}",
                quote_ident(table),
                quote_ident(col_name),
                col_type
            ),
            [],
        )?;
        Ok(())
    }

    pub fn next_col_letter_name(&self, table: &str) -> Result<String> {
        let cols = self.table_columns(table)?;
        let mut n = cols.len();
        loop {
            let name = crate::grid::col_letter(n);
            if !cols.iter().any(|c| c.eq_ignore_ascii_case(&name)) {
                return Ok(name);
            }
            n += 1;
        }
    }

    /// Replace formula/script/sql source with a plain computed value for every computed cell.
    pub fn bake_table_values(
        &self,
        table: &str,
        grid: &crate::grid::GridModel,
    ) -> Result<usize> {
        let mut n = 0usize;
        self.conn.execute("BEGIN", [])?;
        for r in 0..grid.row_count() {
            for c in 0..grid.col_count() {
                if !grid.is_formula(r, c) {
                    continue;
                }
                let display = grid.display_at(r, c);
                if display.starts_with('#') {
                    continue;
                }
                if let (Some(rowid), Some(col)) = (grid.rowid_at(r), grid.col_name(c)) {
                    self.set_cell(table, rowid, col, &display)?;
                    n += 1;
                }
            }
        }
        self.conn.execute("COMMIT", [])?;
        Ok(n)
    }

    pub fn save_query_as_table(&self, name: &str, columns: &[String], rows: &[Vec<String>]) -> Result<()> {
        if columns.is_empty() {
            return Err(Error::Msg("no columns".into()));
        }
        let defs: Vec<(String, String)> = columns
            .iter()
            .map(|c| (sanitize_ident(c).unwrap_or_else(|| c.clone()), "TEXT".into()))
            .collect();
        let col_names: Vec<String> = defs.iter().map(|(n, _)| n.clone()).collect();
        let _ = self.drop_table(name);
        self.create_table(name, &defs)?;
        for row in rows {
            let mut vals = row.clone();
            while vals.len() < col_names.len() {
                vals.push(String::new());
            }
            vals.truncate(col_names.len());
            self.insert_row(name, &col_names, &vals)?;
        }
        Ok(())
    }

    /// Fetch a single cell by absolute 0-based row order (ORDER BY rowid LIMIT 1 OFFSET n).
    pub fn fetch_cell_raw(&self, table: &str, abs_row: usize, col: &str) -> Result<String> {
        let sql = format!(
            "SELECT {} FROM {} ORDER BY rowid LIMIT 1 OFFSET {}",
            quote_ident(col),
            quote_ident(table),
            abs_row
        );
        let v: rusqlite::types::Value = self.conn.query_row(&sql, [], |r| r.get(0))?;
        Ok(value_to_display(&v))
    }

    /// Seed a simple demo table if database is empty.
    pub fn seed_demo_if_empty(&self) -> Result<()> {
        if !self.list_tables()?.is_empty() {
            return Ok(());
        }
        self.create_table(
            "demo",
            &[
                ("name".into(), "TEXT".into()),
                ("qty".into(), "REAL".into()),
                ("price".into(), "REAL".into()),
                ("total".into(), "TEXT".into()),
                ("note".into(), "TEXT".into()),
            ],
        )?;
        let cols = vec![
            "name".into(),
            "qty".into(),
            "price".into(),
            "total".into(),
            "note".into(),
        ];
        // Grid display rows are 1-based: first inserted row → A1/B1/C1/D1
        self.insert_row(
            "demo",
            &cols,
            &[
                "Widget".into(),
                "10".into(),
                "2.5".into(),
                "=B1*C1".into(),
                "rust=cell(\"B1\") * cell(\"C1\")".into(),
            ],
        )?;
        self.insert_row(
            "demo",
            &cols,
            &[
                "Gadget".into(),
                "3".into(),
                "9.99".into(),
                "=B2*C2".into(),
                "python=cell(\"B2\") * cell(\"C2\")".into(),
            ],
        )?;
        self.insert_row(
            "demo",
            &cols,
            &[
                "Total".into(),
                "".into(),
                "".into(),
                "=SUM(D1:D2)".into(),
                "rust=cell(\"D1\") + cell(\"D2\")".into(),
            ],
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum SqlResult {
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Message(String),
}

pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn sanitize_ident(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if i > 0 {
            out.push('_');
        }
    }
    if out.is_empty() || out.chars().next()?.is_ascii_digit() {
        out = format!("c_{}", out);
    }
    Some(out)
}

pub fn value_to_display(v: &SqlValue) -> String {
    match v {
        SqlValue::Null => String::new(),
        SqlValue::Integer(i) => i.to_string(),
        SqlValue::Real(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{:.0}", f)
            } else {
                format!("{}", f)
            }
        }
        SqlValue::Text(s) => s.clone(),
        SqlValue::Blob(b) => format!("<blob {} bytes>", b.len()),
    }
}

pub fn sql_value_to_f64(v: &SqlValue) -> Option<f64> {
    match v {
        SqlValue::Integer(i) => Some(*i as f64),
        SqlValue::Real(f) => Some(*f),
        SqlValue::Text(s) => s.trim().parse().ok(),
        _ => None,
    }
}
