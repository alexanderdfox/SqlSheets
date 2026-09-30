# SqlSheets-rs

**SQL is Excel** — A Rust port of [SqlSheets](https://alexanderdfox.github.io/SqlSheets/) as both a **TUI** and **GUI**.

Real SQLite database · spreadsheet grid · Excel-style formulas · `sql=` cell expressions · offline / local-first.

Original project: [alexanderdfox/SqlSheets](https://github.com/alexanderdfox/SqlSheets) (BSD-3-Clause).

## Features

| Feature | Status |
|--------|--------|
| SQLite (rusqlite, bundled) | ✅ |
| Spreadsheet grid over any table | ✅ |
| Excel formulas (`=SUM(A1:A10)`, `=A1*B2`, …) | ✅ |
| Functions: SUM, AVERAGE, MIN, MAX, COUNT, IF, AND, OR, NOT, ABS, ROUND, LEN, UPPER, LOWER, CONCAT | ✅ |
| `sql=SELECT …` expressions in cells | ✅ |
| `rust=` Rhai (Rust-like) cells | ✅ `rust=cell("A1") * 2` |
| `python=` Python 3 cells | ✅ requires `python3` on PATH |
| Import / export SQLite | ✅ |
| Import / export CSV | ✅ |
| SQL console | ✅ |
| Multi-table sidebar | ✅ |
| Recalculate formulas | ✅ |
| Cell text / fill color + bold + font size | ✅ (persisted in `_sqlsheets_fmt`) |
| Virtualized / windowed large grids | ✅ auto-sliding window (~512 rows), 100k+ supported |
| TUI (ratatui) | ✅ `sqlsheets` |
| GUI (egui) | ✅ `sqlsheets-gui` |

## Build

```bash
# Needs a C toolchain (for bundled SQLite) and Rust 1.75+
cargo build --release -p sqlsheets-tui
cargo build --release -p sqlsheets-gui
```

Binaries:

- `target/release/sqlsheets` — terminal UI  
- `target/release/sqlsheets-gui` — native window UI  

## Usage

### TUI

```bash
sqlsheets                    # empty in-memory DB + demo table
sqlsheets --demo            # same
sqlsheets mydata.sqlite     # open file

# Keys
Tab          cycle focus: tables → grid → SQL
↑↓←→ / hjkl  move cursor
Enter / e    edit cell
r            recalculate formulas
n            new table
o            open SQLite file
s            save SQLite
i            import CSV
x            export CSV
F5 / Ctrl+Enter  run SQL
q            quit
```

### GUI

```bash
sqlsheets-gui
sqlsheets-gui path/to/file.sqlite
```

Toolbar: New · Open · Save · Import CSV · Export CSV · + Table · Recalc  
Click cells to edit; formula bar shows the raw value (`=…` or `sql=…`).  
SQL console at the bottom (Run or Ctrl+Enter).

## Scripts (`rust=` / `python=`)

Replace the original browser `rust=` JavaScript with:

| Prefix | Engine | Example |
|--------|--------|---------|
| `rust=` | **Rhai** (Rust-like, pure Rust, sandboxed) | `rust=cell("A1") * 2` |
| `python=` | **Python 3** (system `python3`) | `python=cell("A1") * 2` |

Helpers in both engines:
- `cell("A1")` — read another cell’s evaluated value

Rhai examples:
```
rust=cell("B1") * cell("C1")
rust=let x = cell("A1"); if x > 0 { x * 2 } else { 0 }
```

Python examples:
```
python=cell("B1") * cell("C2")
python=sum([cell("A1"), cell("A2"), cell("A3")])
```

Toggle in TUI with **`t`**. Disable after opening untrusted files.

## Formula examples

```
=A1+B1
=SUM(A1:A10)
=AVERAGE(B2:B20)
=IF(A1>0, "yes", "no")
=B1*C1
sql=SELECT COUNT(*) FROM demo
sql=SELECT SUM(qty*price) FROM demo WHERE name='Widget'
```

## Architecture

```
sqlsheets-rs/
├── crates/core/     # Database, GridModel, formula engine
├── crates/tui/      # ratatui + crossterm binary `sqlsheets`
└── crates/gui/      # egui + eframe binary `sqlsheets-gui`
```

The core keeps a snapshot of table rows as text (including formula source).  
`recalculate()` walks the grid with a dependency-aware evaluator (circular refs → `#CIRC!`).

## Parity with original SqlSheets

Layout and workflow follow [alexanderdfox.github.io/SqlSheets](https://alexanderdfox.github.io/SqlSheets/):

| Original | Rust port |
|----------|-----------|
| New / Import / Export | Same (GUI labels; TUI `n`/`o`/`s`) |
| CSV ↕ / CSV ↑ | Import / Export CSV |
| + Table, + Row, + Col | Same |
| ƒx Recalc / Space | Same (Space = Step) |
| Scripts: off (default) | Same — `t` or badge to enable |
| Step / ▶ Play / Bake | Same when scripts on |
| `script=` cells | Rhai (`script=` or `rust=`); `python=` also |
| `=SUM(...)` / `sql=` | Same formula + SQL cells |
| fx bar + cell colors | Format bar / `c` in TUI |
| SQL Console + Save result as table | Same |
| Game of Life demos | Use Bake + Play with `script=` cells |

## Large tables (100k+ rows)

Only a **sliding window** of ~512 rows is held in memory. Navigating with arrows / Page Up·Down / Prev·Next recenters the window. Formula and `rust=` / `python=` cell references outside the window are resolved with on-demand SQLite reads. Ranges larger than 50k cells return `#RANGE!`.

Press **`?` / F1** (TUI) or **Help** (GUI) for the full keymap.

## Limitations vs browser original

- No JavaScript `rust=` cells  
- Formula function set is smaller than Excel  
- No Game-of-Life step/play mode  
- No cell color formatting persistence UI (meta table supported in core)

## License

BSD 3-Clause — same spirit as the original SqlSheets project.
