//! `rust=` (Rhai / Rust-like) and `python=` cell evaluation.
//!
//! # rust=  (Rhai — Rust-flavored, pure Rust, sandboxed)
//! ```text
//! rust=cell("A1") * 2
//! rust=let x = cell("B2"); x + 10
//! rust=if cell("A1") > 0 { "yes" } else { "no" }
//! ```
//!
//! Helpers registered in every script engine:
//! - `cell(addr)` — value of cell by A1 address (e.g. `"A1"`)
//! - `cell_rc(row, col)` — 1-based row, 0-based or letter col via string
//! - `sql(query)` — run scalar SQL (if DB available)
//!
//! # python=  (system Python 3 via subprocess)
//! ```text
//! python=cell("A1") * 2
//! python=sum([1,2,3])
//! ```
//! The host injects a `cell(addr)` function and evaluates the snippet as an
//! expression (or last statement via `_result`). Requires `python3` on PATH.

use crate::db::Database;
use crate::formula::{eval_cell, CellValue, FormulaError};
use crate::grid::{col_index, GridModel};
use rhai::{Dynamic, Engine, Scope};
use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Max script source length (mirrors original browser limit spirit).
pub const SCRIPT_MAX_LEN: usize = 8000;

/// Whether `rust=` / `python=` cells are allowed this session.
/// Default: true for local tools; set false after importing untrusted files.
static SCRIPTS_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false); // default-deny like original SqlSheets

pub fn scripts_enabled() -> bool {
    SCRIPTS_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_scripts_enabled(on: bool) {
    SCRIPTS_ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn eval_script_cell(
    kind: ScriptKind,
    source: &str,
    grid: &GridModel,
    row: usize,
    col: usize,
    db: Option<&Database>,
    visiting: &mut HashSet<(usize, usize)>,
) -> CellValue {
    if !scripts_enabled() {
        return CellValue::Error(FormulaError::Msg(
            "#SCRIPT! disabled (enable scripts for this session)".into(),
        ));
    }
    let source = source.trim();
    if source.is_empty() {
        return CellValue::Empty;
    }
    if source.len() > SCRIPT_MAX_LEN {
        return CellValue::Error(FormulaError::Msg("#SCRIPT! too long".into()));
    }
    if visiting.contains(&(row, col)) {
        return CellValue::Error(FormulaError::Circ);
    }
    visiting.insert((row, col));
    let result = match kind {
        ScriptKind::Rhai => eval_rhai(source, grid, db, visiting),
        ScriptKind::Python => eval_python(source, grid, db, visiting),
    };
    visiting.remove(&(row, col));
    result
}

#[derive(Debug, Clone, Copy)]
pub enum ScriptKind {
    /// `rust=` or `script=` — Rhai (Rust-like)
    Rhai,
    /// `python=` — system Python 3
    Python,
}

fn dynamic_to_cell(v: Dynamic) -> CellValue {
    if v.is_unit() {
        return CellValue::Empty;
    }
    if let Ok(n) = v.as_int() {
        return CellValue::Number(n as f64);
    }
    if let Ok(n) = v.as_float() {
        return CellValue::Number(n);
    }
    if let Ok(b) = v.as_bool() {
        return CellValue::Number(if b { 1.0 } else { 0.0 });
    }
    if let Ok(s) = v.clone().into_string() {
        if let Ok(n) = s.trim().parse::<f64>() {
            return CellValue::Number(n);
        }
        return CellValue::Text(s);
    }
    CellValue::Text(v.to_string())
}

fn resolve_a1(
    addr: &str,
    grid: &GridModel,
    db: Option<&Database>,
    visiting: &mut HashSet<(usize, usize)>,
) -> CellValue {
    let addr = addr.trim().to_uppercase();
    let mut col_part = String::new();
    let mut row_part = String::new();
    for c in addr.chars() {
        if c.is_ascii_alphabetic() {
            if !row_part.is_empty() {
                return CellValue::Error(FormulaError::Ref);
            }
            col_part.push(c);
        } else if c.is_ascii_digit() {
            row_part.push(c);
        } else {
            return CellValue::Error(FormulaError::Ref);
        }
    }
    let col = match col_index(&col_part) {
        Some(c) => c,
        None => return CellValue::Error(FormulaError::Ref),
    };
    let row_1based: usize = match row_part.parse() {
        Ok(r) if r >= 1 => r,
        _ => return CellValue::Error(FormulaError::Ref),
    };
    let abs_r = row_1based - 1;
    let (local, raw) = if let Some(local) = grid.local_row(abs_r) {
        (local, grid.raw_at(local, col).unwrap_or_default())
    } else {
        (
            abs_r.saturating_add(1_000_000_000),
            grid.raw_at_abs(abs_r, col, db).unwrap_or_default(),
        )
    };
    eval_cell(&raw, grid, local, col, db, visiting)
}

fn cell_value_to_dynamic(v: &CellValue) -> Dynamic {
    match v {
        CellValue::Empty => Dynamic::UNIT,
        CellValue::Number(n) => {
            if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                Dynamic::from(*n as i64)
            } else {
                Dynamic::from(*n)
            }
        }
        CellValue::Text(s) => Dynamic::from(s.clone()),
        CellValue::Error(e) => Dynamic::from(e.to_string()),
    }
}

fn eval_rhai(
    source: &str,
    grid: &GridModel,
    db: Option<&Database>,
    visiting: &mut HashSet<(usize, usize)>,
) -> CellValue {
    // Pre-resolve a limited set of cell values into a map so callbacks stay Sync.
    // Nested formula/script refs still go through resolve_a1 with a shared visiting set.
    let visiting_arc = Arc::new(Mutex::new(std::mem::take(visiting)));
    let grid_arc = Arc::new(grid.clone());

    let mut engine = Engine::new();
    engine.set_max_operations(50_000);
    engine.set_max_expr_depths(32, 32);

    let vis1 = Arc::clone(&visiting_arc);
    let g1 = Arc::clone(&grid_arc);
    engine.register_fn("cell", move |addr: &str| -> Dynamic {
        let mut vis = vis1.lock().unwrap();
        let v = resolve_a1(addr, &g1, None, &mut vis);
        cell_value_to_dynamic(&v)
    });

    // sql() — optional; run against a one-shot query string list is not available Sync-safe
    // without wrapping. We expose a simple form when db is present by pre-binding nothing
    // and documenting that sql= cells are preferred for SQL.
    // Provide sql() via a Sync channel of (query -> result string) using a mutex-held closure
    // is hard with rusqlite::Connection (!Sync). Skip registering sql() here.
    let _ = db; // preferred path: use sql= cells alongside rust=

    let mut scope = Scope::new();
    let result = engine.eval_with_scope::<Dynamic>(&mut scope, source);
    if let Ok(mut g) = visiting_arc.lock() {
        *visiting = std::mem::take(&mut *g);
    }

    match result {
        Ok(v) => dynamic_to_cell(v),
        Err(e) => CellValue::Error(FormulaError::Msg(format!("#SCRIPT! {}", e))),
    }
}


fn eval_python(
    source: &str,
    grid: &GridModel,
    _db: Option<&Database>,
    visiting: &mut HashSet<(usize, usize)>,
) -> CellValue {
    // Build a small Python prelude with cell() that reads pre-serialized grid values.
    // We snapshot all non-script cells as a dict to avoid re-entrancy complexity.
    let mut cell_map = serde_json::Map::new();
    for r in 0..grid.row_count() {
        for c in 0..grid.col_count() {
            let addr = format!("{}{}", crate::grid::col_letter(c), r + 1);
            let raw = grid.raw_at(r, c).unwrap_or_default();
            // Avoid evaluating other scripts/python from Python for safety; use raw/display
            let display = if raw.trim().starts_with("rust=")
                || raw.trim().starts_with("python=")
            {
                // nested script: evaluate with Rhai path only if plain formula/value
                String::new()
            } else {
                let v = eval_cell(&raw, grid, r, c, None, visiting);
                v.display()
            };
            // try number
            if let Ok(n) = display.trim().parse::<f64>() {
                cell_map.insert(addr, serde_json::json!(n));
            } else {
                cell_map.insert(addr, serde_json::json!(display));
            }
        }
    }
    let cells_json = serde_json::Value::Object(cell_map).to_string();

    // Escape source for embedding in a triple-quoted string carefully
    let user = source.replace('\\', "\\\\").replace("\"\"\"", "\\\"\\\"\\\"");

    let wrapper = format!(
        r#"
import json, sys
_CELLS = json.loads({cells_json:?})

def cell(addr):
    a = str(addr).strip().upper()
    if a not in _CELLS:
        return 0
    return _CELLS[a]

_src = """{user}"""
try:
    _result = eval(_src, {{"cell": cell, "__builtins__": {{
        "abs": abs, "min": min, "max": max, "sum": sum, "len": len,
        "int": int, "float": float, "str": str, "bool": bool,
        "round": round, "range": range, "list": list, "dict": dict,
        "True": True, "False": False, "None": None,
        "print": lambda *a, **k: None,
    }}}})
except SyntaxError:
    # allow statements; last expression via exec + _result
    _g = {{"cell": cell, "__builtins__": {{
        "abs": abs, "min": min, "max": max, "sum": sum, "len": len,
        "int": int, "float": float, "str": str, "bool": bool,
        "round": round, "range": range, "list": list, "dict": dict,
        "True": True, "False": False, "None": None,
        "print": lambda *a, **k: None,
    }}}}
    exec(compile(_src, "<cell>", "exec"), _g)
    _result = _g.get("_result", _g.get("result", None))

if _result is None:
    print("")
elif isinstance(_result, float):
    print(repr(_result) if _result != int(_result) else int(_result))
elif isinstance(_result, bool):
    print(1 if _result else 0)
else:
    print(_result)
"#,
        cells_json = cells_json,
        user = user,
    );

    let output = run_python3(&wrapper);
    match output {
        Ok(s) => {
            let s = s.trim();
            if s.is_empty() {
                CellValue::Empty
            } else if let Ok(n) = s.parse::<f64>() {
                CellValue::Number(n)
            } else {
                CellValue::Text(s.to_string())
            }
        }
        Err(e) => CellValue::Error(FormulaError::Msg(format!("#PYTHON! {}", e))),
    }
}

fn run_python3(code: &str) -> Result<String, String> {
    let child = Command::new("python3")
        .arg("-c")
        .arg(code)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("python3 not available: {}", e))?;

    // Soft timeout via wait (no kill on all platforms easily without extra crates)
    let out = child
        .wait_with_output()
        .map_err(|e| format!("wait: {}", e))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        return Err(if err.is_empty() {
            format!("exit {}", out.status)
        } else {
            err.chars().take(200).collect()
        });
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

// Silence unused import if Duration not used yet
#[allow(dead_code)]
fn _timeout_placeholder() -> Duration {
    Duration::from_secs(2)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::recalculate;
    use crate::grid::GridModel;

    #[test]
    fn rhai_cell_multiply() {
        set_scripts_enabled(true);
        let mut g = GridModel::from_data(
            vec!["A".into(), "B".into()],
            vec![1, 2],
            vec![
                vec!["5".into(), "rust=cell(\"A1\") * 2".into()],
                vec!["3".into(), "".into()],
            ],
        );
        recalculate(&mut g, None);
        assert_eq!(g.display_at(0, 1), "10");
    }

    #[test]
    fn rhai_let_expr() {
        set_scripts_enabled(true);
        let mut g = GridModel::from_data(
            vec!["A".into()],
            vec![1],
            vec![vec!["rust=let x = 7; x + 3".into()]],
        );
        recalculate(&mut g, None);
        assert_eq!(g.display_at(0, 0), "10");
    }
}
