export type WindowTarget =
  | { kind: "settings" }
  | { kind: "podBar"; podId: number }
  | { kind: "podPanel"; podId: number };

export function parseWindowLabel(label: string): WindowTarget | null {
  if (label === "settings") return { kind: "settings" };
  const panel = label.match(/^pod_(\d+)_panel$/);
  const bar = panel ? null : label.match(/^pod_(\d+)$/);
  const podId = Number((panel ?? bar)?.[1]);
  if (!Number.isSafeInteger(podId) || podId <= 0) return null;
  return { kind: panel ? "podPanel" : "podBar", podId };
}
