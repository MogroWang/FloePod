import { ask } from "@tauri-apps/plugin-dialog";
import { ipc } from "@/ipc/client";
import type { OperationPreview } from "@/domain/types";
export async function confirmPreview(preview: OperationPreview): Promise<boolean> {
  if (!preview.requiresConfirmation) return true;
  const lines = [
    preview.title,
    ...preview.warnings.map((warning) => `注意：${warning}`),
    ...preview.details.slice(0, 6),
  ];
  if (preview.details.length > 6) lines.push(`另有 ${preview.details.length - 6} 项…`);
  const message = lines.join("\n\n");
  if (!ipc.inTauri) return window.confirm(message);
  return ask(message, { title: "操作前预览", kind: "warning" });
}
