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
pwsh -File scripts/package-msix.ps1
```

确认安装包和便携包各只有一份，便携包内仅包含 `FloePod.exe`、`.floepod-portable` 和 `使用说明.txt`。发布前校验产物的 SHA-256。

## 已知发布决策

- **签名取决于真实证书**：Release 已支持从 GitHub Actions secrets 注入 PFX，对 exe、NSIS、MSI、MSIX 逐一签名并验证；未配置证书时产物会如实保持未签名，不能宣传为正式签名版。
- **采用明确的手动更新通道**：未集成 `tauri-plugin-updater`，关于页直接打开 GitHub Releases 最新版本；后续只有在建立可信更新端点和签名策略后才启用自动更新。
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
- [ ] 安心模式在 125% / 150% / 200% 下无不可达控件，仅键盘可以完成选择文件、暂存、搜索、导出、撤销和关闭
- [ ] Narrator 能读出匣、项目、选中状态、对话框标题、错误和操作结果；高对比度及减少动画/透明设置生效
- [ ] 文件选择器、文件夹选择器、Ctrl+V、资源管理器“发送到 FloePod”和 Alt+1…9 均可替代拖拽
- [ ] 暂存、复制导出、移动导出和移出操作出现在时间线；24 小时内撤销成功，修改过的目标文件会被保守拒绝删除
- [ ] 规则匣正确过滤类型/名称/来源/大小，日期重命名和子目录不越出暂存目录，重复检测使用内容哈希
- [ ] 交接包的 CSV/JSON/HTML/SHA256SUMS 完整，验证器能发现缺失或篡改文件
- [ ] 隐私扫描和清理全部离线；原文件未修改，图片、PDF、Office 清理副本可正常打开
- [ ] OCR 与全文索引可重建，标签/备注和筛选可用；禁止索引的敏感匣不会写入正文
- [ ] 在支持 EFS 的 NTFS 卷验证敏感匣、Windows Hello、自动锁定、紧急锁定、缩略图禁止和保留期限
- [ ] 无 EFS、无 Windows Hello、策略 JSON 损坏或目录白名单不匹配时明确失败，不显示虚假的已保护状态
- [ ] NSIS、MSI、MSIX、便携 ZIP、裸 exe 和 SHA256SUMS 均被收集；配置证书时对每个 Windows 包执行签名验证

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
