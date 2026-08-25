# Target architecture

The rewrite keeps FloePod's existing technology and compatibility surfaces while
moving behavior out of oversized transport and view files.

## Rust backend

The backend uses a small set of concrete domain modules rather than a generic
service hierarchy:

- `commands`: thin Tauri command boundary and blocking-task handoff
- `pods`: pod/settings mutations and their transactional runtime application
- `staging`: staged-entry ownership, copy/move/text ingestion, and removal
- `file_ops`: safe path copying, unique targets, atomic publish, and rollback
- `export`: per-entry copy/move results and conflict handling
- `drag_out`: expiring cut snapshots and post-OLE identity verification
- `thumbnail`: validated and bounded image decoding
- `db` and `settings`: SQLite schema, migrations, durable models, and validation
- `watcher`: watcher lifecycle plus a separately testable reconciliation core
- `manager`: window geometry and serialized panel state transitions
- `events`: event names, pod window labels, and directed emit helpers
- `autostart`, `hotkeys`, `tray`, `win`, and `lnk`: explicit Windows boundaries

Tauri command names and serde payloads stay stable. Plain synchronous functions
own filesystem and persistence behavior so tests do not need a Tauri runtime.

## Vue frontend

- `domain`: IPC models and pure state transforms
- `ipc`: command constants, real Tauri transport, browser mock, and directed
  event subscriptions
- `stores`: Pinia snapshots and selection ownership
- `windows`: one entry view for each window kind
- `components`: presentational controls and domain-specific UI pieces
- `lib`: small implementation utilities such as animation and formatting

The browser mock remains a first-class development path but is isolated from the
real transport. Tauri imports are consistent, command/event strings have one
source, and pure window-label, selection, theme, and result transforms are tested
with Node's built-in test runner.

## Dependency and lock discipline

The rewrite does not migrate frameworks, runtimes, persistence libraries, or
packaging. It adds no production dependency and does not change the current major
version families. New tests prefer the existing Node and Cargo toolchains.

