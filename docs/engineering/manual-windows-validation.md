# Manual Windows validation

Use a disposable test account or backed-up data for upgrade and destructive-file
cases. Record the Windows version, WebView2 version, monitor topology, build SHA,
and whether the executable is installed or portable.

## Install and upgrade

- [ ] Fresh NSIS install launches and opens OOBE.
- [ ] Fresh portable zip launches and creates sibling `FloePodData`.
- [ ] Installed build uses `%APPDATA%\FloePod`, not the installation directory.
- [ ] Existing installed `data.db`, settings, pods, and staging directories load unchanged.
- [ ] Existing portable `FloePodData` works with and without the new marker.
- [ ] A portable directory that is not writable falls back safely to AppData.
- [ ] A legacy 0.2/0.3 scene database upgrades once and retains every staged row under pod 1.
- [ ] An offline removable staging drive does not erase entries or block other pods.

## Theme and native appearance

- [ ] First frame has no white flash in light, dark, and system modes.
- [ ] Light, dark, and system modes render correctly in all three window types.
- [ ] Live system-theme changes update native windows and WebViews.
- [ ] Forced light/dark modes ignore unrelated media-query changes.
- [ ] Acrylic and plain materials update without repeated flicker.
- [ ] Pod bars are not clipped by DWM rounding.
- [ ] Panels have aligned CSS content, native rounding, shadow, and acrylic edges.

## Pods and monitors

- [ ] Create at least four pods with independent staging directories.
- [ ] Enable and disable pods without changing another pod's state or files.
- [ ] Place pods on top, bottom, left, and right edges.
- [ ] Test primary, secondary, mixed-DPI, and negative-origin monitor layouts.
- [ ] Move a pod from offsets 0 through 1; confirm the final value persists after restart.
- [ ] Change monitor, edge, range position, opacity, material, width, and hover delay.
- [ ] Delete a pod while keeping files, then delete another pod with Recycle Bin cleanup.

## Panel lifecycle and event isolation

- [ ] Hover opens after the configured delay without stealing focus.
- [ ] Click opens/pins, click again closes, and the pin control stays synchronized.
- [ ] Only one ordinary unpinned panel remains active.
- [ ] Pinned, drag-out, pending-drop, and conflict panels are not dismissed by another pod.
- [ ] Leaving bar and panel hides after the watchdog grace period without flicker.
- [ ] Show/hide all pods preserves pins, pending drops, conflicts, and logical visibility.
- [ ] Alt+F4 on a panel hides only its matching panel.
- [ ] Repeatedly switch among multiple pods and verify no item, mode, pin, or animation crosstalk.
- [ ] Resize content-heavy panels and verify no growth loop, clipping, or cross-monitor scale error.

## Drag in and text

- [ ] Drop a file, image, and nested folder with the configured default action.
- [ ] Ctrl forces copy, Shift forces move, and Alt forces shortcut even when released at drop.
- [ ] Ask mode can copy, move, create a shortcut, remember a choice, or cancel.
- [ ] Same names receive deterministic ` (2)`, ` (3)` suffixes.
- [ ] Symlinks, junctions, reparse points, roots, and source-inside-target cases are rejected safely.
- [ ] Cross-volume move succeeds and leaves no internal quarantine/inflight files.
- [ ] Simulate a denied copy/move and verify sources remain recoverable and no success is reported.
- [ ] Capture clipboard text using the global hotkey and tray.
- [ ] Stage text from panel input with a custom title, blank title, `.txt` title, long title, and invalid characters.

## Staging and export

- [ ] Image thumbnails load, remain bounded, and do not block stage/export actions.
- [ ] Single, additive, range, and select-all selection stay inside the active pod.
- [ ] Open and reveal work in Explorer.
- [ ] Single, batch, and clear removal use Recycle Bin and report partial failures accurately.
- [ ] Copy To and Move To work for files and directory trees.
- [ ] Ask, overwrite, skip, and rename conflicts produce the documented result and selection state.
- [ ] Overwrite failure restores or preserves the old target and reports any retained backup.
- [ ] Permission errors and missing sources distinguish completed, stale, skipped, failed, and warning entries.

## OLE drag out

- [ ] Copy drag works with Explorer and at least one other OLE drop target.
- [ ] Cut/move drag deletes staged sources only after the target reports a successful drop.
- [ ] Cancelled or rejected drag leaves sources and database entries unchanged.
- [ ] Modify a file and a nested directory child during drag; cleanup must refuse deletion.
- [ ] Replace a source with a link/reparse point during drag; cleanup must refuse deletion.
- [ ] Partial source-cleanup failure is visible and does not invite an unsafe duplicate retry.

## Watcher and reconciliation

- [ ] Startup indexes existing direct children once per pod.
- [ ] Explorer add, rename, metadata change, and delete update only the matching pod.
- [ ] External changes during application writes are eventually reconciled.
- [ ] Restart after application crash with leftover internal operation paths; they are not indexed.
- [ ] Disconnect and reconnect a removable drive; watcher installation and reconciliation recover.
- [ ] An unreadable directory snapshot does not delete database rows.

## System integration

- [ ] Tray left-click opens settings; every tray pod item opens the correct panel.
- [ ] Tray clipboard, show/hide, and quit actions work.
- [ ] Default and custom global shortcuts work and duplicate/conflicting shortcuts roll back.
- [ ] Installed and portable autostart values coexist, quote paths with spaces, and start the correct binary.
- [ ] Disabling autostart removes only the current executable's value.
- [ ] A second app launch activates the existing settings window without starting another instance.
- [ ] Restart preserves all settings and runtime-derived windows are recreated correctly.

## Packaging

- [ ] `pnpm tauri build --ci` produces the release executable and exactly one NSIS installer.
- [ ] `node scripts/package-portable.mjs` produces exactly one versioned portable zip.
- [ ] Portable zip contains only `FloePod.exe`, `.floepod-portable`, and `使用说明.txt`.
- [ ] Installer and portable executable both launch on a clean Windows 10/11 machine with WebView2.
- [ ] Release checksums match the produced artifacts.

