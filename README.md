# SQLSheet

**Secure, offline SQLite that works like a spreadsheet** — built for privacy-sensitive and public-sector environments.

**Live demo:** [alexanderdfox.github.io/SqlSheets](https://alexanderdfox.github.io/SqlSheets/)

---

## Why government and regulated organizations use it

| Requirement | How SQLSheet addresses it |
|-------------|---------------------------|
| **Data sovereignty / no cloud dependency** | 100% client-side. No server, no account, no backend. Data never leaves the browser. |
| **Zero telemetry** | No analytics, no tracking pixels, no phone-home. |
| **Air-gap friendly** | After first load (or self-host), works fully offline. |
| **Auditability** | Transparent open-source (BSD-3). CSP + Subresource Integrity on sql.js. Scripts **disabled by default**. |
| **Familiar interface** | Spreadsheet grid + real SQLite (joins, indexes, transactions, full SQL). |
| **Export control** | Export complete `.sqlite` or CSV; no proprietary lock-in. |
| **Low IT burden** | Single static HTML page (or self-hosted). No installation, no agents. |

SQLSheet is **not** a multi-user enterprise database and does **not** claim FedRAMP authorization. It is a local-first workbench intended for analysis, FOIA preparation, inventory, budgeting, field data collection, and similar tasks where data must remain under the user’s control.

---

## Purpose

SQLSheet bridges relational databases and spreadsheets:

- Full **SQLite** power (tables, joins, indexes, transactions, full SQL)
- Familiar **spreadsheet grid** (cells, ranges, point-and-click editing)
- **Excel-style formulas** (`=SUM(A1:A10)`)
- Optional **JavaScript** (`script=…`) and **SQL** (`sql=…`) expressions in cells — scripts are **off by default** and must be explicitly enabled per session

Everything runs in the browser. No data leaves the machine.

---

## Core Features

- SQLite engine in the browser (sql.js, integrity-checked)
- Spreadsheet grid view of any table
- Cell formulas:
  - Excel-style: `=SUM(A1:A10)`, `=A1*B2`, …
  - JavaScript: `script=cell('A1')*2` (requires Scripts: on)
  - SQL: `sql=SELECT COUNT(*) FROM employees`
- Import / Export:
  - Full SQLite databases (`.sqlite` / `.db`)
  - CSV (import as new table, export current table)
- Create, drop, and manage multiple tables
- SQL Console for arbitrary queries
- Recalculate formulas on demand
- Works entirely offline once loaded
- Content-Security-Policy enforced; scripts sandboxed and default-deny

---

## Public-sector & regulated use cases

1. **Offline analysis & FOIA / records preparation**  
   Import extracts, clean and query locally, export only what is approved for release.

2. **Inventory, property, and asset tracking**  
   Maintain structured tables with real relational integrity while using a familiar grid.

3. **Budget and fiscal workbooks**  
   Combine spreadsheet formulas with SQL aggregations without sending data to a cloud service.

4. **Field / air-gapped environments**  
   Load once (or host internally), then operate with no network.

5. **Training and data literacy**  
   Teach SQL and spreadsheet concepts in a single, self-contained environment.

6. **Rapid prototyping of data models**  
   Design tables and queries before migrating to an authorized enterprise system.

7. **Data cleaning pipelines**  
   Import messy CSVs, transform with SQL, export clean SQLite or CSV for downstream systems.

---

## Security model (summary)

- **No server-side processing** — all computation is local.
- **Scripts off by default** — `script=` cells do not execute until the user enables Scripts for the session.
- **Content-Security-Policy** restricts unexpected remote code.
- **sql.js** is loaded with Subresource Integrity.
- **No accounts, no authentication tokens, no persistent remote storage.**
- Suitable for self-hosting on internal static web servers or air-gapped media.

Organizations with formal ATO / authorization requirements should evaluate the tool against their own control baselines (NIST SP 800-53, agency-specific policies, etc.). This project provides transparency and technical controls; formal authorization remains the responsibility of the deploying organization.

---

## Accessibility notes

- Semantic structure and keyboard-operable controls.
- High-contrast dark theme by default; system font stack for readability.
- Scripts and optional demos are clearly labeled and non-essential for core data work.
- Further WCAG 2.2 / Section 508 conformance testing is recommended for formal procurement.

---

## Quick Start

1. Open the live demo or host `index.html` (and any required assets) on your own infrastructure.
2. Click **New** or **Import SQLite** / **Import CSV**.
3. Create or select a table.
4. Edit cells normally, or enter:
   - `=SUM(A1:A10)` for spreadsheet formulas
   - `script=cell('A1') * 2` for JavaScript (enable Scripts first)
   - `sql=SELECT COUNT(*) FROM my_table` for SQL
5. Use the **SQL Console** (Ctrl+Enter) for full queries.
6. Export as SQLite or CSV when finished.

---

## Self-hosting (recommended for government networks)

```bash
# Clone or download the repository
git clone https://github.com/alexanderdfox/SqlSheets.git
cd SqlSheets

# Serve with any static file server, or place on an internal web server
# Example (Python):
python3 -m http.server 8080
```

For air-gapped use: download the page and sql.js once on a connected machine, then transfer via approved media.

---

## License

BSD 3-Clause License  
Copyright (c) 2026, Alex Fox

---

## Disclaimer

SQLSheet is provided as open-source software. It is not certified, accredited, or authorized under FedRAMP, FISMA, CMMC, or any other formal government authorization framework. Deploying organizations are responsible for their own risk assessment, configuration management, and compliance documentation.
