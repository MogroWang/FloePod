# FloePod behavior baseline

This document captures the product contract observed at `LEGACY_BASELINE`
`bf40f6d307cd43d05c8676ab7f99e29fa4c0a9da` (`origin/main`, 2026-08-25).
The executable, package, and Tauri configuration all report version `0.6.0`.
The `0.5.1` version shown in the legacy README was stale and is not used as a
behavioral specification.

The rewrite keeps the command names, event names, persisted JSON fields, SQLite
schema, window labels, staging paths, and Windows integration described below.
Verification labels have these meanings:

- **Automated Test Verified**: an automated regression test exercises the contract.
- **Build / Static Verified**: the compiler, type checker, configuration, or a
  deterministic repository check verifies the contract.
- **Manual Windows Validation Required**: the contract depends on Explorer,
  WebView2, DWM, OLE, global input, the registry, or real monitor topology.

Some rows have more than one label because static validation protects the
contract surface while native behavior still requires a person on Windows.

## Product and user workflows

| Area | Baseline behavior | Verification |
| --- | --- | --- |
| Pod lifecycle | Create, update, enable, disable, and delete independent pods. IDs are positive, unique, and monotonically allocated; deletion never permits a reused ID to inherit old entries. | Automated Test Verified; Manual Windows Validation Required |
| Pod placement | Each enabled pod is attached to `top`, `right`, `bottom`, or `left`, uses a configured monitor (or the primary monitor), and stores a normalized `offset` from 0 through 1. | Automated Test Verified; Manual Windows Validation Required |
| Pod appearance | Per-pod opacity is 0.1 through 1.0; material is `acrylic` or `plain`; panel width is 300 through 520 logical pixels. | Automated Test Verified; Manual Windows Validation Required |
| Pod interaction | Hover opens after the per-pod delay. Click opens and pins, pins an already open panel, or closes a pinned panel. A pod can be moved along its edge and persists its final offset. | Automated Test Verified; Manual Windows Validation Required |
| Drag in | Native file drops accept files and folders. The configured action is `ask`, `copy`, `move`, or `shortcut`; Ctrl forces copy, Shift forces move, and Alt forces shortcut using the modifier state sampled while the pointer is still over the pod. | Build / Static Verified; Manual Windows Validation Required |
| Copy into staging | Files and directory trees are copied without overwriting existing names. Conflicts receive ` (2)`, ` (3)`, and so on. Symlinks, junctions, and other reparse points are rejected. | Automated Test Verified; Manual Windows Validation Required |
| Move into staging | Same-volume moves use rename. Cross-volume moves quarantine the source, build an internal staging copy, publish it atomically, commit SQLite, then remove the quarantine. Pre-commit failures attempt to restore the source. | Automated Test Verified; Manual Windows Validation Required |
| Shortcut staging | Windows `.lnk` files are created through `WScript.Shell` hosted by non-interactive PowerShell. A failed batch removes files already created by that batch. | Automated Test Verified; Manual Windows Validation Required |
| Text staging | Clipboard capture, HTML text drop, and panel input create UTF-8 `.txt` entries. A custom title is optional; otherwise the first content line is used. Invalid Windows filename characters are replaced, names are limited to 48 characters, and an existing `.txt` suffix is not duplicated. | Automated Test Verified; Manual Windows Validation Required |
| Staging list | Each panel lists only its pod's entries, newest first, with type glyphs, metadata, and bounded image thumbnails. | Automated Test Verified; Manual Windows Validation Required |
| Selection | Click, additive toggle, range selection, select all, and batch actions operate only on the active pod. Stale selected IDs are discarded after refresh. | Automated Test Verified; Manual Windows Validation Required |
| Open and reveal | A staged entry can be opened or revealed in Explorer through `tauri-plugin-opener`. | Build / Static Verified; Manual Windows Validation Required |
| Remove | Removing or clearing entries moves physical files to the Windows Recycle Bin and removes only successfully deleted or already-missing records from SQLite. Partial failures are reported. | Automated Test Verified; Manual Windows Validation Required |
| Batch export | Selected entries can be copied or moved to a chosen directory. Conflict strategies are ask, overwrite, skip, or unique rename. Results separately report conflicts, completed IDs, skipped IDs, stale IDs, failures, and warnings. | Automated Test Verified; Manual Windows Validation Required |
| Drag out copy | OLE drag-out advertises copy and leaves staged sources and database entries intact. | Build / Static Verified; Manual Windows Validation Required |
| Drag out cut | Before OLE move, the backend records a single-use, expiring snapshot of entry ownership and file or directory identity. After a successful drop it recycles only sources whose path, pod, metadata, and recursive directory fingerprint are unchanged. | Automated Test Verified; Manual Windows Validation Required |
| Browser preview | Vite browser development uses an in-memory mock for pods, entries, panel state, export, and cut tokens. Native-only actions degrade without mutating the host filesystem. | Automated Test Verified; Build / Static Verified |

## Backend invariants

### SQLite and persistent models

The database is `<data directory>/data.db`, opened through bundled SQLite with a
five-second busy timeout, WAL journaling, and foreign keys enabled.

```sql
CREATE TABLE items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  pod_id INTEGER NOT NULL DEFAULT 1,
  kind TEXT NOT NULL,
  staging_path TEXT NOT NULL UNIQUE,
  original_path TEXT,
  name TEXT NOT NULL,
  ext TEXT,
  size INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_items_pod ON items(pod_id);
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
```

`settings.key = 'app'` contains camel-case JSON with these durable fields:

- `theme`: `system`, `light`, or `dark`
- `firstRunDone`: boolean
- `autostart`: boolean
- `hotkeys.toggleBar`, `hotkeys.collectClipboard`, `hotkeys.openPanel`
- `pods[]`: `id`, `name`, `edge`, `monitor`, `offset`, `stagingFolder`,
  `opacity`, `material`, `panelWidth`, `hoverDelayMs`, `dropAction`, and `enabled`

`version` and `dataDir` are runtime metadata returned over IPC and must never be
persisted. `settings.key = 'next_pod_id'` stores the next monotonic pod ID.

Compatibility invariants:

- A 0.2/0.3 database with `scenes` and `items.scene_id` is migrated in one
  transaction. `scene_id` becomes `pod_id`, every legacy item is assigned to pod
  1, the legacy index is removed, and `scenes` is dropped.
- Legacy single-staging-folder JSON becomes pod 1 without dropping theme,
  first-run, appearance, panel, or drop-action values.
- Missing settings and pod fields use the established defaults.
- Existing pod JSON and staged rows round-trip without field renaming.
- Staging paths remain unique across all pods so a physical path cannot be
  indexed by two pods.

Status: **Automated Test Verified**.

### Filesystem safety and bookkeeping

- Every configured and runtime file path must be absolute.
- Path checks normalize `.` and `..`, resolve the nearest existing ancestor, and
  compare Windows components case-insensitively.
- A staging folder cannot be a filesystem root, the user profile or its parent,
  a protected Windows/program directory, the FloePod data directory, or equal to
  or nested with another pod's staging folder.
- Disabled legacy pods may retain an empty staging folder, but enabling or using
  one requires a safe and currently accessible root.
- Source/target equality, destination-inside-source, symlink, junction, reparse
  point, and external race checks happen before destructive operations.
- File selection, file I/O, SQLite bookkeeping, and watcher reconciliation share
  one `file_ops` critical section. The lock order is `file_ops -> db`.
- Copy targets use exclusive creation. Directory copies never merge into an
  existing directory and file copies never truncate an existing file.
- Export builds a complete same-directory temporary copy before publishing the
  final target. Overwrite first renames the old target to a recoverable backup.
- A physical operation is never reported as completed if its file was not
  produced. Partial success is represented through result issue lists or a
  command error that names the affected entries.

Status: **Automated Test Verified**; native Recycle Bin and cross-volume cases
remain **Manual Windows Validation Required**.

### Watcher and reconciliation

- Startup and staging-folder changes set a dirty flag so existing direct children
  are reconciled into SQLite.
- Every enabled pod has a non-recursive native watcher for its own staging root.
- Application writes suppress immediate rescans for three seconds without
  discarding genuine external changes; delayed work re-sets the dirty flag.
- Failed watcher installation is retried every ten seconds. Unavailable folders
  are retried without treating access errors as deletions.
- Reconciliation scans a complete directory snapshot before deleting stale rows.
  A partial or unreadable snapshot cannot prove that an indexed file disappeared.
- Internal `.floepod-inflight-*` and `.floepod-move-source-*` paths, symlinks,
  junctions, and reparse points are never exposed as staged entries.
- Existing `text` rows remain text when a `.txt` file is observed as a normal
  file. External add, delete, rename metadata, kind, size, and spelling changes
  are reconciled per pod.
- Item change events are sent only to `pod_<id>` and `pod_<id>_panel`.

Status: **Automated Test Verified** for snapshot/database reconciliation and
**Manual Windows Validation Required** for `notify` recovery and Explorer races.

## IPC contract

These Tauri command names are stable:

| Domain | Commands | Verification |
| --- | --- | --- |
| Bootstrap | `get_bootstrap`, `get_pod`, `get_monitors`, `get_modifier_state`, `get_hotkey_defaults` | Automated Test Verified; Build / Static Verified |
| Pods/settings | `create_pod`, `update_pod`, `delete_pod`, `save_settings` | Automated Test Verified; Build / Static Verified |
| Staging | `hold_pending_drop`, `stage_paths`, `stage_text`, `list_pod_items`, `remove_items` | Automated Test Verified; Build / Static Verified |
| Export/drag | `export_items`, `prepare_drag_cut`, `finalize_drag_cut`, `cancel_drag_cut`, `read_thumbnail` | Automated Test Verified; Build / Static Verified |
| Panel/window | `show_panel`, `toggle_panel`, `hide_panel`, `get_panel_state`, `set_panel_mode`, `report_presence`, `set_panel_pinned`, `set_dragging_out`, `set_pod_accept`, `set_panel_size`, `move_pod_bar`, `toggle_all_bars`, `open_settings` | Automated Test Verified; Build / Static Verified |
| Lifecycle/logging | `log_frontend`, `app_log`, `quit_app` | Build / Static Verified |
| Drag plugin | `plugin:drag|start_drag` with an OLE mode of copy or move and a completion channel | Build / Static Verified; Manual Windows Validation Required |

Rust uses serde camel-case models, matching the TypeScript domain types. Unknown
settings or pod patch fields fail instead of being silently accepted. Numeric
strings remain accepted for legacy range-control callers.

## Event and window routing contract

| Event | Scope and payload | Verification |
| --- | --- | --- |
| `floepod://items-changed` | Directed to the matching bar and panel; `{ podId }` | Automated Test Verified; Manual Windows Validation Required |
| `floepod://settings-changed` | Global full `Settings` snapshot | Build / Static Verified |
| `floepod://pods-changed` | Global signal to refresh bootstrap | Build / Static Verified |
| `floepod://panel-mode` | Directed to matching panel; `{ mode, paths }` | Automated Test Verified; Manual Windows Validation Required |
| `floepod://panel-shown` | Directed to matching panel; animation signal | Automated Test Verified; Manual Windows Validation Required |
| `floepod://panel-pinned` | Directed to matching panel; `{ pinned }` | Automated Test Verified; Manual Windows Validation Required |
| `floepod://panel-state` | Directed complete `PanelState` snapshot | Automated Test Verified; Manual Windows Validation Required |
| `floepod://panel-hidden` | Directed to matching panel after a real hidden-state transition | Automated Test Verified; Manual Windows Validation Required |
| `floepod://collect-clipboard` | Directed to the first enabled pod bar; `{ podId }` | Automated Test Verified; Manual Windows Validation Required |

Window labels are part of the contract:

- `settings`: the single configured settings/OOBE window.
- `pod_<positive id>`: one edge bar per enabled pod.
- `pod_<positive id>_panel`: the matching staging panel.

All Vue windows share `index.html`; the label is resolved synchronously before
the first mount. Unknown labels render an error view and do not mount a pod.
The settings window intercepts close and hides. Pod panel close requests are
converted to the matching pod's hide transition.

Panel invariants:

- Runtime state is isolated by pod ID.
- Only one dismissible, unpinned list panel remains active. Pinned panels, OLE
  drag-out, pending drop questions, and conflict dialogs are protected.
- State changes and native window side effects are serialized by `panel_ops` so
  late show/hide work cannot split logical and native visibility.
- `SW_SHOWNOACTIVATE` displays bars and panels without taking foreground focus;
  the WebViews remain focusable so native drag/drop and intentional interaction
  work.
- The watchdog hides an unpinned list panel only after the pointer left both bar
  and panel for 320 ms.
- Hiding all pods is a reversible pause: native windows hide, while pinned,
  pending-drop, and conflict context remains intact.
- A panel WebView registers directed listeners before fetching `get_panel_state`;
  revision counters prevent an older snapshot from overwriting a newer event.

Status: state and geometry are **Automated Test Verified**; native visibility,
focus, anti-flicker, and multi-window behavior are **Manual Windows Validation Required**.

## Native Windows behavior

- `GetAsyncKeyState` reads Ctrl, Shift, and Alt during native drag in.
- `ShowWindow(SW_SHOWNOACTIVATE)` plus topmost `SetWindowPos` shows UI without
  activating it. Direct `ShowWindow(SW_HIDE)` avoids the WebView2/Tauri hide quirk
  that can leave a visible top-level placeholder.
- DWM rounding is disabled for the custom pod silhouette and requested for panel
  shadow/acrylic alignment. Unsupported Windows versions may ignore the hint.
- `tauri-plugin-drag` provides Explorer-compatible OLE drag out.
- `.lnk` files are created through Windows Script Host COM.
- `trash` integrates delete and successful move-source cleanup with the Recycle Bin.
- Per-executable autostart values live under HKCU Run. The executable is always
  quoted, installed and portable instances use distinct deterministic names,
  and a legacy shared `FloePod` value is removed only when it targets the same
  executable.
- The tray opens settings, opens a selected pod, captures clipboard text into the
  first enabled pod, toggles all pods, and quits.
- Global hotkeys are validated for syntax and duplicates before the active set is
  removed. A partial new registration is cleaned up; save rollback restores the
  old registrations and settings.
- The single-instance plugin brings the existing settings window forward.

Status: parsers and state decisions are **Automated Test Verified**; integration
with Win32, Explorer, DWM, OLE, registry, tray, and global shortcuts is
**Manual Windows Validation Required**.

## Theme and first frame

- Modes are light, dark, and follow system.
- A synchronous head script applies the media-query theme before Vue or the main
  stylesheet loads. The app stays transparent until the theme store is ready,
  with a 1.5-second safety fallback.
- The store updates DOM classes, `color-scheme`, the native Tauri theme, WebView2
  media queries, and native theme-change events.
- Non-system modes ignore WebView2 media-query changes that can be caused by
  forcing the native theme.

Status: theme transforms and startup markup are **Automated Test Verified** and
**Build / Static Verified**; DWM/WebView live behavior is
**Manual Windows Validation Required**.

## Portable and installed data paths

- Portable mode is explicitly requested by `.floepod-portable` beside the
  executable. An existing sibling `FloePodData` directory also remains portable
  for upgrade compatibility.
- Portable data uses `<exe directory>/FloePodData` only when it passes a real
  exclusive write probe.
- Installed mode uses absolute `%APPDATA%\FloePod`, then absolute
  `%LOCALAPPDATA%\FloePod`, and finally an absolute temporary-directory fallback.
- Installed builds are not considered portable merely because their program
  directory is writable.
- The portable package contains exactly `FloePod.exe`, `.floepod-portable`, and
  `使用说明.txt`; it is published only after compression succeeds.

Status: path selection and marker behavior are **Automated Test Verified**;
installed/portable upgrades and removable media are **Manual Windows Validation Required**.

## Packaging and CI

- Toolchain: pnpm 11.22.0, Node 24.19.0, Rust 1.97.1
  `x86_64-pc-windows-msvc`, Vite 7, Tauri 2, NSIS, and bundled SQLite.
- CI audits JavaScript and Rust dependencies. Windows CI installs the frozen
  lockfile, builds the frontend, checks Rust formatting, runs Rust tests and
  Clippy with warnings denied, builds the Tauri NSIS target, builds the portable
  zip, and uploads checksummed artifacts.
- `v*` tags additionally verify that the tag, package, Cargo, and Tauri versions
  match, attest build provenance, and publish the Windows release assets.
- Release profile keeps size optimization, LTO, one codegen unit, symbol
  stripping, and abort-on-panic.

Status: manifests and workflows are **Build / Static Verified**. Local commands
are recorded in the PR; installer and portable launch remain
**Manual Windows Validation Required** even when packaging succeeds.

## Intentional compatibility quirks

- README content is informative only; runtime manifests and code define the
  version and behavior.
- A missing legacy pod ID deserializes as zero so create requests can omit the
  backend-owned ID. Persisted settings still reject zero and duplicate IDs.
- An offline removable staging drive may remain configured. It blocks I/O for
  that pod but does not block unrelated pods or cause watcher deletion.
- A disabled legacy pod may retain an empty folder until it is enabled.
- `text` item kind survives ordinary `.txt` observation during reconciliation.
- Global hide preserves modal and pin state and does not emit the semantic
  `panel-hidden` transition.
- OOBE pod creation can be retried idempotently by staging-folder identity when
  the first response is lost after the database commit.
- Delete and export are intentionally partial-success operations; already
  completed physical effects are not rolled back by retrying unrelated failures.

