/** Negative compile checks: vue-tsc must reject each marked incompatible contract. */
import type { CommandContract, EventContract, PrivacyScanResult, StagedItem } from "./generated";

export function assertContractTypes(
  stage: (args: CommandContract["stage_text"]["args"]) => void,
  patch: (args: CommandContract["update_pod"]["args"]) => void,
  event: (value: EventContract["floepod://panel-state"]) => void,
  item: StagedItem,
  scan: PrivacyScanResult,
) {
  stage({ podId: 1, content: "text", title: null });
  patch({ podId: 1, patch: { panelWidth: "440" } }); // Legacy numeric strings remain valid input.
  // @ts-expect-error Command argument names are camelCase.
  stage({ pod_id: 1, content: "text", title: null });
  // @ts-expect-error A file ID cannot be passed as text.
  stage({ podId: "1", content: "text", title: null });
  // @ts-expect-error Settings patches have real field types.
  patch({ podId: 1, patch: { enabled: "false" } });
  // @ts-expect-error Unknown patch keys are not silently accepted.
  patch({ podId: 1, patch: { panelWidht: 440 } });
  // @ts-expect-error Event payload shape is fixed by the native emitter.
  event({ mode: "list", paths: [], pinned: true });
  // @ts-expect-error Public enumerations retain their literal values.
  event({ mode: "other", paths: [], pinned: true, visible: true, draggingOut: false });
  // @ts-expect-error Optional Rust values are nullable, not always strings.
  const original: string = item.originalPath;
  // @ts-expect-error A skipped scan cannot be represented as an arbitrary boolean.
  scan.files[0].status = false;
  return original;
}
