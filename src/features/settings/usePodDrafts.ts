import { reactive } from "vue";
import { createDrafts, type Draft } from "@/domain/drafts";
import type { Pod } from "@/domain/types";

type NumberField = { [Key in keyof Pod]: Pod[Key] extends number ? Key : never }[keyof Pod];
type ColorField = "borderColor" | "panelColor" | "barColor";

export function usePodDrafts(savePod: (id: number, patch: Partial<Pod>) => Promise<boolean>) {
  const numbers = createDrafts(reactive(new Map<string, Draft<number>>()));
  const colors = createDrafts(reactive(new Map<string, Draft<string>>()));
  const key = (id: number, field: string) => `${id}:${field}`;
  const podNumberValue = (pod: Pod, field: NumberField) =>
    numbers.get(key(pod.id, field)) ?? pod[field];
  const previewPodNumber = (id: number, field: NumberField, value: number) =>
    numbers.preview(key(id, field), value);
  async function commitPodNumber(pod: Pod, field: NumberField, value: number) {
    const draft = previewPodNumber(pod.id, field, value);
    if (await savePod(pod.id, { [field]: value })) numbers.accept(key(pod.id, field), draft);
  }
  function podHexColorValue(raw: string): string {
    const triple = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/i.exec(raw);
    return triple
      ? `#${triple[1]}${triple[1]}${triple[2]}${triple[2]}${triple[3]}${triple[3]}`
      : raw || "#ffffff";
  }
  const colorDraftValue = (pod: Pod, field: ColorField) =>
    colors.get(key(pod.id, field)) ?? pod[field];
  const previewPodColor = (pod: Pod, field: ColorField, value: string) =>
    colors.preview(key(pod.id, field), value);
  const hasColorDraft = (pod: Pod, field: ColorField) => {
    const value = colors.get(key(pod.id, field));
    return value != null && value !== podHexColorValue(pod[field]);
  };
  async function confirmPodColor(pod: Pod, field: ColorField) {
    const draft = colors.current(key(pod.id, field));
    if (draft && (await savePod(pod.id, { [field]: draft.value })))
      colors.accept(key(pod.id, field), draft);
  }
  async function clearPodColor(pod: Pod, field: ColorField) {
    previewPodColor(pod, field, "");
    await confirmPodColor(pod, field);
  }
  return {
    podNumberValue,
    previewPodNumber,
    commitPodNumber,
    podHexColorValue,
    colorDraftValue,
    previewPodColor,
    hasColorDraft,
    confirmPodColor,
    clearPodColor,
  };
}
