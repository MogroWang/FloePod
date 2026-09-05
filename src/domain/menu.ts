import type { StagedItem } from "@/domain/types";

/**
 * 右键菜单项描述：浮动面板组装 → 菜单窗口渲染 → 选择后原样回传浮动面板执行。
 * 与 Rust `menu::MenuItemSpec` 保持同构（serde camelCase 往返）。
 */
export interface MenuItemSpec {
  /** 动作 id：open / reveal / copy / copyPath / remove；分隔线用 separator。 */
  id: string;
  label: string;
  separator?: boolean;
  danger?: boolean;
  disabled?: boolean;
  /** 目标条目 id；分隔线为空。 */
  itemIds?: number[];
  /** 动作自带的文本负载（复制路径的多行内容）。 */
  text?: string;
}

export function separator(id: string): MenuItemSpec {
  return { id, label: "", separator: true };
}

/**
 * 按当前选择构建条目菜单。单选提供打开 / 定位；多选聚焦批量复制与移出。
 * 全选文字时复制动作复制文本内容，否则复制文件本身。
 */
export function buildItemMenu(entries: StagedItem[]): MenuItemSpec[] {
  const ids = entries.map((item) => item.id);
  const n = ids.length;
  if (n === 0) return [];
  const single = n === 1;
  const allText = entries.every((item) => item.kind === "text");
  const menu: MenuItemSpec[] = [];
  if (single) {
    menu.push({ id: "open", label: "打开", itemIds: ids });
    menu.push({ id: "reveal", label: "打开所在位置", itemIds: ids });
    menu.push(separator("sep-open"));
  }
  menu.push({
    id: "copy",
    label: allText
      ? single
        ? "复制文字"
        : `复制 ${n} 段文字`
      : single
        ? "复制"
        : `复制 ${n} 项`,
    itemIds: ids,
  });
  menu.push({
    id: "copyPath",
    label: single ? "复制文件路径" : `复制 ${n} 条路径`,
    text: entries.map((item) => item.stagingPath).join("\r\n"),
  });
  menu.push(separator("sep-remove"));
  menu.push({
    id: "remove",
    label: single ? "移出暂存" : `移出 ${n} 项`,
    danger: true,
    itemIds: ids,
  });
  return menu;
}
