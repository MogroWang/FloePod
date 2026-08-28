/** 展示格式化工具 */

/** 匣的显示不透明度；下限 0.1 与后端校验（0.1..=1.0）一致。 */
export function clampOpacity(opacity: number | undefined | null): number {
  return Math.min(1, Math.max(0.1, opacity ?? 1));
}

/** 长列表预览：最多展示 limit 个，返回剩余数量供"以及另外 N 项"使用。 */
export function previewSlice<T>(items: T[], limit = 6): { shown: T[]; extra: number } {
  return { shown: items.slice(0, limit), extra: Math.max(0, items.length - limit) };
}

/** 缩略图可解码的图片扩展名，与 Rust 侧 image crate 启用的解码器保持一致。 */
export const THUMBNAIL_IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico"];

export function formatSize(bytes: number): string {
  if (bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  // 1023.5~1024 之间四舍五入会显示成 "1024 B"，主动进位成 "1.0 KB"。
  if (i < units.length - 1 && Math.round(v) >= 1024) {
    v = 1;
    i += 1;
  }
  const n = v >= 100 || i === 0 ? Math.round(v).toString() : v.toFixed(1);
  return `${n} ${units[i]}`;
}

const two = (n: number) => n.toString().padStart(2, "0");

export function formatTime(unixMs: number): string {
  const d = new Date(unixMs);
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  const hm = `${two(d.getHours())}:${two(d.getMinutes())}`;
  if (sameDay) return hm;
  const sameYear = d.getFullYear() === now.getFullYear();
  const md = `${d.getMonth() + 1}月${d.getDate()}日`;
  return sameYear ? `${md} ${hm}` : `${d.getFullYear()}年${md}`;
}

export function kindLabel(kind: string): string {
  switch (kind) {
    case "folder":
      return "文件夹";
    case "text":
      return "文字";
    case "shortcut":
      return "快捷方式";
    default:
      return "文件";
  }
}
