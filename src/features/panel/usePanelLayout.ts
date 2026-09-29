import type { PanelContext } from "./context";
import { ipc } from "@/ipc/client";
import { nextTick, onBeforeUnmount, ref, type Ref } from "vue";
import type { PanelMode } from "@/domain/types";

export function usePanelLayout(
  context: PanelContext,
  mode: Ref<PanelMode>,
  textOpen: Ref<boolean>,
) {
  const { isMounted } = context;
  const rootEl = ref<HTMLElement | null>(null);
  const headEl = ref<HTMLElement | null>(null);
  const listEl = ref<HTMLElement | null>(null);
  const contentEl = ref<HTMLElement | null>(null);
  const footEl = ref<HTMLElement | null>(null);
  let ro: ResizeObserver | null = null;
  let sizeTimer: number | undefined;
  let resizeSequence = 0;

  function cssPixels(value: string): number {
    const parsed = Number.parseFloat(value);
    return Number.isFinite(parsed) ? parsed : 0;
  }

  function scheduleResize() {
    // 文字暂存视图不参与调高：浮动面板保持打开文字编辑前的尺寸，内容超高时滚动。
    if (textOpen.value) return;
    window.clearTimeout(sizeTimer);
    const sequence = ++resizeSequence;
    sizeTimer = window.setTimeout(async () => {
      await nextTick();
      if (sequence !== resizeSequence || !isMounted()) return;
      const root = rootEl.value;
      const body = listEl.value;
      const content = contentEl.value;
      const head = headEl.value;
      if (!root || !body || !content || !head) return;

      const bodyStyle = getComputedStyle(body);
      const rootStyle = getComputedStyle(root);
      const bodyPadding = cssPixels(bodyStyle.paddingTop) + cssPixels(bodyStyle.paddingBottom);
      const rootBorder =
        cssPixels(rootStyle.borderTopWidth) + cssPixels(rootStyle.borderBottomWidth);
      // .panel-content 有 min-height: 100%：直接量 scrollHeight 会把「当前
      // 视口高度」当成内容高度读回来，叠加 scrollHeight 的整数取整与显示器
      // 非整数缩放的舍入，每次切换视图都收缩约 1px，面板因此越切越短。
      // 测量前临时解除最小高度，读取与视口无关的固有内容高度再恢复。
      const previousMinHeight = content.style.minHeight;
      content.style.minHeight = "0";
      const intrinsicBody = Math.ceil(content.scrollHeight + bodyPadding);
      content.style.minHeight = previousMinHeight;
      const bodyHeight = mode.value === "list" ? Math.min(intrinsicBody, 560) : intrinsicBody;
      const chromeHeight = head.offsetHeight + (footEl.value?.offsetHeight ?? 0) + rootBorder;
      await ipc
        .setPanelSize(context.podId(), Math.ceil(bodyHeight + chromeHeight))
        .catch((err) => console.error("panel resize failed", err));
    }, 110);
  }

  function observeContent() {
    ro = new ResizeObserver(scheduleResize);
    if (contentEl.value) ro.observe(contentEl.value);
  }
  onBeforeUnmount(() => {
    ro?.disconnect();
    window.clearTimeout(sizeTimer);
    resizeSequence += 1;
  });
  const bind = (target: Ref<HTMLElement | null>) => (element: unknown) => {
    target.value = element as HTMLElement | null;
  };
  return {
    rootEl,
    bindHead: bind(headEl),
    bindList: bind(listEl),
    bindContent: bind(contentEl),
    bindFoot: bind(footEl),
    bindRoot: bind(rootEl),
    scheduleResize,
    observeContent,
  };
}
