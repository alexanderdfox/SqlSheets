You are an expert full-stack developer specializing in modern web apps, SQLite, and spreadsheet-like interfaces. 

I want you to significantly improve the open-source project SqlSheets (https://github.com/alexanderdfox/SqlSheets) so it becomes a competitive local-first “real SQLite database that feels like Excel”.

Current state:
- Browser-only app using sql.js
- Basic spreadsheet grid over SQLite tables
- Supports Excel-style formulas, `script=` JavaScript, and `sql=` expressions in cells
- Import/export SQLite + CSV
- Simple SQL console
- Completely offline, no accounts

Goal: Turn it into a polished, high-performance, extensible local-first tool while keeping the core free and fully offline.

Please implement the following improvements in priority order. Work incrementally, commit cleanly, and keep the existing architecture as much as possible unless a clear better approach is needed.

### Phase 1 – Foundation & Polish (do this first)
1. Modern, performant grid
   - Virtualized rendering (react-window / tanstack-virtual or equivalent)
   - Smooth scrolling with thousands of rows
   - Better cell editing experience, keyboard navigation, multi-cell selection, copy/paste
2. Robust formula engine
   - Proper dependency graph and automatic recalculation
   - Expanded Excel-compatible functions
   - Named ranges
   - Clear error handling and circular reference detection
3. Desktop-ready packaging
   - Convert to a proper PWA with offline support
   - Add Tauri or Electron wrapper so it can run as a native desktop app with proper file system access
4. Better multi-table UX
   - Visual table list + relationship diagram
   - Foreign-key aware editing
   - Easy way to create and manage indexes and constraints from the UI

### Phase 2 – Performance & Persistence
- Switch persistence to Origin Private File System (OPFS) or IndexedDB for larger databases
- Optional DuckDB-WASM mode for heavy analytics while keeping SQLite as the source of truth
- Pagination / progressive loading for large tables
- Background vacuum and optimization

### Phase 3 – Collaboration & Sharing (keep offline-first)
- Local-first sync using CRDTs or a simple conflict-free merge strategy
- Shareable read-only or edit links (optional light cloud layer)
- Version history / snapshots of the .sqlite file
- Real-time multi-user editing when online (optional)

### Phase 4 – Power Features
- Plugin / extension system (users can add custom JS functions and UI components)
- Rich charts and pivot-style views
- Forms and simple “app” views on top of tables
- Optional AI assistant (local or API) for generating SQL, formulas, and cleaning data
- Better import/export: .xlsx, Parquet, and optional read-only connectors to Postgres/MySQL

### Phase 5 – Ecosystem
- High-quality documentation and template gallery
- Clear contribution guidelines and roadmap
- Automated tests and stable release process

Technical constraints & preferences:
- Prefer TypeScript
- Keep the core completely free and offline-capable
- Prefer modern, lightweight libraries
- Maintain backward compatibility with existing .sqlite files
- Code should be clean, well-commented, and easy for other contributors

Start by analyzing the current codebase thoroughly, then propose a concrete implementation plan for Phase 1. After I approve the plan, begin implementing it step by step.
