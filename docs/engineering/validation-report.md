# Engineering validation report

Baseline: `bf40f6d307cd43d05c8676ab7f99e29fa4c0a9da`

Branch: `refactor/new-baseline`

Platform: Windows x64

Validation date: 2026-08-25

| Check | Result | Detail |
| --- | --- | --- |
| `pnpm install --frozen-lockfile` | PASS | pnpm 11.22.0; lockfile unchanged |
| `pnpm test` | PASS | 7 frontend domain and IPC contract tests |
| `pnpm build` | PASS | `vue-tsc --noEmit` and Vite production build |
| `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` | PASS | No formatting diff |
| `cargo check --manifest-path src-tauri/Cargo.toml --locked` | PASS | Windows x64 development profile |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked` | PASS | 52 Rust tests; no failures or ignored tests |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings` | PASS | No Clippy warnings |
| `pnpm audit --audit-level high --registry https://registry.npmjs.org` | PASS | No known vulnerabilities |
| `cargo audit --file src-tauri/Cargo.lock` | PASS WITH WARNINGS | No vulnerability failure; RustSec reported 17 allowed transitive maintenance/unsoundness warnings, primarily cross-platform GTK3 and UNIC dependencies |
| `pnpm tauri build --ci` | PASS | Built release executable and `FloePod_0.6.0_x64-setup.exe` |
| `node scripts/package-portable.mjs` | PASS | Built `FloePod-0.6.0-win-x64-portable.zip` |
| Portable archive contents | PASS | Exactly `FloePod.exe`, `.floepod-portable`, and `使用说明.txt` under the `FloePod/` root |

The generated executable, installer, portable zip, `dist/`, `node_modules/`, and
Cargo `target/` directory are ignored build artifacts and are not part of the
Git changes.

Native interaction scenarios that cannot be established by a build alone remain
listed in [manual-windows-validation.md](manual-windows-validation.md). In
particular, a person must validate focus behavior, real Explorer OLE drag/drop,
Recycle Bin recovery, registry autostart, global hotkeys, live DWM/WebView theme
behavior, removable drives, and mixed-DPI monitor topology before release.
