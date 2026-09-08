import { open } from "@tauri-apps/plugin-dialog";
import { ipc } from "@/ipc/client";
import { BROWSER_PREVIEW_STAGING_ROOT } from "@/lib/env";
export function folderPicker(showToast: (message: string) => void) {
  async function pickFolder(): Promise<string | null> {
    if (!ipc.inTauri) return BROWSER_PREVIEW_STAGING_ROOT;
    try {
      const dir = await open({ directory: true, multiple: false, title: "选择暂存文件夹" });
      return typeof dir === "string" ? dir : null;
    } catch (err) {
      console.error("folder picker failed", err);
      showToast("无法打开文件夹选择器");
      return null;
    }
  }

  return pickFolder;
}
