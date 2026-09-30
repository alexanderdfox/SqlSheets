//! SQLSheet — SQLite + spreadsheet formulas in the terminal.
//!
//! Keys:
//!   Tab          switch focus (tables / grid / sql)
//!   ↑↓←→ / hjkl  navigate grid
//!   Enter        edit cell
//!   e            edit cell (same)
//!   r            recalculate formulas
//!   n            new table
//!   i            import CSV (path prompt)
//!   o            open SQLite file
//!   s            save SQLite
//!   x            export CSV
//!   ? / F1       help
//!   c            cell color / font format
//!   [ / ]        prev / next row window (large tables)
//!   t            toggle rust=/python= scripts
//!   q / Ctrl-C   quit
//!   In SQL pane: Ctrl-Enter run (or F5)

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Row, Table, Wrap},
    Frame, Terminal,
};
use sqlsheets_core::{
    formula::recalculate, scripts_enabled, set_scripts_enabled, CellFmt, Database, GridModel,
    SqlResult,
};
use std::{
    io::{self, stdout},
    path::PathBuf,
    time::Duration,
};

#[derive(Parser, Debug)]
#[command(name = "sqlsheets", about = "SQLite + Spreadsheet + Formulas (TUI)")]
struct Args {
    /// Open an existing .sqlite / .db file
    file: Option<PathBuf>,
    /// Start with demo data if no file given
    #[arg(long)]
    demo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Tables,
    Grid,
    Sql,
    Modal,
}

#[derive(Debug)]
enum Modal {
    None,
    EditCell { buffer: String },
    Prompt { title: String, buffer: String, action: PromptAction },
    Message { text: String },
    /// Color format: cycle text/bg presets with keys
    Format { text_idx: usize, bg_idx: usize, bold: bool },
    Help,
}

#[derive(Debug, Clone)]
enum PromptAction {
    OpenFile,
    SaveFile,
    ImportCsv,
    ExportCsv,
    NewTable,
    SqlSaveAsTable,
}

struct App {
    db: Database,
    tables: Vec<sqlsheets_core::TableInfo>,
    table_idx: usize,
    grid: GridModel,
    focus: Focus,
    cursor_row: usize,
    cursor_col: usize,
    scroll_row: usize,
    scroll_col: usize,
    sql_input: String,
    sql_result: String,
    status: String,
    modal: Modal,
    table_list_state: ListState,
    should_quit: bool,
}

impl App {
    fn new(db: Database) -> Result<Self> {
        let mut app = Self {
            db,
            tables: vec![],
            table_idx: 0,
            grid: GridModel::empty(),
            focus: Focus::Grid,
            cursor_row: 0,
            cursor_col: 0,
            scroll_row: 0,
            scroll_col: 0,
            sql_input: "SELECT name FROM sqlite_master WHERE type='table';".into(),
            sql_result: String::new(),
            status: "Tab · Enter edit · r recalc · t scripts on/off · o open · s save · q quit".into(),
            modal: Modal::None,
            table_list_state: ListState::default(),
            should_quit: false,
        };
        app.refresh_tables()?;
        Ok(app)
    }

    fn refresh_tables(&mut self) -> Result<()> {
        self.tables = self.db.list_tables()?;
        if self.tables.is_empty() {
            self.grid = GridModel::empty();
            self.table_idx = 0;
            self.table_list_state.select(None);
        } else {
            if self.table_idx >= self.tables.len() {
                self.table_idx = 0;
            }
            self.table_list_state.select(Some(self.table_idx));
            self.load_current_table()?;
        }
        Ok(())
    }

    fn load_current_table(&mut self) -> Result<()> {
        self.load_window(0)
    }

    fn load_window(&mut self, offset: usize) -> Result<()> {
        if let Some(t) = self.tables.get(self.table_idx).map(|t| t.name.clone()) {
            self.grid = GridModel::from_db_window(&self.db, &t, offset, Some(GridModel::WINDOW_SIZE))?;
            self.cursor_row = 0;
            self.cursor_col = 0;
            self.scroll_row = 0;
            self.scroll_col = 0;
            self.status = format!(
                "Table `{}` — rows {}–{} of {} × {} cols  |  c=color  [=prev ]=next",
                t,
                offset + 1,
                offset + self.grid.row_count(),
                self.grid.total_rows.max(self.grid.row_count()),
                self.grid.col_count()
            );
        }
        Ok(())
    }

    /// Slide the virtual window if the cursor's absolute row is near an edge / outside.
    fn ensure_cursor_window(&mut self) -> Result<()> {
        let abs = self.grid.row_offset + self.cursor_row;
        if !self.grid.needs_window_shift(abs) && self.grid.local_row(abs).is_some() {
            return Ok(());
        }
        let new_off = GridModel::window_offset_for(abs, self.grid.total_rows);
        if new_off == self.grid.row_offset {
            return Ok(());
        }
        let keep_abs = abs;
        self.load_window(new_off)?;
        self.cursor_row = keep_abs.saturating_sub(self.grid.row_offset)
            .min(self.grid.row_count().saturating_sub(1));
        Ok(())
    }

        fn select_table(&mut self, idx: usize) -> Result<()> {
        if idx < self.tables.len() {
            self.table_idx = idx;
            self.table_list_state.select(Some(idx));
            self.load_current_table()?;
        }
        Ok(())
    }

    fn edit_current_cell(&mut self) {
        if self.grid.row_count() == 0 || self.grid.col_count() == 0 {
            return;
        }
        let raw = self
            .grid
            .raw_at(self.cursor_row, self.cursor_col)
            .unwrap_or_default();
        self.modal = Modal::EditCell { buffer: raw };
        self.focus = Focus::Modal;
    }

    fn commit_edit(&mut self, value: String) -> Result<()> {
        let row = self.cursor_row;
        let col = self.cursor_col;
        if let (Some(rowid), Some(col_name)) = (self.grid.rowid_at(row), self.grid.col_name(col)) {
            self.db
                .set_cell(&self.grid.table_name, rowid, col_name, &value)?;
            self.grid.set_raw(row, col, value);
            recalculate(&mut self.grid, Some(&self.db));
            self.status = format!("Updated {}{}", sqlsheets_core::col_letter(col), row + 1);
        }
        self.modal = Modal::None;
        self.focus = Focus::Grid;
        Ok(())
    }

    fn run_sql(&mut self) -> Result<()> {
        let sql = self.sql_input.clone();
        match self.db.execute_sql(&sql) {
            Ok(SqlResult::Table { columns, rows }) => {
                let mut out = String::new();
                out.push_str(&columns.join(" | "));
                out.push('\n');
                out.push_str(&"-".repeat(columns.len().saturating_mul(8).max(20)));
                out.push('\n');
                for (i, row) in rows.iter().enumerate() {
                    if i >= 50 {
                        out.push_str(&format!("… {} more rows\n", rows.len() - 50));
                        break;
                    }
                    out.push_str(&row.join(" | "));
                    out.push('\n');
                }
                self.sql_result = out;
                self.status = format!("Query OK — {} row(s)", rows.len());
                // Refresh tables in case DDL
                let _ = self.refresh_tables();
            }
            Ok(SqlResult::Message(m)) => {
                self.sql_result = m.clone();
                self.status = m;
                let _ = self.refresh_tables();
            }
            Err(e) => {
                self.sql_result = format!("Error: {e}");
                self.status = "SQL error".into();
            }
        }
        Ok(())
    }

    fn open_prompt(&mut self, title: &str, action: PromptAction) {
        self.modal = Modal::Prompt {
            title: title.into(),
            buffer: String::new(),
            action,
        };
        self.focus = Focus::Modal;
    }

    fn handle_prompt(&mut self, buffer: String, action: PromptAction) -> Result<()> {
        self.modal = Modal::None;
        self.focus = Focus::Grid;
        match action {
            PromptAction::OpenFile => {
                let path = PathBuf::from(buffer.trim());
                self.db = Database::open(&path).with_context(|| format!("open {:?}", path))?;
                self.refresh_tables()?;
                self.status = format!("Opened {}", path.display());
            }
            PromptAction::SaveFile => {
                let path = if buffer.trim().is_empty() {
                    PathBuf::from(&self.db.file_name)
                } else {
                    PathBuf::from(buffer.trim())
                };
                self.db.save_to_path(&path)?;
                self.db.file_name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("sqlsheet.sqlite")
                    .to_string();
                self.status = format!("Saved {}", path.display());
            }
            PromptAction::ImportCsv => {
                let path = PathBuf::from(buffer.trim());
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("imported")
                    .to_string();
                let n = self.db.import_csv(&name, &path)?;
                self.refresh_tables()?;
                // select new table
                if let Some(idx) = self.tables.iter().position(|t| t.name == name) {
                    self.select_table(idx)?;
                }
                self.status = format!("Imported {n} rows into `{name}`");
            }
            PromptAction::ExportCsv => {
                if self.grid.table_name.is_empty() {
                    self.status = "No table selected".into();
                    return Ok(());
                }
                let path = if buffer.trim().is_empty() {
                    PathBuf::from(format!("{}.csv", self.grid.table_name))
                } else {
                    PathBuf::from(buffer.trim())
                };
                let n = self.db.export_csv(&self.grid.table_name, &path)?;
                self.status = format!("Exported {n} rows → {}", path.display());
            }
            PromptAction::NewTable => {
                let name = buffer.trim();
                if name.is_empty() {
                    return Ok(());
                }
                self.db.create_table(
                    name,
                    &[
                        ("A".into(), "TEXT".into()),
                        ("B".into(), "TEXT".into()),
                        ("C".into(), "TEXT".into()),
                    ],
                )?;
                // insert a few empty rows
                for _ in 0..5 {
                    self.db.insert_row(
                        name,
                        &["A".into(), "B".into(), "C".into()],
                        &["".into(), "".into(), "".into()],
                    )?;
                }
                self.refresh_tables()?;
                if let Some(idx) = self.tables.iter().position(|t| t.name == name) {
                    self.select_table(idx)?;
                }
                self.status = format!("Created table `{name}`");
            }
            PromptAction::SqlSaveAsTable => {
                // not wired in this minimal version
                self.status = "Save-as-table: run CREATE TABLE AS in SQL pane".into();
            }
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let db = if let Some(path) = args.file {
        Database::open(&path).with_context(|| format!("open {:?}", path))?
    } else {
        let db = Database::new_in_memory()?;
        if args.demo {
            db.seed_demo_if_empty()?;
            // Fix demo formulas to use correct 1-based rows matching inserted order
            // Re-seed with correct formula row numbers after knowing order
        }
        db.seed_demo_if_empty()?;
        db
    };

    // Fix demo formulas: after seed, reload and patch totals to correct A1 refs
    // (seed uses fixed B2 etc which matches insertion order)

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(db)?;
    let res = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    res
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if handle_key(app, key.code, key.modifiers)? {
                    break;
                }
            }
        }
        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Result<bool> {
    // Modal takes all input
    if matches!(app.focus, Focus::Modal) {
        return handle_modal_key(app, code, mods);
    }

    if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        return Ok(true);
    }

    match code {
        KeyCode::Char('q') if app.focus != Focus::Sql => return Ok(true),
        KeyCode::Tab => {
            app.focus = match app.focus {
                Focus::Tables => Focus::Grid,
                Focus::Grid => Focus::Sql,
                Focus::Sql => Focus::Tables,
                Focus::Modal => Focus::Grid,
            };
        }
        KeyCode::F(5) => {
            app.run_sql()?;
        }
        KeyCode::Char('r') | KeyCode::Char(' ') if app.focus == Focus::Grid => {
            recalculate(&mut app.grid, Some(&app.db));
            app.status = "Step · recalculated".into();
        }
        KeyCode::Char('b') if app.focus == Focus::Grid => {
            if !scripts_enabled() {
                app.status = "Enable Scripts first (press t)".into();
            } else {
                recalculate(&mut app.grid, Some(&app.db));
                match app.db.bake_table_values(&app.grid.table_name.clone(), &app.grid) {
                    Ok(n) => {
                        let name = app.grid.table_name.clone();
                        let off = app.grid.row_offset;
                        let _ = app.load_window(off);
                        app.status = format!("Baked {n} cells");
                        let _ = name;
                    }
                    Err(e) => app.status = format!("Bake: {e}"),
                }
            }
        }
        KeyCode::Char('+') if app.focus == Focus::Grid => {
            let table = app.grid.table_name.clone();
            if !table.is_empty() {
                let cols = app.grid.col_names.clone();
                let vals: Vec<String> = cols.iter().map(|_| String::new()).collect();
                if app.db.insert_row(&table, &cols, &vals).is_ok() {
                    let off = app.grid.row_offset;
                    let _ = app.load_window(off);
                    app.status = "Row added".into();
                }
            }
        }
        KeyCode::Char('t') if app.focus != Focus::Sql => {
            let on = !scripts_enabled();
            set_scripts_enabled(on);
            recalculate(&mut app.grid, Some(&app.db));
            app.status = if on {
                "Scripts: on (script=/rust=/python=)".into()
            } else {
                "Scripts: off".into()
            };
        }
        KeyCode::Char('?') | KeyCode::F(1) => {
            app.modal = Modal::Help;
            app.focus = Focus::Modal;
        }
        KeyCode::Char('c') if app.focus == Focus::Grid => {
            let fmt = app.grid.fmt_at(app.cursor_row, app.cursor_col);
            let text_idx = PRESET_HEX.iter().position(|&h| h == fmt.text_color.as_str()).unwrap_or(0);
            let bg_idx = PRESET_HEX.iter().position(|&h| h == fmt.bg_color.as_str()).unwrap_or(0);
            app.modal = Modal::Format { text_idx, bg_idx, bold: fmt.bold };
            app.focus = Focus::Modal;
        }
        KeyCode::Char('[') if app.focus == Focus::Grid => {
            let off = app.grid.row_offset.saturating_sub(GridModel::WINDOW_SIZE);
            app.load_window(off)?;
        }
        KeyCode::Char(']') if app.focus == Focus::Grid => {
            let off = (app.grid.row_offset + GridModel::WINDOW_SIZE).min(app.grid.total_rows.saturating_sub(1));
            app.load_window(off)?;
        }
        KeyCode::Char('o') if app.focus != Focus::Sql => {
            app.open_prompt("Open SQLite file path:", PromptAction::OpenFile);
        }
        KeyCode::Char('s') if app.focus != Focus::Sql => {
            app.open_prompt(
                "Save SQLite path (empty = current name):",
                PromptAction::SaveFile,
            );
        }
        KeyCode::Char('i') if app.focus != Focus::Sql => {
            app.open_prompt("Import CSV path:", PromptAction::ImportCsv);
        }
        KeyCode::Char('x') if app.focus != Focus::Sql => {
            app.open_prompt(
                "Export CSV path (empty = table.csv):",
                PromptAction::ExportCsv,
            );
        }
        KeyCode::Char('n') if app.focus != Focus::Sql => {
            app.open_prompt("New table name:", PromptAction::NewTable);
        }
        KeyCode::Enter | KeyCode::Char('e') if app.focus == Focus::Grid => {
            app.edit_current_cell();
        }
        _ => match app.focus {
            Focus::Tables => handle_tables_key(app, code)?,
            Focus::Grid => handle_grid_key(app, code)?,
            Focus::Sql => handle_sql_key(app, code, mods)?,
            Focus::Modal => {}
        },
    }
    Ok(false)
}

fn handle_modal_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) -> Result<bool> {
    match &mut app.modal {
        Modal::EditCell { buffer } => match code {
            KeyCode::Esc => {
                app.modal = Modal::None;
                app.focus = Focus::Grid;
            }
            KeyCode::Enter => {
                let v = buffer.clone();
                app.commit_edit(v)?;
            }
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(c) => buffer.push(c),
            _ => {}
        },
        Modal::Prompt { buffer, action, .. } => match code {
            KeyCode::Esc => {
                app.modal = Modal::None;
                app.focus = Focus::Grid;
            }
            KeyCode::Enter => {
                let buf = buffer.clone();
                let act = action.clone();
                app.handle_prompt(buf, act)?;
            }
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(c) => buffer.push(c),
            _ => {}
        },
        Modal::Message { .. } => {
            app.modal = Modal::None;
            app.focus = Focus::Grid;
        }
        Modal::Help => {
            app.modal = Modal::None;
            app.focus = Focus::Grid;
        }
        Modal::Format { text_idx, bg_idx, bold } => match code {
            KeyCode::Esc => {
                app.modal = Modal::None;
                app.focus = Focus::Grid;
            }
            KeyCode::Enter => {
                let fmt = CellFmt {
                    text_color: PRESET_HEX[*text_idx].to_string(),
                    bg_color: if *bg_idx == 0 { String::new() } else { PRESET_HEX[*bg_idx].to_string() },
                    font_size: 0,
                    bold: *bold,
                };
                let r = app.cursor_row;
                let c = app.cursor_col;
                app.grid.set_fmt(r, c, fmt);
                let _ = app.grid.save_fmt_cell(&app.db, r, c);
                app.status = format!("Format applied to {}", app.grid.a1(r, c));
                app.modal = Modal::None;
                app.focus = Focus::Grid;
            }
            KeyCode::Char('t') | KeyCode::Right => {
                *text_idx = (*text_idx + 1) % PRESET_HEX.len();
            }
            KeyCode::Char('T') | KeyCode::Left => {
                *text_idx = (*text_idx + PRESET_HEX.len() - 1) % PRESET_HEX.len();
            }
            KeyCode::Char('b') | KeyCode::Down => {
                *bg_idx = (*bg_idx + 1) % PRESET_HEX.len();
            }
            KeyCode::Char('B') | KeyCode::Up => {
                *bg_idx = (*bg_idx + PRESET_HEX.len() - 1) % PRESET_HEX.len();
            }
            KeyCode::Char('f') => {
                *bold = !*bold;
            }
            KeyCode::Char('x') => {
                // clear
                let r = app.cursor_row;
                let c = app.cursor_col;
                app.grid.set_fmt(r, c, CellFmt::default());
                let _ = app.grid.save_fmt_cell(&app.db, r, c);
                app.modal = Modal::None;
                app.focus = Focus::Grid;
                app.status = "Format cleared".into();
            }
            _ => {}
        },
        Modal::None => {
            app.focus = Focus::Grid;
        }
    }
    Ok(false)
}

fn handle_tables_key(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            if app.table_idx > 0 {
                app.select_table(app.table_idx - 1)?;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.table_idx + 1 < app.tables.len() {
                app.select_table(app.table_idx + 1)?;
            }
        }
        KeyCode::Enter => {
            app.focus = Focus::Grid;
        }
        _ => {}
    }
    Ok(())
}

fn handle_grid_key(app: &mut App, code: KeyCode) -> Result<()> {
    let rows = app.grid.row_count();
    let cols = app.grid.col_count();
    if rows == 0 || cols == 0 {
        return Ok(());
    }
    // Absolute navigation across the full table when virtualized
    let total = app.grid.total_rows.max(rows);
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            let abs = app.grid.row_offset + app.cursor_row;
            if abs > 0 {
                let new_abs = abs - 1;
                if let Some(local) = app.grid.local_row(new_abs) {
                    app.cursor_row = local;
                } else {
                    app.load_window(GridModel::window_offset_for(new_abs, total))?;
                    app.cursor_row = new_abs.saturating_sub(app.grid.row_offset);
                }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let abs = app.grid.row_offset + app.cursor_row;
            if abs + 1 < total {
                let new_abs = abs + 1;
                if let Some(local) = app.grid.local_row(new_abs) {
                    app.cursor_row = local;
                } else {
                    app.load_window(GridModel::window_offset_for(new_abs, total))?;
                    app.cursor_row = new_abs.saturating_sub(app.grid.row_offset)
                        .min(app.grid.row_count().saturating_sub(1));
                }
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if app.cursor_col > 0 {
                app.cursor_col -= 1;
            }
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if app.cursor_col + 1 < cols {
                app.cursor_col += 1;
            }
        }
        KeyCode::Home => app.cursor_col = 0,
        KeyCode::End => app.cursor_col = cols.saturating_sub(1),
        KeyCode::PageUp => {
            let abs = (app.grid.row_offset + app.cursor_row).saturating_sub(40);
            app.load_window(GridModel::window_offset_for(abs, total))?;
            app.cursor_row = abs.saturating_sub(app.grid.row_offset);
        }
        KeyCode::PageDown => {
            let abs = (app.grid.row_offset + app.cursor_row + 40).min(total.saturating_sub(1));
            app.load_window(GridModel::window_offset_for(abs, total))?;
            app.cursor_row = abs.saturating_sub(app.grid.row_offset)
                .min(app.grid.row_count().saturating_sub(1));
        }
        _ => {}
    }
    app.ensure_cursor_window()?;
    Ok(())
}

fn handle_sql_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Result<()> {
    match code {
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => {
            // leave to global
        }
        KeyCode::Enter if mods.contains(KeyModifiers::CONTROL) => {
            app.run_sql()?;
        }
        KeyCode::Char(c) => app.sql_input.push(c),
        KeyCode::Backspace => {
            app.sql_input.pop();
        }
        KeyCode::Esc => app.focus = Focus::Grid,
        _ => {}
    }
    Ok(())
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Min(5),
            Constraint::Length(8), // sql
            Constraint::Length(1), // status
        ])
        .split(f.size());

    // Title
    let title = Paragraph::new(Line::from(vec![
        Span::styled(" SQLSheet ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(
            "· SQLite + formulas  ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(format!("[{}]", app.db.file_name)),
    ]));
    f.render_widget(title, chunks[0]);

    // Main: sidebar + grid
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(20)])
        .split(chunks[1]);

    draw_tables(f, app, main[0]);
    draw_grid(f, app, main[1]);
    draw_sql(f, app, chunks[2]);

    // Status
    let status = Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::Yellow));
    f.render_widget(status, chunks[3]);

    // Modal overlay
    match &app.modal {
        Modal::EditCell { buffer } => {
            draw_modal(
                f,
                "Edit cell (Enter=save, Esc=cancel)",
                buffer,
                &format!(
                    "{}{}",
                    sqlsheets_core::col_letter(app.cursor_col),
                    app.cursor_row + 1
                ),
            );
        }
        Modal::Prompt { title, buffer, .. } => {
            draw_modal(f, title, buffer, "");
        }
        Modal::Message { text } => {
            draw_modal(f, "Message", text, "");
        }
                Modal::Help => {
            let body = HELP_TEXT;
            draw_modal(f, "Help (? / F1)", body, "");
        }
        Modal::Format { text_idx, bg_idx, bold } => {
            let body = format!(
                "Text: {}  Fill: {}  Bold: {}\n[t/T] text  [b/B] fill  [f] bold  [Enter] apply  [x] clear  [Esc]",
                PRESET_NAMES[*text_idx],
                PRESET_NAMES[*bg_idx],
                if *bold { "ON" } else { "off" }
            );
            draw_modal(f, "Cell format", &body, &app.grid.a1(app.cursor_row, app.cursor_col));
        }
        Modal::None => {}
    }
}

fn draw_tables(f: &mut Frame, app: &App, area: Rect) {
    let border = if app.focus == Focus::Tables {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let items: Vec<ListItem> = app
        .tables
        .iter()
        .map(|t| {
            ListItem::new(format!(" {} ({})", t.name, t.row_count))
        })
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Tables ").border_style(border))
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("► ");
    let mut state = app.table_list_state.clone();
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_grid(f: &mut Frame, app: &App, area: Rect) {
    let border = if app.focus == Focus::Grid {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " {} ",
            if app.grid.table_name.is_empty() {
                "(no table)".into()
            } else {
                app.grid.table_name.clone()
            }
        ))
        .border_style(border);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.grid.col_count() == 0 {
        let p = Paragraph::new("No table selected.\nPress n to create, o to open, or i to import CSV.")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(p, inner);
        return;
    }

    // Visible columns / rows
    let col_width = 12u16;
    let visible_cols = ((inner.width.saturating_sub(4)) / col_width).max(1) as usize;
    let visible_rows = inner.height.saturating_sub(1) as usize;

    let start_col = app
        .cursor_col
        .saturating_sub(visible_cols.saturating_sub(1) / 2)
        .min(app.grid.col_count().saturating_sub(visible_cols));
    let start_row = app
        .cursor_row
        .saturating_sub(visible_rows.saturating_sub(1) / 2)
        .min(app.grid.row_count().saturating_sub(visible_rows));

    // Header row
    let mut header_cells = vec![ratatui::widgets::Cell::from("#")
        .style(Style::default().fg(Color::DarkGray))];
    for c in start_col..(start_col + visible_cols).min(app.grid.col_count()) {
        let letter = sqlsheets_core::col_letter(c);
        let name = app.grid.col_names.get(c).map(|s| s.as_str()).unwrap_or("");
        let label = format!("{}:{}", letter, name);
        let style = if c == app.cursor_col {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };
        header_cells.push(ratatui::widgets::Cell::from(label).style(style));
    }
    let header = Row::new(header_cells).height(1);

    let mut rows = Vec::new();
    for r in start_row..(start_row + visible_rows).min(app.grid.row_count()) {
        let mut cells = vec![ratatui::widgets::Cell::from((r + 1).to_string())
            .style(Style::default().fg(Color::DarkGray))];
        for c in start_col..(start_col + visible_cols).min(app.grid.col_count()) {
            let text = app.grid.display_at(r, c);
            let fmt = app.grid.fmt_at(r, c);
            let mut style = Style::default();
            if let Some(col) = hex_to_color(&fmt.text_color) {
                style = style.fg(col);
            } else if let Some(k) = app.grid.formula_kind(r, c) {
                style = style.fg(match k {
                    "fx" => Color::Magenta,
                    "rust" => Color::Green,
                    "python" => Color::Yellow,
                    "sql" => Color::Cyan,
                    _ => Color::White,
                });
            }
            if let Some(col) = hex_to_color(&fmt.bg_color) {
                style = style.bg(col);
            }
            if fmt.bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            if r == app.cursor_row && c == app.cursor_col {
                style = style.bg(Color::Blue).fg(Color::White).add_modifier(Modifier::BOLD);
            }
            // Truncate
            let display = if text.chars().count() > 11 {
                format!("{}…", text.chars().take(10).collect::<String>())
            } else {
                text
            };
            cells.push(ratatui::widgets::Cell::from(display).style(style));
        }
        rows.push(Row::new(cells).height(1));
    }

    let widths: Vec<Constraint> = std::iter::once(Constraint::Length(4))
        .chain((0..visible_cols).map(|_| Constraint::Length(col_width)))
        .collect();

    let table = Table::new(rows, widths).header(header);
    f.render_widget(table, inner);
}

fn draw_sql(f: &mut Frame, app: &App, area: Rect) {
    let border = if app.focus == Focus::Sql {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let input = Paragraph::new(app.sql_input.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" SQL (Ctrl+Enter / F5) ")
                .border_style(border),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(input, chunks[0]);

    let result = Paragraph::new(app.sql_result.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Result ")
                .border_style(border),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(result, chunks[1]);
}

fn draw_modal(f: &mut Frame, title: &str, body: &str, subtitle: &str) {
    let h = if title.contains("Help") { 10 } else { 5 };
    let area = centered_rect(72, h, f.size());
    f.render_widget(Clear, area);
    let text = if subtitle.is_empty() {
        body.to_string()
    } else {
        format!("[{}] {}", subtitle, body)
    };
    let p = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", title))
                .border_style(Style::default().fg(Color::Green)),
        )
        .style(Style::default().fg(Color::White));
    f.render_widget(p, area);
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height) / 2),
            Constraint::Length(height),
            Constraint::Percentage((100 - height) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Preset palette for TUI color picker (index 0 = default / none for bg).
const PRESET_HEX: &[&str] = &[
    "",
    "#E8EEF7",
    "#EF4444",
    "#22C55E",
    "#3B82F6",
    "#EAB308",
    "#A78BFA",
    "#F97316",
    "#0B0F14",
    "#1E2736",
];
const PRESET_NAMES: &[&str] = &[
    "default", "white", "red", "green", "blue", "yellow", "purple", "orange", "black", "surface",
];

fn hex_to_color(hex: &str) -> Option<Color> {
    let (r, g, b) = CellFmt::parse_rgb(hex)?;
    Some(Color::Rgb(r, g, b))
}

const HELP_TEXT: &str = "SQLSheet - SQLite + formulas (port of original)\nSpace/r Step  b Bake  t Scripts on/off  + add row  c colors\no open  s save  i/x CSV  n new table  [ ] window  F5 SQL\n=SUM(A1:A10)  script=cell(\"A1\")*2  sql=SELECT ...\nScripts OFF by default. Virtual 512-row window for large tables. q quit";
