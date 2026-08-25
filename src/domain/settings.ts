import type { ThemeMode } from "./types.ts";

export function resolveTheme(mode: ThemeMode, systemDark: boolean): "light" | "dark" {
  return mode === "system" ? (systemDark ? "dark" : "light") : mode;
}

/** Lexical fallback used only to reconcile an OOBE request whose IPC response
 * may have been lost. Rust remains authoritative for filesystem identity. */
export function normalizeWindowsPathKey(input: string): string {
  const source = input.trim().replace(/\//g, "\\");
  if (!source) return "";
  const unc = source.startsWith("\\\\");
  const driveRoot = /^[a-zA-Z]:\\+$/.test(source);
  const parts = source.split("\\").filter((part) => part !== "" && part !== ".");
  let normalized = parts.join("\\");
  if (unc) normalized = `\\\\${normalized}`;
  else if (driveRoot && /^[a-zA-Z]:$/.test(normalized)) normalized += "\\";
  return normalized.toLowerCase();
}
