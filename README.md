<p align="center">
  <img src="docs/fp-logo.png" alt="浮匣 FloePod" width="420" />
</p>

# 浮匣 FloePod

本地优先的 Windows 屏幕边缘文件暂存工具。把文件、图片、文字拖到屏幕边缘的浮匣里集中保管，需要时再拖出去或批量导出。不联网、无遥测，所有数据只存在你自己的电脑上。

当前源码版本 **1.0.0**，提供 x64 Windows 便携包和 NSIS 安装包。

## 功能

- **多匣体系**：多个「匣」各自独立贴在屏幕的上/下/左/右边缘，可分别设置所在显示器、沿边缘位置、暂存文件夹、不透明度与材质（亚克力 / 纯半透明），随时启用或停用
- **拖入即暂存**：文件、文件夹、图片拖到匣上松手即入暂存；落地动作可选 复制 / 移动 / 创建快捷方式（可固定选择，也可用修饰键临时直达：`Ctrl`=复制、`Shift`=移动、`Alt`=快捷方式）
- **文字暂存**：全局热键一键收集剪贴板文字，或面板手动输入（可填文件标题，默认取正文首行；自动存为 `.txt`）
- **暂存面板**：悬停或单击浮匣弹出（不抢焦点）；文件缩略图、多选、批量「复制到… / 移动到…」导出、拖出（复制 / 剪切）、移出暂存（进回收站，可反悔）
- **单一活动面板**：同一时刻只展开一个未固定的面板，切匣自动收起其他面板，避免多匣窗口重叠闪烁
- **文件夹对账**：启动或更换暂存文件夹时，自动把文件夹里已有的文件读入列表；资源管理器里手动增删也会同步
- **主题跟随**：浅色 / 深色 / 跟随系统；启动预置主题避免白色首帧闪烁，原生与 WebView 双通道同步
- **多窗口定向事件**：每个匣的事件只发给自己的窗口，多匣并存互不串扰
- **系统集成**：托盘菜单、全局快捷键（默认 `Alt+Shift+F` 显示/隐藏匣、`Alt+Shift+S` 收集剪贴板、`Alt+Shift+P` 打开面板）、开机自启（可选）
- **显式便携模式**：便携包带 marker，数据写入 exe 旁 `FloePodData/`；安装版写入 `%APPDATA%\FloePod`，已有便携数据继续自动识别

## 技术栈

- 前端：Vue 3 + TypeScript + Vite + Pinia（设计令牌走纯 CSS 变量，无 UI 框架运行时）
- 后端：Rust + Tauri 2（多窗口、原生拖放、托盘、全局快捷键、系统主题）
- 存储：SQLite（rusqlite bundled，含 WAL）——暂存条目 + 匣配置 + 设置
- 拖出：`tauri-plugin-drag`（Windows OLE 拖放，剪切模式遵循移动契约回删源文件）
- 圆角：Windows 11 DWM 系统圆角与 CSS 剪裁对齐

## 开发

```bash
pnpm install          # 安装前端依赖
pnpm test             # 前端逻辑与 IPC 测试
pnpm tauri dev        # 开发运行（Rust + Vite 联动）
cargo test --manifest-path src-tauri/Cargo.toml --locked
pnpm build            # 前端类型检查 + 产物构建
pnpm tauri build      # 发布构建（NSIS 安装包 + 裸 exe）
pnpm tauri icon app-icon.png             # 由源图生成全套图标
```

### 结构

```
src/                    # Vue 前端（按窗口 label 分发视图，无需路由）
  domain/               # 共享类型以及选择、主题、窗口和拖放规则
  ipc/                  # Tauri 命令、事件与浏览器预览 mock
  windows/              # PodBar（胶囊条）/ PodPanel（面板）/ SettingsWindow（设置与 OOBE）
  components/           # 文件图形、缩略图、拖入询问、冲突解决与表单控件
  stores/               # Pinia：settings（设置 + 主题）/ staging（条目 + 选中态）
  lib/                  # 弹簧动画、缩略图队列与展示格式化
src-tauri/src/
  lib.rs                # 应用装配（插件/托盘/首启引导/启动恢复）
  manager.rs            # 窗口位置、面板显隐与单一活动面板管理
  commands.rs           # Tauri 命令入口
  pods.rs staging.rs    # 匣配置与暂存条目管理
  file_ops.rs export.rs # 文件操作、批量导出与错误处理
  drag_out.rs thumbnail.rs # OLE 拖出与缩略图读取
  db.rs settings.rs state.rs   # SQLite 持久化与运行态
  clipboard.rs menu.rs   # 系统剪贴板写入、右键菜单窗口管理
  watcher.rs            # 暂存文件夹监听与按匣同步（notify）
  autostart.rs          # 开机自启（HKCU 注册表同步）
  events.rs logging.rs  # 事件名与定向发送、日志
  win.rs lnk.rs paths.rs       # Win32 辅助、.lnk 快捷方式、路径探测
  tray.rs hotkeys.rs          # 托盘与全局快捷键
```

### 便携版打包

```bash
pnpm tauri build
node scripts/package-portable.mjs
# -> dist/FloePod-<版本>-win-x64-portable.zip
```

## 版本

- **1.3.0**（当前）：自动屏蔽（指定应用前台时隐藏全部匣、离开自动恢复）；匣自动收起开关与延迟；面板独立材质与不透明度；材质扩展为亚克力 / 云母 / 模糊 / 普通；位置示意图放大并支持拖动小蓝点定位；修复右键菜单无法再次打开与阴影显示错误
- **1.2.0**：匣宽度 / 圆角 / 边框颜色与不透明度可调；点击匣切换面板显隐并以固定方式弹出；多匣设置改为列表选匣编辑；新建匣弹窗询问名称与文件夹；修复显示器下拉重复「主显示器」
- **1.1.0**：禁用 WebView 原生右键菜单，新增专属设计的右键菜单（打开、打开所在位置、复制到系统剪贴板、复制路径、移出暂存）；文字条目可复制文本内容
- **1.0.0**：更新 FloePod 品牌图形，统一应用、安装包和便携包的版本信息
- **0.6.0**：强化文件复制 / 移动 / 导出回滚与剪切拖出身份检查；修复面板状态竞态、首启幂等、多显示器缩放和安装版 / 便携版路径隔离；增加自动审计、Windows 构建与 Release 流水线
- **0.5.1**：多匣事件定向化消除串扰；文字暂存支持文件标题；文件名截断放宽至 48 字符；启动预置主题防白闪 + 原生主题同步；面板 Windows 11 系统圆角
- **0.5.0**：单一活动面板，看门狗改为直接隐藏避免重叠闪烁；暂存文件夹启动/变更对账；面板尺寸防抖；材质只应用一次减少重绘闪烁
- **0.4.0**：引入多匣体系取代「场景」，每个匣独立配置位置、显示器与暂存文件夹

## 说明

- 需要 WebView2 运行时（Windows 10/11 一般自带）
- 发布前请按 [Windows 发布检查单](docs/release-checklist.md) 完成自动检查和人工验证
- 图标为正式 FP Logo（`docs/fp-logo.png` 横向字标 / `app-icon.png` 应用图标）；如需重新生成图标执行
  `pnpm tauri icon app-icon.png && pnpm tauri build` 即可
