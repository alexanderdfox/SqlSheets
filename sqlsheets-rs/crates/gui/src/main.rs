//! SqlSheets GUI — polished egui front-end with virtualized grid + cell formatting.
//!
//! Port of https://alexanderdfox.github.io/SqlSheets/

use clap::Parser;
use eframe::egui::{self, Color32, FontId, RichText, Sense, Vec2};
use sqlsheets_core::{
    col_letter, formula::recalculate, scripts_enabled, set_scripts_enabled, CellFmt, Database,
    GridModel, SqlResult, TableInfo,
};
use std::path::PathBuf;

const WINDOW_ROWS: usize = sqlsheets_core::GridModel::WINDOW_SIZE;
const ROW_HEIGHT: f32 = 26.0;
const COL_WIDTH: f32 = 110.0;
const ROW_NUM_W: f32 = 48.0;

#[derive(Parser, Debug)]
#[command(name = "sqlsheets-gui", about = "SQLite + Spreadsheet + Formulas (GUI)")]
struct Args {
    file: Option<PathBuf>,
    #[arg(long)]
    demo: bool,
}

struct SqlSheetsApp {
    db: Database,
    tables: Vec<TableInfo>,
    selected_table: Option<String>,
    grid: GridModel,
    cursor: (usize, usize),
    edit_buffer: String,
    editing: bool,
    sql_input: String,
    sql_output: String,
    status: String,
    formula_bar: String,
    pending_select: Option<String>,
    pick_text: [f32; 3],
    pick_bg: [f32; 3],
    use_text_color: bool,
    use_bg_color: bool,
    font_size_step: i8,
    bold: bool,
    dark: bool,
    scroll_to_row: Option<usize>,
    show_help: bool,
    play_running: bool,
    play_interval_ms: u64,
    last_play: std::time::Instant,
}

impl SqlSheetsApp {
    fn new(db: Database) -> Self {
        let mut app = Self {
            db,
            tables: vec![],
            selected_table: None,
            grid: GridModel::empty(),
            cursor: (0, 0),
            edit_buffer: String::new(),
            editing: false,
            sql_input: "SELECT name FROM sqlite_master WHERE type='table';".into(),
            sql_output: String::new(),
            status: "Ready".into(),
            formula_bar: String::new(),
            pending_select: None,
            pick_text: [0.91, 0.93, 0.97],
            pick_bg: [0.08, 0.10, 0.14],
            use_text_color: false,
            use_bg_color: false,
            font_size_step: 0,
            bold: false,
            dark: true,
            scroll_to_row: None,
            show_help: false,
            play_running: false,
            play_interval_ms: 150,
            last_play: std::time::Instant::now(),
        };
        let _ = app.refresh();
        app
    }

    fn refresh(&mut self) -> anyhow::Result<()> {
        self.tables = self.db.list_tables()?;
        if let Some(name) = self.selected_table.clone() {
            if self.tables.iter().any(|t| t.name == name) {
                self.load_table(&name)?;
            } else {
                let first = self.tables.first().map(|t| t.name.clone());
                self.selected_table = first.clone();
                if let Some(n) = first {
                    self.load_table(&n)?;
                } else {
                    self.grid = GridModel::empty();
                }
            }
        } else {
            let first = self.tables.first().map(|t| t.name.clone());
            self.selected_table = first.clone();
            if let Some(n) = first {
                self.load_table(&n)?;
            }
        }
        Ok(())
    }

    fn load_table(&mut self, name: &str) -> anyhow::Result<()> {
        self.load_window(name, 0)
    }

    fn load_window(&mut self, name: &str, offset: usize) -> anyhow::Result<()> {
        self.grid = GridModel::from_db_window(&self.db, name, offset, Some(WINDOW_ROWS))?;
        self.cursor = (0, 0);
        self.editing = false;
        self.sync_fmt_pickers();
        self.update_formula_bar();
        self.status = format!(
            "Table `{}` — showing {}–{} of {} rows × {} cols",
            name,
            offset + 1,
            offset + self.grid.row_count(),
            self.grid.total_rows.max(self.grid.row_count()),
            self.grid.col_count()
        );
        Ok(())
    }

    fn ensure_row_visible(&mut self, absolute_row: usize) {
        let total = self.grid.total_rows.max(1);
        let abs = absolute_row.min(total.saturating_sub(1));
        let start = self.grid.row_offset;
        let end = start + self.grid.row_count();
        if abs < start || abs >= end {
            let new_off = abs.saturating_sub(WINDOW_ROWS / 4);
            if let Some(name) = self.selected_table.clone() {
                let _ = self.load_window(&name, new_off);
                self.cursor.0 = abs.saturating_sub(self.grid.row_offset);
            }
        } else {
            self.cursor.0 = abs.saturating_sub(start);
        }
        self.scroll_to_row = Some(self.cursor.0);
    }

    fn update_formula_bar(&mut self) {
        let (r, c) = self.cursor;
        self.formula_bar = self.grid.raw_at(r, c).unwrap_or_default();
    }

    fn sync_fmt_pickers(&mut self) {
        let (r, c) = self.cursor;
        let fmt = self.grid.fmt_at(r, c);
        self.use_text_color = !fmt.text_color.is_empty();
        self.use_bg_color = !fmt.bg_color.is_empty();
        self.font_size_step = fmt.font_size;
        self.bold = fmt.bold;
        if let Some((r, g, b)) = CellFmt::parse_rgb(&fmt.text_color) {
            self.pick_text = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
        }
        if let Some((r, g, b)) = CellFmt::parse_rgb(&fmt.bg_color) {
            self.pick_bg = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
        }
    }

    fn apply_fmt_to_cursor(&mut self) {
        let (r, c) = self.cursor;
        let fmt = CellFmt {
            text_color: if self.use_text_color {
                rgb_hex(self.pick_text)
            } else {
                String::new()
            },
            bg_color: if self.use_bg_color {
                rgb_hex(self.pick_bg)
            } else {
                String::new()
            },
            font_size: self.font_size_step,
            bold: self.bold,
        };
        self.grid.set_fmt(r, c, fmt);
        let _ = self.grid.save_fmt_cell(&self.db, r, c);
        self.status = format!("Format saved for {}", self.grid.a1(r, c));
    }

    fn clear_fmt_cursor(&mut self) {
        let (r, c) = self.cursor;
        self.grid.set_fmt(r, c, CellFmt::default());
        let _ = self.grid.save_fmt_cell(&self.db, r, c);
        self.use_text_color = false;
        self.use_bg_color = false;
        self.font_size_step = 0;
        self.bold = false;
        self.status = format!("Format cleared for {}", self.grid.a1(r, c));
    }

    fn commit_cell(&mut self) {
        let (r, c) = self.cursor;
        if let (Some(rowid), Some(col)) = (self.grid.rowid_at(r), self.grid.col_name(c)) {
            let table = self.grid.table_name.clone();
            if self.db.set_cell(&table, rowid, col, &self.edit_buffer).is_ok() {
                self.grid.set_raw(r, c, self.edit_buffer.clone());
                recalculate(&mut self.grid, Some(&self.db));
                self.status = format!("Saved {}", self.grid.a1(r, c));
            }
        }
        self.editing = false;
        self.update_formula_bar();
    }

    fn run_sql(&mut self) {
        match self.db.execute_sql(&self.sql_input) {
            Ok(SqlResult::Table { columns, rows }) => {
                let mut out = columns.join(" │ ");
                out.push('\n');
                for (i, row) in rows.iter().enumerate() {
                    if i >= 100 {
                        out.push_str(&format!("\n… {} more", rows.len() - 100));
                        break;
                    }
                    out.push('\n');
                    out.push_str(&row.join(" │ "));
                }
                self.sql_output = out;
                self.status = format!("{} rows", rows.len());
                let _ = self.refresh();
            }
            Ok(SqlResult::Message(m)) => {
                self.sql_output = m.clone();
                self.status = m;
                let _ = self.refresh();
            }
            Err(e) => {
                self.sql_output = format!("Error: {e}");
                self.status = "SQL error".into();
            }
        }
    }

    fn open_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("SQLite", &["sqlite", "db", "sqlite3"])
            .pick_file()
        {
            match Database::open(&path) {
                Ok(db) => {
                    self.db = db;
                    self.selected_table = None;
                    let _ = self.refresh();
                    self.status = format!("Opened {}", path.display());
                }
                Err(e) => self.status = format!("Open failed: {e}"),
            }
        }
    }

    fn save_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("SQLite", &["sqlite", "db"])
            .set_file_name(&self.db.file_name)
            .save_file()
        {
            match self.db.save_to_path(&path) {
                Ok(()) => {
                    self.db.file_name = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("sqlsheet.sqlite")
                        .to_string();
                    self.status = format!("Saved {}", path.display());
                }
                Err(e) => self.status = format!("Save failed: {e}"),
            }
        }
    }

    fn import_csv(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .pick_file()
        {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("imported")
                .to_string();
            match self.db.import_csv(&name, &path) {
                Ok(n) => {
                    self.selected_table = Some(name.clone());
                    let _ = self.refresh();
                    self.status = format!("Imported {n} rows → `{name}`");
                }
                Err(e) => self.status = format!("Import failed: {e}"),
            }
        }
    }

    fn export_csv(&mut self) {
        let Some(table) = self.selected_table.clone() else {
            self.status = "No table".into();
            return;
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name(format!("{table}.csv"))
            .save_file()
        {
            match self.db.export_csv(&table, &path) {
                Ok(n) => self.status = format!("Exported {n} rows"),
                Err(e) => self.status = format!("Export failed: {e}"),
            }
        }
    }

    fn new_table(&mut self) {
        let name = format!("table{}", self.tables.len() + 1);
        if self
            .db
            .create_table(
                &name,
                &[
                    ("A".into(), "TEXT".into()),
                    ("B".into(), "TEXT".into()),
                    ("C".into(), "TEXT".into()),
                    ("D".into(), "TEXT".into()),
                ],
            )
            .is_ok()
        {
            for _ in 0..20 {
                let _ = self.db.insert_row(
                    &name,
                    &["A".into(), "B".into(), "C".into(), "D".into()],
                    &["".into(), "".into(), "".into(), "".into()],
                );
            }
            self.selected_table = Some(name.clone());
            let _ = self.refresh();
            self.status = format!("Created `{name}`");
        }
    }

    fn new_db(&mut self) {
        if let Ok(db) = Database::new_in_memory() {
            self.db = db;
            self.selected_table = None;
            self.grid = GridModel::empty();
            let _ = self.refresh();
            self.status = "New in-memory database".into();
        }
    }


    fn add_row(&mut self) {
        let Some(table) = self.selected_table.clone() else { return };
        let cols = self.grid.col_names.clone();
        if cols.is_empty() { return; }
        let vals: Vec<String> = cols.iter().map(|_| String::new()).collect();
        if self.db.insert_row(&table, &cols, &vals).is_ok() {
            let _ = self.load_table(&table);
            self.status = "Row added".into();
        }
    }

    fn add_col(&mut self) {
        let Some(table) = self.selected_table.clone() else { return };
        if let Ok(name) = self.db.next_col_letter_name(&table) {
            if self.db.add_column(&table, &name, "TEXT").is_ok() {
                // backfill empty for existing rows is automatic for SQLite ALTER
                let _ = self.load_table(&table);
                self.status = format!("Column `{name}` added");
            }
        }
    }

    fn del_row(&mut self) {
        let Some(table) = self.selected_table.clone() else { return };
        let r = self.cursor.0;
        if let Some(rowid) = self.grid.rowid_at(r) {
            if self.db.delete_row(&table, rowid).is_ok() {
                let _ = self.load_table(&table);
                self.status = "Row deleted".into();
            }
        }
    }

    fn del_col(&mut self) {
        self.status = "SQLite cannot DROP COLUMN on all versions — recreate table or leave unused".into();
    }

    fn drop_table(&mut self) {
        let Some(table) = self.selected_table.clone() else { return };
        if self.db.drop_table(&table).is_ok() {
            self.selected_table = None;
            let _ = self.refresh();
            self.status = format!("Dropped `{table}`");
        }
    }

    fn bake(&mut self) {
        if !scripts_enabled() {
            self.status = "Enable Scripts first so values can be computed before baking".into();
            return;
        }
        let Some(table) = self.selected_table.clone() else {
            self.status = "No table open".into();
            return;
        };
        recalculate(&mut self.grid, Some(&self.db));
        match self.db.bake_table_values(&table, &self.grid) {
            Ok(n) => {
                let _ = self.load_table(&table);
                self.status = format!("Baked {n} computed cells into plain values");
            }
            Err(e) => self.status = format!("Bake failed: {e}"),
        }
    }

    fn save_sql_as_table(&mut self) {
        // Parse last SQL output is weak; re-run and save
        match self.db.execute_sql(&self.sql_input) {
            Ok(SqlResult::Table { columns, rows }) => {
                let name = format!("result{}", self.tables.len() + 1);
                match self.db.save_query_as_table(&name, &columns, &rows) {
                    Ok(()) => {
                        self.selected_table = Some(name.clone());
                        let _ = self.refresh();
                        self.status = format!("Saved result as `{name}`");
                    }
                    Err(e) => self.status = format!("Save failed: {e}"),
                }
            }
            _ => self.status = "Run a SELECT first".into(),
        }
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        if self.dark {
            let mut v = egui::Visuals::dark();
            v.panel_fill = Color32::from_rgb(11, 15, 20);
            v.window_fill = Color32::from_rgb(21, 27, 36);
            v.extreme_bg_color = Color32::from_rgb(15, 20, 28);
            v.widgets.noninteractive.bg_fill = Color32::from_rgb(30, 39, 54);
            v.widgets.inactive.bg_fill = Color32::from_rgb(30, 39, 54);
            v.widgets.hovered.bg_fill = Color32::from_rgb(42, 53, 72);
            v.selection.bg_fill = Color32::from_rgb(37, 99, 235);
            v.hyperlink_color = Color32::from_rgb(96, 165, 250);
            style.visuals = v;
        } else {
            style.visuals = egui::Visuals::light();
        }
        style.spacing.item_spacing = Vec2::new(6.0, 4.0);
        style.spacing.button_padding = Vec2::new(10.0, 4.0);
        ctx.set_style(style);
    }

    fn draw_virtual_grid(&mut self, ui: &mut egui::Ui) {
        let cols = self.grid.col_count();
        let rows = self.grid.row_count();
        let total_w = ROW_NUM_W + cols as f32 * COL_WIDTH;
        let total_h = (rows + 1) as f32 * ROW_HEIGHT;

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                ui.set_height(total_h);
                ui.set_width(total_w.max(viewport.width()));

                let first_row = ((viewport.min.y / ROW_HEIGHT).floor() as usize).saturating_sub(1);
                let last_row = (((viewport.max.y / ROW_HEIGHT).ceil() as usize) + 1).min(rows);

                let painter = ui.painter();
                let origin = ui.min_rect().min;

                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(origin.x, origin.y),
                        Vec2::new(total_w, ROW_HEIGHT),
                    ),
                    0.0,
                    Color32::from_rgb(30, 39, 54),
                );

                painter.text(
                    egui::pos2(origin.x + ROW_NUM_W / 2.0, origin.y + ROW_HEIGHT / 2.0),
                    egui::Align2::CENTER_CENTER,
                    "#",
                    FontId::proportional(11.0),
                    Color32::from_rgb(132, 148, 173),
                );

                for c in 0..cols {
                    let x = origin.x + ROW_NUM_W + c as f32 * COL_WIDTH;
                    let letter = col_letter(c);
                    let name = self.grid.col_names.get(c).map(|s| s.as_str()).unwrap_or("");
                    painter.text(
                        egui::pos2(x + COL_WIDTH / 2.0, origin.y + 8.0),
                        egui::Align2::CENTER_CENTER,
                        &letter,
                        FontId::proportional(10.0),
                        Color32::from_rgb(59, 130, 246),
                    );
                    painter.text(
                        egui::pos2(x + COL_WIDTH / 2.0, origin.y + 18.0),
                        egui::Align2::CENTER_CENTER,
                        name,
                        FontId::proportional(11.0),
                        Color32::from_rgb(234, 179, 8),
                    );
                }

                for r in first_row..last_row {
                    if r >= rows {
                        break;
                    }
                    let y = origin.y + (r + 1) as f32 * ROW_HEIGHT;
                    let abs_row_label = self.grid.row_offset + r + 1;

                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(origin.x, y),
                            Vec2::new(ROW_NUM_W, ROW_HEIGHT),
                        ),
                        0.0,
                        Color32::from_rgb(30, 39, 54),
                    );
                    painter.text(
                        egui::pos2(origin.x + ROW_NUM_W / 2.0, y + ROW_HEIGHT / 2.0),
                        egui::Align2::CENTER_CENTER,
                        abs_row_label.to_string(),
                        FontId::proportional(11.0),
                        Color32::from_rgb(132, 148, 173),
                    );

                    for c in 0..cols {
                        let x = origin.x + ROW_NUM_W + c as f32 * COL_WIDTH;
                        let cell_rect = egui::Rect::from_min_size(
                            egui::pos2(x, y),
                            Vec2::new(COL_WIDTH, ROW_HEIGHT),
                        );

                        let fmt = self.grid.fmt_at(r, c);
                        let selected = self.cursor == (r, c);
                        let kind = self.grid.formula_kind(r, c);

                        let mut bg = if r % 2 == 0 {
                            Color32::from_rgb(21, 27, 36)
                        } else {
                            Color32::from_rgb(15, 20, 28)
                        };
                        if !fmt.bg_color.is_empty() {
                            bg = color_from_hex(&fmt.bg_color, bg);
                        }
                        if selected {
                            bg = Color32::from_rgb(30, 58, 95);
                        }
                        painter.rect_filled(cell_rect, 0.0, bg);
                        painter.rect_stroke(
                            cell_rect,
                            0.0,
                            egui::Stroke::new(1.0_f32, Color32::from_rgb(42, 53, 72)),
                        );
                        if selected {
                            painter.rect_stroke(
                                cell_rect.shrink(1.0),
                                0.0,
                                egui::Stroke::new(2.0_f32, Color32::from_rgb(59, 130, 246)),
                            );
                        }

                        let display = if self.editing && selected {
                            self.edit_buffer.clone()
                        } else {
                            self.grid.display_at(r, c)
                        };
                        let mut text_color = Color32::from_rgb(226, 232, 240);
                        if !fmt.text_color.is_empty() {
                            text_color = color_from_hex(&fmt.text_color, text_color);
                        } else if let Some(k) = kind {
                            text_color = match k {
                                "fx" => Color32::from_rgb(167, 139, 250),
                                "rust" => Color32::from_rgb(52, 211, 153),
                                "python" => Color32::from_rgb(251, 191, 36),
                                "sql" => Color32::from_rgb(56, 189, 248),
                                _ => text_color,
                            };
                        }
                        let size = 12.0 + fmt.font_size as f32;
                        let font = if fmt.bold {
                            FontId::new(size, egui::FontFamily::Proportional)
                        } else {
                            FontId::monospace(size)
                        };
                        let shown = if display.chars().count() > 14 {
                            format!("{}…", display.chars().take(13).collect::<String>())
                        } else {
                            display
                        };
                        painter.text(
                            egui::pos2(x + 6.0, y + ROW_HEIGHT / 2.0),
                            egui::Align2::LEFT_CENTER,
                            shown,
                            font,
                            text_color,
                        );

                        let id = ui.id().with(("cell", r, c));
                        let resp = ui.interact(cell_rect, id, Sense::click());
                        if resp.clicked() {
                            if self.editing && self.cursor != (r, c) {
                                self.commit_cell();
                            }
                            self.cursor = (r, c);
                            self.edit_buffer = self.grid.raw_at(r, c).unwrap_or_default();
                            self.formula_bar = self.edit_buffer.clone();
                            self.sync_fmt_pickers();
                            self.editing = true;
                        }
                    }
                }

                if let Some(target) = self.scroll_to_row.take() {
                    let y = (target + 1) as f32 * ROW_HEIGHT;
                    ui.scroll_to_rect(
                        egui::Rect::from_min_size(
                            egui::pos2(origin.x, origin.y + y),
                            Vec2::new(10.0, ROW_HEIGHT),
                        ),
                        Some(egui::Align::Center),
                    );
                }
            });

        if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            let abs = self.grid.row_offset + self.cursor.0 + 1;
            self.ensure_row_visible(abs);
            self.sync_fmt_pickers();
            self.update_formula_bar();
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            let abs = self.grid.row_offset + self.cursor.0;
            if abs > 0 {
                self.ensure_row_visible(abs - 1);
                self.sync_fmt_pickers();
                self.update_formula_bar();
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            if self.cursor.1 + 1 < self.grid.col_count() {
                self.cursor.1 += 1;
                self.sync_fmt_pickers();
                self.update_formula_bar();
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            if self.cursor.1 > 0 {
                self.cursor.1 -= 1;
                self.sync_fmt_pickers();
                self.update_formula_bar();
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && self.editing {
            self.commit_cell();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.editing = false;
            self.update_formula_bar();
        }
    }
}

fn rgb_hex(rgb: [f32; 3]) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        (rgb[0] * 255.0) as u8,
        (rgb[1] * 255.0) as u8,
        (rgb[2] * 255.0) as u8
    )
}

fn color_from_hex(hex: &str, fallback: Color32) -> Color32 {
    CellFmt::parse_rgb(hex)
        .map(|(r, g, b)| Color32::from_rgb(r, g, b))
        .unwrap_or(fallback)
}

fn toolbar_btn(ui: &mut egui::Ui, label: &str, mut on_click: impl FnMut()) {
    if ui
        .add(
            egui::Button::new(RichText::new(label).size(13.0))
                .fill(Color32::from_rgb(30, 39, 54)),
        )
        .clicked()
    {
        on_click();
    }
}

impl eframe::App for SqlSheetsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_theme(ctx);

        if self.play_running {
            if self.last_play.elapsed().as_millis() as u64 >= self.play_interval_ms {
                recalculate(&mut self.grid, Some(&self.db));
                self.last_play = std::time::Instant::now();
                ctx.request_repaint_after(std::time::Duration::from_millis(self.play_interval_ms));
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }


        if let Some(name) = self.pending_select.take() {
            self.selected_table = Some(name.clone());
            let _ = self.load_table(&name);
        }

        egui::TopBottomPanel::top("toolbar")
            .frame(egui::Frame::none().fill(Color32::from_rgb(21, 27, 36)).inner_margin(egui::Margin::symmetric(10.0, 6.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("SQLSheet")
                            .color(Color32::from_rgb(59, 130, 246))
                            .strong()
                            .size(14.0),
                    );
                    ui.label(
                        RichText::new("· SQLite + formulas")
                            .color(Color32::from_rgb(132, 148, 173))
                            .size(12.0),
                    );
                    ui.add_space(4.0);
                    toolbar_btn(ui, "New", || self.new_db());
                    toolbar_btn(ui, "Import", || self.open_file());
                    toolbar_btn(ui, "Export", || self.save_file());
                    ui.separator();
                    toolbar_btn(ui, "CSV ↕", || self.import_csv());
                    toolbar_btn(ui, "CSV ↑", || self.export_csv());
                    ui.separator();
                    if ui
                        .add(
                            egui::Button::new(RichText::new("+ Table").size(12.0).color(Color32::WHITE))
                                .fill(Color32::from_rgb(59, 130, 246)),
                        )
                        .clicked()
                    {
                        self.new_table();
                    }
                    // Simulation controls (visible when scripts on)
                    if scripts_enabled() && self.selected_table.is_some() {
                        ui.separator();
                        if ui.button(RichText::new("Step").size(12.0)).on_hover_text("Recalculate once (Space)").clicked() {
                            recalculate(&mut self.grid, Some(&self.db));
                            self.status = "Step · recalculated".into();
                        }
                        let play_label = if self.play_running { "■ Stop" } else { "▶ Play" };
                        let play_btn = ui.button(
                            RichText::new(play_label)
                                .size(12.0)
                                .color(if self.play_running {
                                    Color32::from_rgb(11, 15, 20)
                                } else {
                                    Color32::from_rgb(34, 197, 94)
                                }),
                        );
                        if play_btn.clicked() {
                            if self.play_running {
                                self.play_running = false;
                                self.status = "Stopped".into();
                            } else if !scripts_enabled() {
                                self.status = "Enable Scripts first".into();
                            } else {
                                self.play_running = true;
                                self.last_play = std::time::Instant::now();
                                self.status = format!("Playing · every {}ms", self.play_interval_ms);
                            }
                        }
                        egui::ComboBox::from_id_source("play_ms")
                            .selected_text(format!("{}ms", self.play_interval_ms))
                            .width(64.0)
                            .show_ui(ui, |ui| {
                                for ms in [80u64, 150, 300, 500, 1000] {
                                    ui.selectable_value(&mut self.play_interval_ms, ms, format!("{ms}ms"));
                                }
                            });
                        if ui.button(RichText::new("Bake").size(12.0)).on_hover_text("Write computed values as plain numbers").clicked() {
                            self.bake();
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(&self.status)
                                .color(Color32::from_rgb(132, 148, 173))
                                .size(11.0),
                        );
                        ui.add_space(8.0);
                        let scripts_on = scripts_enabled();
                        let badge = if scripts_on {
                            RichText::new("Scripts: on")
                                .color(Color32::from_rgb(245, 158, 11))
                                .size(11.0)
                        } else {
                            RichText::new("Scripts: off")
                                .color(Color32::from_rgb(34, 197, 94))
                                .size(11.0)
                        };
                        if ui
                            .add(egui::Button::new(badge).fill(Color32::from_rgb(30, 39, 54)))
                            .on_hover_text("Toggle script=/rust=/python= for this session (default off)")
                            .clicked()
                        {
                            if !scripts_on {
                                // confirm-like: just enable
                                set_scripts_enabled(true);
                                self.status = "Scripts enabled for this session".into();
                            } else {
                                set_scripts_enabled(false);
                                self.play_running = false;
                                self.status = "Scripts disabled".into();
                            }
                            recalculate(&mut self.grid, Some(&self.db));
                        }
                    });
                });
            });

        egui::TopBottomPanel::top("table_bar")
            .frame(egui::Frame::none().fill(Color32::from_rgb(21, 27, 36)).inner_margin(egui::Margin::symmetric(10.0, 4.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(name) = &self.selected_table {
                        ui.label(RichText::new(name).strong().size(14.0));
                    } else {
                        ui.label(RichText::new("(no table)").weak());
                    }
                    if ui.button(RichText::new("+ Row").size(12.0)).clicked() {
                        self.add_row();
                    }
                    if ui.button(RichText::new("+ Col").size(12.0)).clicked() {
                        self.add_col();
                    }
                    if ui.button(RichText::new("− Row").size(12.0).color(Color32::from_rgb(239, 68, 68))).clicked() {
                        self.del_row();
                    }
                    if ui
                        .button(RichText::new("ƒx Recalc").size(12.0))
                        .on_hover_text("Recalculate all formulas")
                        .clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Space) && !i.modifiers.any())
                    {
                        recalculate(&mut self.grid, Some(&self.db));
                        self.status = "Recalculated".into();
                    }
                    if ui.button(RichText::new("Help").size(12.0)).clicked() {
                        self.show_help = true;
                    }
                    if ui
                        .button(RichText::new("Drop").size(12.0).color(Color32::from_rgb(239, 68, 68)))
                        .clicked()
                    {
                        self.drop_table();
                    }
                    ui.separator();
                    // Format tools (original fx-bar colors)
                    ui.label(RichText::new("Format").color(Color32::from_rgb(167, 139, 250)).size(12.0));
                    ui.checkbox(&mut self.use_text_color, "Text");
                    if self.use_text_color {
                        ui.color_edit_button_rgb(&mut self.pick_text);
                    }
                    ui.checkbox(&mut self.use_bg_color, "Fill");
                    if self.use_bg_color {
                        ui.color_edit_button_rgb(&mut self.pick_bg);
                    }
                    ui.checkbox(&mut self.bold, "Bold");
                    if ui.button(RichText::new("Apply").size(11.0)).clicked() {
                        self.apply_fmt_to_cursor();
                    }
                    if ui.button(RichText::new("Clear").size(11.0)).clicked() {
                        self.clear_fmt_cursor();
                    }
                    if self.grid.total_rows > WINDOW_ROWS {
                        ui.separator();
                        if ui.button("< Prev").clicked() {
                            let off = self.grid.row_offset.saturating_sub(WINDOW_ROWS);
                            if let Some(n) = self.selected_table.clone() {
                                let _ = self.load_window(&n, off);
                            }
                        }
                        if ui.button("Next >").clicked() {
                            let off = (self.grid.row_offset + WINDOW_ROWS)
                                .min(self.grid.total_rows.saturating_sub(1));
                            if let Some(n) = self.selected_table.clone() {
                                let _ = self.load_window(&n, off);
                            }
                        }
                        ui.label(
                            RichText::new(format!(
                                "{}–{}/{}",
                                self.grid.row_offset + 1,
                                self.grid.row_offset + self.grid.row_count(),
                                self.grid.total_rows
                            ))
                            .small()
                            .weak(),
                        );
                    }
                });
            });

        egui::TopBottomPanel::bottom("sql_panel")
            .resizable(true)
            .default_height(150.0)
            .frame(egui::Frame::none().fill(Color32::from_rgb(21, 27, 36)).inner_margin(8.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("SQL Console (Ctrl+Enter to run)")
                            .strong()
                            .color(Color32::from_rgb(132, 148, 173))
                            .size(12.0),
                    );
                    if ui.button("Run").clicked()
                        || ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter))
                    {
                        self.run_sql();
                    }
                    if ui.button("Clear").clicked() {
                        self.sql_input.clear();
                        self.sql_output.clear();
                    }
                    if ui.button("Save result as table").clicked() {
                        self.save_sql_as_table();
                    }
                });
                ui.add(
                    egui::TextEdit::multiline(&mut self.sql_input)
                        .desired_width(f32::INFINITY)
                        .desired_rows(3)
                        .font(FontId::monospace(13.0))
                        .text_color(Color32::from_rgb(226, 232, 240)),
                );
                ui.separator();
                egui::ScrollArea::vertical().max_height(72.0).show(ui, |ui| {
                    ui.monospace(
                        RichText::new(&self.sql_output).color(Color32::from_rgb(148, 163, 184)),
                    );
                });
            });

        egui::SidePanel::left("tables")
            .resizable(true)
            .default_width(168.0)
            .frame(egui::Frame::none().fill(Color32::from_rgb(21, 27, 36)).inner_margin(8.0))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Tables")
                        .small()
                        .color(Color32::from_rgb(132, 148, 173))
                        .strong(),
                );
                ui.add_space(4.0);
                let names: Vec<(String, i64)> = self
                    .tables
                    .iter()
                    .map(|t| (t.name.clone(), t.row_count))
                    .collect();
                for (name, count) in names {
                    let selected = self.selected_table.as_deref() == Some(name.as_str());
                    let resp = ui.selectable_label(
                        selected,
                        RichText::new(format!("  {}  ({})", name, count)).size(13.0),
                    );
                    if resp.clicked() {
                        self.pending_select = Some(name);
                    }
                }
                if self.tables.is_empty() {
                    ui.label(RichText::new("No tables yet").weak().italics());
                }
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(11, 15, 20)).inner_margin(0.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("fx")
                            .color(Color32::from_rgb(167, 139, 250))
                            .strong()
                            .size(12.0),
                    );
                    let (r, c) = self.cursor;
                    ui.monospace(
                        RichText::new(self.grid.a1(r, c))
                            .color(Color32::from_rgb(132, 148, 173))
                            .size(13.0),
                    );
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.formula_bar)
                            .desired_width(f32::INFINITY)
                            .font(FontId::monospace(13.0))
                            .text_color(Color32::from_rgb(226, 232, 240)),
                    );
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.edit_buffer = self.formula_bar.clone();
                        self.commit_cell();
                    }
                    if response.changed() {
                        self.edit_buffer = self.formula_bar.clone();
                    }
                });
                ui.add_space(2.0);
                ui.separator();

                if self.grid.col_count() == 0 {
                    ui.vertical_centered(|ui| {
                        ui.add_space(48.0);
                        ui.heading(
                            RichText::new("Welcome to SQLSheet")
                                .color(Color32::from_rgb(226, 232, 240))
                                .size(28.0),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("SQLite + spreadsheet formulas + scripts — fully offline.")
                                .color(Color32::from_rgb(132, 148, 173))
                                .size(14.0),
                        );
                        ui.label(
                            RichText::new(
                                "=SUM(A1:A10)  ·  script=cell(\"A1\")*2  ·  sql=SELECT COUNT(*) FROM t",
                            )
                            .color(Color32::from_rgb(96, 165, 250))
                            .monospace()
                            .size(13.0),
                        );
                        ui.add_space(20.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button(RichText::new("Create table").size(13.0))
                                .clicked()
                            {
                                self.new_table();
                            }
                            if ui
                                .button(RichText::new("Import SQLite").size(13.0))
                                .clicked()
                            {
                                self.open_file();
                            }
                            if ui
                                .button(RichText::new("Sample data").size(13.0))
                                .clicked()
                            {
                                let _ = self.db.seed_demo_if_empty();
                                if self.db.list_tables().map(|t| t.is_empty()).unwrap_or(true) {
                                    // already empty and seed failed? ignore
                                }
                                // if demo already exists, still refresh
                                self.selected_table = Some("demo".into());
                                let _ = self.refresh();
                                self.status = "Sample data loaded".into();
                            }
                            if ui
                                .button(RichText::new("Formula help").size(13.0))
                                .clicked()
                            {
                                self.show_help = true;
                            }
                        });
                    });
                    return;
                }

                self.draw_virtual_grid(ui);
            });

        if self.show_help {
            egui::Window::new("Help — SQLSheet")
                .collapsible(false)
                .resizable(true)
                .default_width(520.0)
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new("Navigation").strong());
                    ui.label("Click cells to edit · Arrow keys move · Enter commits · Esc cancels");
                    ui.label("Formula bar edits the selected cell (fx)");
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("Cell types").strong());
                    ui.monospace("=SUM(A1:A10)     Excel-style formulas");
                    ui.monospace("rust=cell(\"A1\")*2   Rhai (Rust-like) scripts");
                    ui.monospace("python=cell(\"A1\")*2 Python 3 (needs python3)");
                    ui.monospace("sql=SELECT COUNT(*) FROM t   Scalar SQL");
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("Formatting").strong());
                    ui.label("Format bar: text/fill color, size, bold → Apply. Persisted in the DB.");
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("Large tables (100k+ rows)").strong());
                    ui.label(format!(
                        "Only {} rows are loaded at a time. Arrow keys and Prev/Next slide the window automatically; formulas can still reference cells outside the window via on-demand SQLite reads.",
                        sqlsheets_core::GridModel::WINDOW_SIZE
                    ));
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("Files").strong());
                    ui.label("New / Open / Save SQLite · Import / Export CSV · SQL console at the bottom");
                    ui.add_space(8.0);
                    if ui.button("Close").clicked() {
                        self.show_help = false;
                    }
                });
        }
    }
}

fn main() -> eframe::Result<()> {
    let args = Args::parse();
    let db = if let Some(path) = args.file {
        Database::open(&path).expect("open database")
    } else {
        let db = Database::new_in_memory().expect("memory db");
        let _ = db.seed_demo_if_empty();
        db
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("SQLSheet — SQLite + Spreadsheet + Formulas"),
        ..Default::default()
    };

    eframe::run_native(
        "SQLSheet",
        options,
        Box::new(|_cc| Box::new(SqlSheetsApp::new(db))),
    )
}
