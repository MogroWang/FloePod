import type { MonitorInfo } from "./types.ts";

export function monitorLogicalSpan(monitor: MonitorInfo, vertical: boolean): number {
  const scale = Number.isFinite(monitor.scaleFactor) && monitor.scaleFactor > 0
    ? monitor.scaleFactor
    : 1;
  const physical = vertical ? monitor.height : monitor.width;
  return physical > 0 ? physical / scale : 0;
}

export function offsetAfterDrag(start: number, delta: number, span: number): number {
  if (!Number.isFinite(span) || span <= 0) return Math.min(1, Math.max(0, start));
  return Math.min(1, Math.max(0, start + delta / span));
}

