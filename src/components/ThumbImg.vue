<script setup lang="ts">
/** 图片缩略图：走 Rust 命令读取字节（仅限暂存文件夹内的图片），非图片回落为图形 */
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { ipc } from "@/lib/ipc";
import { withThumbnailSlot } from "@/lib/thumbnailQueue";
import type { ItemKind } from "@/types";
import FileGlyph from "./FileGlyph.vue";

const props = defineProps<{
  kind: ItemKind;
  path: string;
  ext: string | null;
  name: string;
}>();

const url = ref<string | null>(null);
const boxEl = ref<HTMLElement | null>(null);
let loadSequence = 0;
let visible = false;
let observer: IntersectionObserver | null = null;

const IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico"];

function replaceUrl(next: string | null) {
  if (url.value) URL.revokeObjectURL(url.value);
  url.value = next;
}

async function load() {
  const sequence = ++loadSequence;
  replaceUrl(null);
  if (!visible) return;
  if (!IMAGE_EXTS.includes((props.ext ?? "").toLowerCase())) return;
  try {
    const payload = await withThumbnailSlot(async () => {
      // A component may leave the viewport or be reused while waiting in the
      // shared queue; skip the native IPC entirely when that request is stale.
      if (sequence !== loadSequence || !visible) return null;
      return ipc.readThumbnail(props.path);
    });
    if (payload) {
      const blob = new Blob([new Uint8Array(payload.bytes)], { type: payload.mime });
      const next = URL.createObjectURL(blob);
      if (sequence !== loadSequence) {
        URL.revokeObjectURL(next);
        return;
      }
      replaceUrl(next);
    }
  } catch {
    // Non-images and unreadable files intentionally fall back to FileGlyph.
  }
}

onMounted(() => {
  if (typeof IntersectionObserver === "undefined") {
    visible = true;
    void load();
    return;
  }
  observer = new IntersectionObserver(
    (entries) => {
      const isVisible = entries.some((entry) => entry.isIntersecting);
      if (isVisible === visible) return;
      visible = isVisible;
      if (visible) void load();
      else loadSequence += 1;
    },
    { rootMargin: "160px" },
  );
  if (boxEl.value) observer.observe(boxEl.value);
});
watch(() => [props.path, props.ext], () => void load());
onBeforeUnmount(() => {
  visible = false;
  observer?.disconnect();
  loadSequence += 1;
  replaceUrl(null);
});
</script>

<template>
  <div ref="boxEl" class="thumb-box">
    <img v-if="url" :src="url" :alt="name" class="thumb-img" draggable="false" />
    <FileGlyph v-else :kind="kind" :ext="ext" :size="22" class="thumb-glyph" />
  </div>
</template>

<style scoped>
.thumb-box {
  width: 44px;
  height: 44px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  background: var(--surface-raised);
  border: 1px solid color-mix(in oklab, var(--line) 72%, transparent);
  box-sizing: border-box;
  overflow: hidden;
}
.thumb-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.thumb-glyph {
  color: var(--ink-3);
}
</style>
