import type { ThemeMode } from "./types.ts";

export function resolveTheme(mode: ThemeMode, systemDark: boolean): "light" | "dark" {
  return mode === "system" ? (systemDark ? "dark" : "light") : mode;
}

/** 首次设置的 IPC 响应丢失时用于比对路径；文件身份仍以后端结果为准。 */
export function normalizeWindowsPathKey(input: string): string {
  const source = input.trim().replace(/\//g, "\\");
  if (!source) return "";
  const unc = source.startsWith("\\\\");
  // "D:" 与 "D:\" 是同一个盘根；尾随反斜杠可有可无。
  const driveRoot = /^[a-zA-Z]:(\\+)?$/.test(source);
  const parts = source.split("\\").filter((part) => part !== "" && part !== ".");
  let normalized = parts.join("\\");
  if (unc) normalized = `\\\\${normalized}`;
  else if (driveRoot && /^[a-zA-Z]:$/.test(normalized)) normalized += "\\";
  return normalized.toLowerCase();
}
