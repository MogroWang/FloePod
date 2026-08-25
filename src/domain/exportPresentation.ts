import type { ExportMode, ExportResult } from "./types.ts";

export interface ExportPresentation {
  /** `null` preserves the current selection; an array replaces it. */
  selection: number[] | null;
  message: string;
}

export function presentExport(result: ExportResult, mode: ExportMode): ExportPresentation {
  const verb = mode === "move" ? "移动" : "复制";
  let selection: number[] | null = null;
  if (result.failed.length || result.warnings.length) {
    selection = result.failed.map((issue) => issue.id);
  } else if (mode === "move") {
    selection = [...result.skippedIds];
  }

  let message: string;
  if (result.failed.length || result.warnings.length) {
    const parts = [`已完成 ${result.completedIds.length} 项`];
    if (result.staleIds.length) parts.push(`清理 ${result.staleIds.length} 条失效索引`);
    if (result.failed.length) parts.push(`${result.failed.length} 项可重试`);
    if (result.warnings.length) parts.push(`${result.warnings.length} 项需检查`);
    const first = result.failed[0] ?? result.warnings[0];
    message = `${parts.join("，")}：${first.name} ${first.error}`;
  } else if (result.skippedIds.length) {
    const stale = result.staleIds.length ? `，清理 ${result.staleIds.length} 条失效索引` : "";
    message = `${verb}完成 ${result.completedIds.length} 项，跳过 ${result.skippedIds.length} 项${stale}`;
  } else if (result.staleIds.length) {
    message = `已${verb} ${result.completedIds.length} 项，清理 ${result.staleIds.length} 条失效索引`;
  } else {
    message = `已${verb} ${result.completedIds.length} 项`;
  }
  return { selection, message };
}
