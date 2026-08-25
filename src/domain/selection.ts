export type SelectionMode = "set" | "toggle" | "range";

export interface SelectionResult {
  selected: Set<number>;
  anchor: number | null;
}

export function updateSelection(
  selected: ReadonlySet<number>,
  orderedIds: readonly number[],
  id: number,
  mode: SelectionMode,
  anchor: number | null,
): SelectionResult {
  const next = new Set(selected);
  if (mode === "set") return { selected: new Set([id]), anchor: id };
  if (mode === "toggle") {
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return { selected: next, anchor: id };
  }
  if (anchor == null) {
    next.add(id);
    return { selected: next, anchor: id };
  }

  const start = orderedIds.indexOf(anchor);
  const end = orderedIds.indexOf(id);
  if (start < 0 || end < 0) return { selected: new Set([id]), anchor: id };
  const [low, high] = start < end ? [start, end] : [end, start];
  for (const selectedId of orderedIds.slice(low, high + 1)) next.add(selectedId);
  return { selected: next, anchor };
}

