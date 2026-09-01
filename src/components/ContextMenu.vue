<script setup lang="ts">
/**
 * 右键菜单的纯渲染组件：浮匣设计语言（近实心卡片、悬浮高亮、克制动效）。
 * 宿主负责定位与动作执行——菜单窗口模式由 ContextMenuWindow 回传选择，
 * 浏览器预览降级时由 PodPanel 内嵌执行。
 */
import type { MenuItemSpec } from "@/domain/menu";

defineProps<{ items: MenuItemSpec[] }>();
const emit = defineEmits<{ (e: "execute", item: MenuItemSpec): void }>();

/** 菜单图标按动作 id 映射（lucide 风格线性路径）。 */
const ICON_PATHS: Record<string, string[]> = {
  open: [
    "M18 13v6a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h6",
    "M14 4h6v6",
    "M20 4l-9 9",
  ],
  reveal: [
    "M10 14 20 4M14 4h6v6M11 5H6a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2v-5",
  ],
  copy: ["M9 9h10a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V10a1 1 0 0 1 1-1Z", "M5 15V5a2 2 0 0 1 2-2h10"],
  copyPath: [
    "M9.5 14.5 14.5 9.5",
    "M10.5 6.5 12 5a4.24 4.24 0 0 1 6 6l-1.5 1.5",
    "M13.5 17.5 12 19a4.24 4.24 0 0 1-6-6l1.5-1.5",
  ],
  remove: [
    "M5 7h14M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m3 0-1 13a1.5 1.5 0 0 1-1.5 1.4h-7A1.5 1.5 0 0 1 6.5 20L5.5 7",
  ],
};

function iconPaths(id: string): string[] {
  return ICON_PATHS[id] ?? [];
}
</script>

<template>
  <div class="menu-card" role="menu">
    <template v-for="(item, index) in items" :key="`${item.id}-${index}`">
      <div v-if="item.separator" class="menu-sep" role="separator" />
      <button
        v-else
        type="button"
        class="menu-item"
        :class="{ danger: item.danger, disabled: item.disabled }"
        role="menuitem"
        :disabled="item.disabled"
        @click="emit('execute', item)"
      >
        <svg
          v-if="iconPaths(item.id).length"
          class="menu-icon"
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.7"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path v-for="(d, di) in iconPaths(item.id)" :key="di" :d="d" />
        </svg>
        <span class="menu-label">{{ item.label }}</span>
      </button>
    </template>
  </div>
</template>

<style scoped>
.menu-card {
  width: 232px;
  box-sizing: border-box;
  padding: 5px;
  border-radius: 12px;
  /* 透明菜单窗口里不能使用 backdrop-filter：WebView2 会在透明背景上
     渲染出整块发黑的伪影（阴影显示错误的根源），只保留近实心的表面色。 */
  background: color-mix(in srgb, var(--surface) 96%, transparent);
  border: 1px solid var(--glass-line);
  box-shadow: var(--shadow-panel);
  transform-origin: top left;
  animation: menu-pop 160ms var(--ease-out);
}
@keyframes menu-pop {
  from {
    opacity: 0;
    transform: scale(0.96) translateY(-3px);
  }
}
.menu-item {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  height: 32px;
  padding: 0 9px;
  border: 0;
  background: transparent;
  border-radius: 8px;
  color: var(--ink);
  font-size: 12.5px;
  font-weight: 520;
  font-family: inherit;
  letter-spacing: 0.005em;
  text-align: left;
  cursor: default;
  transition: background 100ms ease, transform 80ms ease;
}
.menu-item:hover:not(.disabled) {
  background: color-mix(in oklab, var(--surface-2) 72%, transparent);
}
.menu-item:active:not(.disabled) {
  transform: scale(0.985);
}
.menu-icon {
  flex-shrink: 0;
  color: var(--ink-2);
  transition: color 100ms ease;
}
.menu-item:hover:not(.disabled) .menu-icon {
  color: var(--ink);
}
.menu-item.danger {
  color: var(--danger);
}
.menu-item.danger .menu-icon {
  color: var(--danger);
}
.menu-item.danger:hover:not(.disabled) {
  background: color-mix(in oklab, var(--danger) 13%, transparent);
}
.menu-item.disabled {
  opacity: 0.4;
  cursor: wait;
}
.menu-label {
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.menu-sep {
  height: 1px;
  margin: 5px 8px;
  background: var(--glass-line);
}
</style>
