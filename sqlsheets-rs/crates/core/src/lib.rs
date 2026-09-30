//! SqlSheets-rs core: real SQLite + spreadsheet grid + formulas + sql= / rust= cells.
//!
//! Port of https://alexanderdfox.github.io/SqlSheets/ (BSD-3-Clause).

pub mod formula;
pub mod grid;
pub mod db;
pub mod script;

pub use db::{Database, TableInfo, SqlResult};
pub use formula::{eval_cell, recalculate, FormulaError, CellValue};
pub use grid::{GridModel, CellRef, CellFmt, col_letter, col_index};
pub use script::{scripts_enabled, set_scripts_enabled, ScriptKind};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("csv: {0}")]
    Csv(#[from] csv::Error),
    #[error("{0}")]
    Msg(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Meta table used for cell formatting (mirrors original _sqlsheets_fmt).
pub const FMT_TABLE: &str = "_sqlsheets_fmt";
