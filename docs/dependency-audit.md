# 依赖审计记录（2026-09-08）

本次保持现有大版本兼容范围，更新 Vue 3.5.42、Pinia 3.0.4、TypeScript 5.9.3、vue-tsc 3.3.11；前后端 Tauri clipboard-manager 2.3.3、dialog 2.7.3、opener 2.5.5；Rust dialog 的 fs 依赖更新到 2.5.2。未为消除告警跨越 Tauri/GTK 的不兼容大版本。

新增构建/开发依赖：schemars 产生真实 DTO Schema、syn/quote 提取实际命令签名、Prettier 固定格式、yaml/ajv 校验固定工作流 Schema。tempfile 从测试依赖提升为运行依赖，用于独占临时输出目录。所有解析、队列和扫描仍在本机执行，没有引入云解析或运行时网络请求。

官方 npm registry 审计未发现已知漏洞。RustSec `cargo audit` 未发现 vulnerability 类条目，但仍有以下告警，CI 原样显示：

| 告警 | 锁定包 | 实际路径与处理 |
| --- | --- | --- |
| RUSTSEC-2024-0370 unmaintained | proc-macro-error 1.0.4 | glib-macros → glib/GTK 平台链；Windows x86_64-msvc 依赖树不包含它。保留跨平台锁文件，未伪造升级 |
| RUSTSEC-2024-0429 unsound | glib 0.18.5 | GTK/drag/tray 的非 Windows 路径；Windows 目标不可达。修复版本要求 glib ≥0.20，不能直接替换 Tauri 所用 GTK 0.18 ABI；未来 Linux 支持仍需上游迁移 |
| RUSTSEC-2025-0081、0075、0080、0100、0098 unmaintained | unic-char-property/range/common/ucd-ident/ucd-version 0.9.0 | Windows 构建与运行树实际可达：urlpattern → tauri-utils → Tauri。没有在本次范围内找到兼容的维护版替换；不是“Windows 不受影响” |

未维护不等于已证明可利用，Windows 不可达也不等于其他平台已安全。没有添加 advisory ignore、审计失败降级或跳过锁文件校验。每周计划再次查询新披露告警。离线本地验证所用 vendor/cache 位于工作区外层，不提交仓库；下载的 crates 用 crates.io 索引 SHA-256 校验，正式 CI 仍使用官方 registry 和 `--locked`。
