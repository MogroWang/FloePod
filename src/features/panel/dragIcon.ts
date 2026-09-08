export function makeDragIcon(paths: string[], ext: string | null): string {
  const c = document.createElement("canvas");
  const dpr = window.devicePixelRatio || 1;
  const w = 44;
  c.width = 64 * dpr;
  c.height = 64 * dpr;
  const ctx = c.getContext("2d");
  if (!ctx) return "";
  ctx.scale(dpr, dpr);
  const css = getComputedStyle(document.documentElement);
  const color = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  ctx.fillStyle = color("--surface-raised", "#f6f7f8");
  const r = 10;
  roundRect(ctx, 10, 8, w, 48, r);
  ctx.fill();
  ctx.strokeStyle = color("--line-strong", "#d6dade");
  ctx.lineWidth = 1.5;
  roundRect(ctx, 10, 8, w, 48, r);
  ctx.stroke();
  ctx.fillStyle = color("--ink", "#3d434a");
  ctx.font = "600 13px 'Segoe UI', sans-serif";
  ctx.textAlign = "center";
  ctx.fillText((ext ?? "文件").slice(0, 5).toUpperCase(), 32, 36);
  if (paths.length > 1) {
    ctx.fillStyle = color("--accent", "#2d7ca3");
    ctx.beginPath();
    ctx.arc(46, 44, 11, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = "#fff";
    ctx.font = "600 11px 'Segoe UI', sans-serif";
    ctx.fillText(String(paths.length), 46, 48);
  }
  return c.toDataURL("image/png");
}

function roundRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}
