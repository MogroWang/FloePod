# Windows 发布检查单

发布前先记录 Windows、WebView2、构建提交和显示器配置。涉及删除、移动、升级的场景请使用测试数据。

## 自动检查

```powershell
pnpm install --frozen-lockfile
pnpm test
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
pnpm tauri build --ci
node scripts/package-portable.mjs
```

确认安装包和便携包各只有一份，便携包内仅包含 `FloePod.exe`、`.floepod-portable` 和 `使用说明.txt`。发布前校验产物的 SHA-256。

## 已知发布决策

- **未做代码签名**：安装包和便携包首次运行会触发 SmartScreen / 未知发布者提示，属当前有意取舍；若未来购买证书，在 `tauri.conf.json` 与 Release 流水线中补充签名步骤。
- **无自动更新通道**：未集成 `tauri-plugin-updater`，升级依赖用户手动下载新版；如需热更新再引入并配置端点。
- 第三方 GitHub Actions 均以完整 commit SHA 固定（注释标注版本），升级时更新 SHA 并核对 diff。

## 安装与数据

- [ ] 全新安装会进入首次设置；便携版会在程序旁创建 `FloePodData`
- [ ] 安装版使用 `%APPDATA%\FloePod`，不会误读或覆盖便携数据
- [ ] 旧版本数据库升级后，匣配置和暂存条目仍然完整
- [ ] 暂存目录位于移动硬盘或网络位置时，离线不会影响其他匣
- [ ] 安装版与便携版的开机启动互不覆盖，路径含空格时也能正常启动

## 匣与面板

- [ ] 在不同显示器和四条屏幕边缘创建、移动、启用和停用多个匣
- [ ] 在混合缩放和负坐标显示器布局中检查位置、尺寸与持久化结果
- [ ] 悬停、单击、固定、全局显示/隐藏和 `Alt+F4` 的行为符合预期
- [ ] 同一时刻只有一个普通面板展开；固定、拖放和冲突面板不会被误收起
- [ ] 亮色、暗色、跟随系统、亚克力和纯色材质切换时没有白屏或明显闪烁

## 暂存与导出

- [ ] 文件、文件夹、图片和文字可以暂存，重名项会自动改名
- [ ] `Ctrl`、`Shift`、`Alt` 分别触发复制、移动和快捷方式
- [ ] 同盘和跨盘移动失败时源文件仍可恢复，临时文件不会残留
- [ ] 复制到、移动到和冲突处理能正确区分完成、跳过、失效、失败和警告
- [ ] 移出暂存和清空操作进入回收站，部分失败会准确提示
- [ ] 缩略图、打开、在资源管理器中显示、多选和范围选择工作正常

## 拖出与文件监听

- [ ] 拖出复制不会删除源文件；拖出移动仅在目标确认成功后清理源文件
- [ ] 取消拖出或拖出期间文件发生变化时，源文件和数据库记录保持不变
- [ ] 在资源管理器中新增、重命名、修改或删除文件后，只刷新对应的匣
- [ ] 目录暂时不可读或磁盘断开时，不会把现有数据库记录当作已删除

## 系统集成

- [ ] 托盘菜单、全局快捷键、剪贴板收集和单实例唤起工作正常
- [ ] 安装包与便携包可在干净的 Windows 10/11 环境启动
- [ ] 发布标签与 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 的版本一致
