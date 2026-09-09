import type { Edge, DropAction, Material, ThemeMode } from "@/domain/types";
export const EDGES: { value: Edge; label: string }[] = [
  { value: "top", label: "上" },
  { value: "bottom", label: "下" },
  { value: "left", label: "左" },
  { value: "right", label: "右" },
];

export const DROP_ACTIONS: { value: DropAction; label: string }[] = [
  { value: "ask", label: "询问" },
  { value: "copy", label: "复制" },
  { value: "move", label: "移动" },
  { value: "shortcut", label: "快捷方式" },
];

export const MATERIALS: { value: Material; label: string }[] = [
  { value: "acrylic", label: "亚克力" },
  { value: "plain", label: "普通" },
];

export const THEMES: { value: ThemeMode; label: string }[] = [
  { value: "system", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" },
];
