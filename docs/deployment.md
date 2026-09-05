# FloePod Windows 部署与机构策略

FloePod 的 Community 功能不需要账号、在线许可证或云服务。机构可以使用 MSI、MSIX 或 NSIS 包离线部署，并通过本机策略文件限制高风险行为。

## 构建产物

```powershell
pnpm install --frozen-lockfile
pnpm tauri build --ci
node scripts/package-portable.mjs
pwsh -File scripts/package-msix.ps1
```

构建后应获得：

- NSIS 安装程序；
- MSI 安装程序；
- MSIX 包；
- 裸 `FloePod.exe`；
- 便携 ZIP。

CI 和 Release 会对最终资产生成 `SHA256SUMS.txt` 与 GitHub 构建来源证明。正式分发前应配置 Authenticode 证书；仓库不会附带、生成或伪造生产证书。

## Authenticode

Release 工作流支持以下 GitHub Actions secrets：

- `WINDOWS_SIGN_CERTIFICATE_BASE64`：PFX 文件的 Base64；
- `WINDOWS_SIGN_CERTIFICATE_PASSWORD`：PFX 密码。

证书存在时，工作流使用 Windows SDK `signtool.exe` 对 exe、NSIS、MSI 和 MSIX 签名并执行 `/pa /all` 验证。证书只写入 runner 临时目录，流程结束后删除。没有配置证书时仍能构建测试产物，但不得把它们宣传成“已签名正式版”。

本地签名也可使用：

```powershell
$env:FLOEPOD_SIGN_CERT_PATH = "D:\secure\floepod.pfx"
$env:FLOEPOD_SIGN_CERT_PASSWORD = "由安全输入提供"
pwsh -File scripts/sign-windows-artifacts.ps1 -Path .\dist\FloePod-1.4.0-win-x64.msix
```

不要把证书或密码提交到仓库。

## 静默安装

```powershell
msiexec.exe /i FloePod_1.4.0_x64_en-US.msi /qn /norestart
```

NSIS 通常支持 `/S`，MSIX 可由管理员使用 `Add-AppxPackage`、Intune 或受信任的软件分发系统部署。MSIX 的 `Publisher` 必须与签名证书主题完全一致；可通过 `FLOEPOD_MSIX_PUBLISHER` 或 `package-msix.ps1 -Publisher` 指定。

## 机构策略

将 [organization-policy.example.json](organization-policy.example.json) 复制为：

```text
%PROGRAMDATA%\FloePod\organization-policy.json
```

支持字段：

- `disableMove`：禁止移动源文件和移动导出；
- `requireCopyDefault`：机构界面应以复制作为首选；
- `requirePrivacyScan`：禁止普通导出，要求使用安全导出或可信交接包；
- `lockRules`：禁止用户修改规则匣；
- `disableFulltextIndex`：禁止全文和 OCR 索引；
- `allowedDataRoots`：暂存目录白名单；
- `maximumHistoryDays`：审计导出的最大历史范围；
- `mandatoryRetentionDays`：敏感匣最长本地保留天数；
- `diagnosticIncludePaths`：默认 `false`，诊断和审计中隐藏本地路径；
- `supportContact`：设置页显示的机构支持信息。
- `managedHotkeys`：管理员下发的一组全局快捷键；
- `managedPods`：管理员预设的完整匣配置和规则模板。

无策略文件时应用保持普通个人模式。格式无效的策略会明确报错，不会静默降级成“已管理”。

## 审计、诊断和设置迁移

“安心中心”可以：

- 导出 JSON/CSV 操作审计；
- 生成脱敏诊断 ZIP；
- 备份或导入本地设置；
- 验证可信交接包。

诊断包默认不包含文件内容、缩略图、OCR 正文或完整本地路径。设置导入会先完成结构、路径、机构白名单与 EFS 检查，验证失败时保留原设置。

## 支持与 LTS 边界

仓库提供可重复构建、策略格式和诊断包基础，但“长期支持版本”“响应时间承诺”“现场培训”“公益席位”“退款规则”都需要维护者建立真实运营流程，不能仅靠代码声明为已提供。
