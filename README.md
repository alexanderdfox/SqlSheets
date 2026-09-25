# SqlSheets

**SQL IS Excel** — A browser-based tool that combines a full SQLite database with a spreadsheet interface and Excel-style formulas.

Live demo: [alexanderdfox.github.io/SqlSheets](https://alexanderdfox.github.io/SqlSheets/)

---

## Purpose

SqlSheets bridges the gap between relational databases and spreadsheets. It gives you:

- The power of **SQLite** (real tables, joins, indexes, transactions, full SQL)
- The familiarity of a **spreadsheet grid** (cells, ranges, point-and-click editing)
- **Formulas** that feel like Excel (`=SUM(A1:A10)`)
- **JavaScript** expressions (`script=...`) and **SQL** expressions (`sql=...`) that can live directly inside cells

Everything runs client-side in the browser. No server, no account, no data leaving your machine.

Ideal for people who want database power without leaving a spreadsheet-like workflow, or spreadsheet users who need proper relational data modeling.

---

## Core Features

- **SQLite engine** in the browser (via sql.js)
- Spreadsheet grid view of any table
- Cell formulas:
  - Excel-style: `=SUM(A1:A10)`, `=A1*B2`, etc.
  - JavaScript: `script=cell('A1')*2`
  - SQL: `sql=SELECT COUNT(*) FROM employees`
- Import / Export:
  - Full SQLite databases (`.sqlite` / `.db`)
  - CSV (import as new table, export current table)
- Create, drop, and manage multiple tables
- SQL Console for arbitrary queries (`SELECT`, `INSERT`, `UPDATE`, `CREATE`, etc.)
- Recalculate all formulas on demand
- Works entirely offline once loaded

---

## Use Cases

### 1. Lightweight local database with a friendly UI
Keep small-to-medium structured datasets (contacts, inventory, projects, budgets) in a real SQLite file while editing them in a familiar grid. Export the `.sqlite` file and use it elsewhere.

### 2. Ad-hoc analysis & reporting
Import a CSV or SQLite dump, run SQL queries in the console, save result sets as new tables, and use spreadsheet formulas for final calculations and formatting.

### 3. Prototyping data models
Design tables, relationships, and sample data quickly. Test queries and derived columns (via `sql=` or `script=` cells) before moving to a production database.

### 4. Teaching / learning SQL + spreadsheets
Demonstrate the relationship between tables and sheets, show how `GROUP BY` relates to pivot tables, and let students mix SQL and formulas in the same environment.

### 5. Offline personal data tools
Build small personal apps (expense trackers, reading lists, workout logs, etc.) that live as a single `.sqlite` file and open instantly in the browser.

### 6. Data cleaning & transformation
Import messy CSVs, clean them with SQL (`UPDATE`, `DELETE`, calculated columns), then export clean CSVs or a proper SQLite database.

### 7. Hybrid formula + query workflows
Put a SQL aggregate or lookup directly in a cell (`sql=SELECT SUM(amount) FROM expenses WHERE month = '2026-09'`) while using classic spreadsheet formulas for surrounding calculations.

### 8. Sharing reproducible datasets
Send someone a single `.sqlite` file + the SqlSheets URL. They open it in the browser and immediately have both the data and a powerful query/formula environment.

---

## Quick Start

1. Open [alexanderdfox.github.io/SqlSheets](https://alexanderdfox.github.io/SqlSheets/)
2. Click **New** or **Import SQLite** / **Import CSV**
3. Create or select a table
4. Edit cells normally, or enter:
   - `=SUM(A1:A10)` for spreadsheet formulas
   - `script=cell('A1') * 2` for JavaScript
   - `sql=SELECT COUNT(*) FROM my_table` for SQL
5. Use the **SQL Console** (Ctrl+Enter) for full queries
6. Export your work as SQLite or CSV when finished

---

## License

BSD 3-Clause License  
Copyright (c) 2026, Alex Fox
