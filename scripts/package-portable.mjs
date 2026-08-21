/**
 * 便携版打包：取 tauri build 的裸 exe + 使用说明，压成 zip。
 * 用法：先 pnpm tauri build，再 node scripts/package-portable.mjs
 */
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const EXE = join(ROOT, "src-tauri", "target", "release", "FloePod.exe");
const DIST = join(ROOT, "dist");

if (!existsSync(EXE)) {
  console.error(`未找到 ${EXE}，请先运行 pnpm tauri build`);
  process.exit(1);
}

const version = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8")).version;
const sizeMB = (statSync(EXE).size / 1024 / 1024).toFixed(1);

const README = `浮匣 FloePod v${version}（便携版）
================================

【启动】双击 FloePod.exe 即可运行，无需安装。
首次启动会引导你选择一个「暂存文件夹」。

【基本用法】
1. 把文件 / 图片 / 文件夹拖到屏幕左（右）边缘的浮匣上，松手暂存
   - 默认会询问：复制 / 移动 / 创建快捷方式（可在设置里固定，之后不再问）
   - 按住 Ctrl 拖入 = 复制，Shift = 移动，Alt = 创建快捷方式（临时直达）
2. 鼠标悬停浮匣弹出暂存面板；单击浮匣可固定面板，再单击收起
3. 面板里可以：
   - 把条目拖出到任何程序 / 文件夹（底部可切换「复制 / 剪切」）
   - 多选后「复制到… / 移动到…」批量导出
   - 移出暂存（文件进回收站，可反悔）
4. 文字也能暂存：热键收集剪贴板（默认 Alt+Shift+S），或面板里点「文字」按钮

【快捷键（默认，可在设置中修改）】
  Alt+Shift+F  显示 / 隐藏浮匣
  Alt+Shift+S  收集剪贴板文字到暂存
  Alt+Shift+P  打开暂存面板

【数据位置】
本便携版把数据存在 exe 旁的 FloePodData 文件夹（数据库 + 配置），
整个文件夹拷到 U 盘即可带走。若该目录不可写，回退到 %APPDATA%\\FloePod。

【依赖】Windows 10/11（需 WebView2 运行时，系统一般自带）。

【卸载】删除本文件夹即可，不写注册表（除非开启过「开机自启」，
请先在设置中关闭）。

浮匣 FloePod · 本地优先 · 不联网 · 不收集任何数据
`;

const zipPath = join(DIST, `FloePod-${version}-win-x64-portable.zip`);
mkdirSync(DIST, { recursive: true });
const stageRoot = mkdtempSync(join(DIST, "portable-stage-"));
const stage = join(stageRoot, "FloePod");
const temporaryZip = join(stageRoot, "portable.zip");

try {
  mkdirSync(stage);
  copyFileSync(EXE, join(stage, "FloePod.exe"));
  // Marker prevents the installed build from selecting its program directory merely because it is
  // writable, while keeping the zip build explicitly portable on first launch.
  writeFileSync(join(stage, ".floepod-portable"), "portable\n", "utf8");
  writeFileSync(join(stage, "使用说明.txt"), README, "utf8");

  const expectedFiles = [".floepod-portable", "FloePod.exe", "使用说明.txt"].sort();
  const entries = readdirSync(stage, { withFileTypes: true });
  const actualFiles = entries.map((entry) => entry.name).sort();
  if (
    entries.some((entry) => !entry.isFile()) ||
    actualFiles.length !== expectedFiles.length ||
    actualFiles.some((name, index) => name !== expectedFiles[index])
  ) {
    throw new Error(`便携版暂存目录包含非预期内容: ${actualFiles.join(", ")}`);
  }

  execFileSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      "Compress-Archive -LiteralPath $env:FLOEPOD_PORTABLE_STAGE -DestinationPath $env:FLOEPOD_PORTABLE_ZIP -Force",
    ],
    {
      env: {
        ...process.env,
        FLOEPOD_PORTABLE_STAGE: stage,
        FLOEPOD_PORTABLE_ZIP: temporaryZip,
      },
    },
  );
  // Only expose the final release name after compression completed. A killed/failed
  // Compress-Archive process can therefore never leave a half-written public artifact.
  rmSync(zipPath, { force: true });
  renameSync(temporaryZip, zipPath);
} finally {
  rmSync(stageRoot, { recursive: true, force: true });
}

console.log(`OK ${zipPath}  (exe ${sizeMB} MB)`);
