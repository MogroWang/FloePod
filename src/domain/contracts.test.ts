import assert from "node:assert/strict";
import test from "node:test";

import { dropActionFor } from "./dropAction.ts";
import { presentExport } from "./exportPresentation.ts";
import { formatSize } from "../lib/format.ts";
import { buildItemMenu } from "./menu.ts";
import { monitorLogicalSpan, offsetAfterDrag } from "./podPosition.ts";
import { updateSelection } from "./selection.ts";
import { normalizeWindowsPathKey, resolveTheme } from "./settings.ts";
import { parseWindowLabel } from "./windowLabel.ts";
import type { StagedItem } from "./types.ts";
import { Commands } from "../ipc/commands.ts";
import { Events } from "../ipc/eventNames.ts";

test("窗口标签只匹配对应视图", () => {
  assert.deepEqual(parseWindowLabel("settings"), { kind: "settings" });
  assert.deepEqual(parseWindowLabel("pod_7"), { kind: "podBar", podId: 7 });
  assert.deepEqual(parseWindowLabel("pod_7_panel"), { kind: "podPanel", podId: 7 });
  assert.deepEqual(parseWindowLabel("context_menu"), { kind: "contextMenu" });
  for (const invalid of ["pod_0", "pod_-1", "pod_7_extra", "pod_7_panel_extra", "other"]) {
    assert.equal(parseWindowLabel(invalid), null, invalid);
  }
});

test("右键菜单按选择规模与条目类型组装", () => {
  const file: StagedItem = {
    id: 11, podId: 1, kind: "file", stagingPath: "D:\\s\\a.png", originalPath: null,
    name: "a.png", ext: "png", size: 1, createdAt: 1,
  };
  const text: StagedItem = {
    ...file, id: 12, kind: "text", stagingPath: "D:\\s\\a.txt", name: "a.txt", ext: "txt",
  };

  const single = buildItemMenu([file]);
  assert.deepEqual(
    single.filter((item) => !item.separator).map((item) => item.id),
    ["open", "reveal", "copy", "copyPath", "remove"],
  );
  assert.equal(single[single.length - 1]?.danger, true);

  const multiText = buildItemMenu([text, { ...text, id: 13 }]);
  const copy = multiText.find((item) => item.id === "copy");
  assert.equal(copy?.label, "复制 2 段文字");
  const remove = multiText.find((item) => item.id === "remove");
  assert.equal(remove?.label, "移出 2 项");
  assert.deepEqual(remove?.itemIds, [12, 13]);
  const copyPath = multiText.find((item) => item.id === "copyPath");
  assert.equal(copyPath?.text, "D:\\s\\a.txt\r\nD:\\s\\a.txt");
});

test("多选和范围选择会修复失效锚点", () => {
  const toggled = updateSelection(new Set([2]), [1, 2, 3, 4], 4, "toggle", 2);
  assert.deepEqual([...toggled.selected].sort(), [2, 4]);
  assert.equal(toggled.anchor, 4);

  const range = updateSelection(toggled.selected, [1, 2, 3, 4], 1, "range", 4);
  assert.deepEqual([...range.selected].sort(), [1, 2, 3, 4]);
  assert.equal(range.anchor, 4);

  const repaired = updateSelection(range.selected, [10, 11], 11, "range", 4);
  assert.deepEqual([...repaired.selected], [11]);
  assert.equal(repaired.anchor, 11);
});

test("主题解析和首次设置路径比较保持兼容", () => {
  assert.equal(resolveTheme("system", true), "dark");
  assert.equal(resolveTheme("system", false), "light");
  assert.equal(resolveTheme("dark", false), "dark");
  assert.equal(normalizeWindowsPathKey(" C:/Stage/./Files/ "), "c:\\stage\\files");
  assert.equal(normalizeWindowsPathKey("\\\\Server\\Share\\Folder\\"), "\\\\server\\share\\folder");
  assert.equal(normalizeWindowsPathKey("D:\\"), "d:\\");
  // "D:" 与 "D:\" 是同一个盘根，归一化结果必须一致
  assert.equal(normalizeWindowsPathKey("D:"), "d:\\");
  assert.equal(normalizeWindowsPathKey("d:/"), "d:\\");
});

test("文件大小在单位边界上进位而不是显示 1024", () => {
  assert.equal(formatSize(1023), "1023 B");
  assert.equal(formatSize(1023.9), "1.0 KB");
  assert.equal(formatSize(1024), "1.0 KB");
  assert.equal(formatSize(1536), "1.5 KB");
  assert.equal(formatSize(0), "");
});

test("匣拖动按目标显示器缩放并限制偏移范围", () => {
  const monitor = {
    name: "DISPLAY2",
    label: "显示器 2",
    primary: false,
    x: 1920,
    y: -200,
    width: 3840,
    height: 2160,
    scaleFactor: 2,
  };
  assert.equal(monitorLogicalSpan(monitor, false), 1920);
  assert.equal(monitorLogicalSpan(monitor, true), 1080);
  assert.equal(offsetAfterDrag(0.5, 192, 1920), 0.6);
  assert.equal(offsetAfterDrag(0.95, 200, 1000), 1);
  assert.equal(offsetAfterDrag(0.05, -200, 1000), 0);
});

test("拖入修饰键遵循既定优先级", () => {
  assert.equal(dropActionFor({ ctrl: true, shift: true, alt: true }, "ask"), "copy");
  assert.equal(dropActionFor({ ctrl: false, shift: true, alt: true }, "copy"), "move");
  assert.equal(dropActionFor({ ctrl: false, shift: false, alt: true }, "move"), "shortcut");
  assert.equal(dropActionFor({ ctrl: false, shift: false, alt: false }, "ask"), null);
  assert.equal(dropActionFor({ ctrl: false, shift: false, alt: false }, "shortcut"), "shortcut");
});

test("部分导出失败后只选中可重试项", () => {
  const result = {
    conflicts: [],
    completedIds: [1, 2],
    skippedIds: [3],
    staleIds: [4],
    failed: [{ id: 5, name: "retry.txt", error: "denied" }],
    warnings: [{ id: 2, name: "copied.txt", error: "source retained" }],
  };
  const presentation = presentExport(result, "move");
  assert.deepEqual(presentation.selection, [5]);
  assert.match(presentation.message, /已完成 2 项/);
  assert.match(presentation.message, /1 项可重试/);
  assert.match(presentation.message, /1 项需检查/);

  assert.deepEqual(presentExport({ ...result, failed: [], warnings: [] }, "move").selection, [3]);
  assert.equal(presentExport({ ...result, failed: [], warnings: [] }, "copy").selection, null);
});

test("IPC 命令和事件名保持稳定且不重复", () => {
  const commands = Object.values(Commands);
  assert.equal(new Set(commands).size, commands.length);
  assert.deepEqual(commands, [
    "get_bootstrap", "get_modifier_state", "get_hotkey_defaults",
    "create_pod", "update_pod", "delete_pod", "save_settings", "hold_pending_drop",
    "stage_paths", "stage_text", "list_pod_items", "remove_items",
    "list_operations", "undo_operation", "retry_operation",
    "preview_remove_items", "preview_export_items", "scan_privacy", "safe_export_items",
    "create_handoff", "verify_handoff", "rebuild_search_index", "search_items",
    "update_item_annotation", "get_item_annotation", "get_pod_security_status",
    "unlock_sensitive_pod", "lock_sensitive_pod", "lock_all_sensitive_pods",
    "get_organization_policy", "export_audit_log", "export_diagnostic_bundle",
    "export_settings_file", "import_settings_file", "export_items",
    "prepare_drag_cut", "finalize_drag_cut", "cancel_drag_cut", "read_thumbnail", "show_panel",
    "toggle_panel", "hide_panel", "set_panel_mode", "get_panel_state", "report_presence",
    "set_panel_pinned", "set_dragging_out", "set_pod_accept", "set_panel_size", "move_pod_bar",
    "open_settings", "open_staged_item", "open_pod_folder",
    "copy_staged_to_clipboard", "reveal_staged_items", "write_clipboard_text",
    "read_clipboard_files",
    "context_menu_ready", "open_context_menu", "resize_context_menu",
    "context_menu_choice", "hide_context_menu",
    "log_frontend", "app_log", "quit_app",
  ]);
  const events = Object.values(Events);
  assert.equal(new Set(events).size, events.length);
  assert.deepEqual(events, [
    "floepod://items-changed", "floepod://settings-changed", "floepod://pods-changed",
    "floepod://panel-mode", "floepod://panel-shown", "floepod://panel-pinned",
    "floepod://panel-state", "floepod://panel-hidden", "floepod://collect-clipboard",
    "floepod://request-file-picker",
    "floepod://pod-lock-changed",
    "floepod://context-menu-show", "floepod://context-menu-choice", "floepod://context-menu-closed",
  ]);
});
