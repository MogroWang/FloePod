<script setup lang="ts">
/**
 * 位置状态示意图：一个圆角矩形代表屏幕，当前边高亮，
 * 边上位置点随「沿边缘位置」实时移动，下方标注边 · 百分比 · 显示器。
 * 只读展示，点击由下方的「屏幕边缘」分段选择负责（分组与映射）。
 */
import { computed } from "vue";

const props = defineProps<{
  edge: string; // top / right / bottom / left
  offset: number; // 0.0 - 1.0
  monitorLabel?: string;
}>();

const EDGE_LABELS: Record<string, string> = {
  top: "上",
  right: "右",
  bottom: "下",
  left: "左",
};

const pct = computed(() => Math.round(props.offset * 100));

const dotStyle = computed<Record<string, string>>(() => {
  const p = `${pct.value}%`;
  switch (props.edge) {
    case "top":
      return { left: p, top: "0", transform: "translate(-50%, -50%)" };
    case "bottom":
      return { left: p, top: "100%", transform: "translate(-50%, -50%)" };
    case "right":
      return { left: "100%", top: p, transform: "translate(-50%, -50%)" };
    default:
      return { left: "0", top: p, transform: "translate(-50%, -50%)" };
  }
});

const caption = computed(() => {
  const parts = [EDGE_LABELS[props.edge] ?? "?", `${pct.value}%`];
  if (props.monitorLabel) parts.push(props.monitorLabel);
  return parts.join(" · ");
});
</script>

<template>
  <div class="edge-diagram">
    <div class="screen">
      <span class="edge e-top" :class="{ on: edge === 'top' }" />
      <span class="edge e-right" :class="{ on: edge === 'right' }" />
      <span class="edge e-bottom" :class="{ on: edge === 'bottom' }" />
      <span class="edge e-left" :class="{ on: edge === 'left' }" />
      <span class="dot" :style="dotStyle" />
    </div>
    <div class="caption">{{ caption }}</div>
  </div>
</template>

<style scoped>
.edge-diagram {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 4px 0 8px;
}
.screen {
  position: relative;
  width: 76px;
  height: 50px;
  border-radius: 7px;
  border: 1px solid var(--line-strong);
  background: var(--surface-hover);
}
.edge {
  position: absolute;
  background: var(--line);
  transition: background 160ms var(--ease-out), height 160ms var(--ease-out),
    width 160ms var(--ease-out);
}
.edge.on {
  background: var(--accent);
}
.e-top {
  top: 0;
  left: 10%;
  right: 10%;
  height: 3px;
}
.e-bottom {
  bottom: 0;
  left: 10%;
  right: 10%;
  height: 3px;
}
.e-left {
  left: 0;
  top: 12%;
  bottom: 12%;
  width: 3px;
}
.e-right {
  right: 0;
  top: 12%;
  bottom: 12%;
  width: 3px;
}
.dot {
  position: absolute;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--accent);
  box-shadow: 0 0 0 2px var(--surface-raised);
  transition: left 160ms var(--ease-out), top 160ms var(--ease-out);
}
.caption {
  font-size: 11.5px;
  color: var(--ink-3);
  font-variant-numeric: tabular-nums;
  letter-spacing: 0.01em;
}
</style>
