import type { PanelContext } from "./context";
import { ipc } from "@/ipc/client";
import { ref, watch } from "vue";
import { readText } from "@tauri-apps/plugin-clipboard-manager";

export function usePanelText(context: PanelContext) {
  const { showToast, refreshAfterMutation } = context;
  const textOpen = ref(false);
  const textTitle = ref("");
  const textValue = ref("");
  const textBusy = ref(false);
  let draftRevision = 0;
  watch(
    [textTitle, textValue],
    () => {
      draftRevision += 1;
    },
    { flush: "sync" },
  );
  async function stashText() {
    const content = textValue.value.trim();
    if (!content || context.isBusy()) return;
    const revision = draftRevision;
    textBusy.value = true;
    try {
      // 标题留空时自动取正文第一个非空行的前 10 个字。
      const title = textTitle.value.trim() || autoTextTitle(content);
      await ipc.stageText(context.podId(), textValue.value, title || undefined);
      if (revision === draftRevision) {
        textTitle.value = "";
        textValue.value = "";
        textOpen.value = false;
      }
      if (await refreshAfterMutation("文字暂存")) showToast("文字已暂存");
    } catch {
      showToast("暂存失败，请重试");
    } finally {
      textBusy.value = false;
    }
  }

  function autoTextTitle(content: string): string {
    const firstLine = content
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find((line) => line.length > 0);
    return (firstLine ?? "").slice(0, 10);
  }

  /** 一键读取剪贴板填入正文：剪贴板有内容时追加，避免覆盖已输入的文字。 */
  async function pasteClipboard() {
    try {
      const text = await readText();
      if (!text.trim()) {
        showToast("剪贴板里没有文字");
        return;
      }
      textValue.value = textValue.value ? `${textValue.value}\n${text}` : text;
    } catch (err) {
      console.error("read clipboard failed", err);
      showToast("无法读取剪贴板");
    }
  }

  return { textOpen, textTitle, textValue, textBusy, stashText, pasteClipboard };
}
