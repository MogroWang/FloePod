# 一 PR 一版本的自动发布

## 合并前

版本在 PR 中统一更新 `package.json`、`src-tauri/tauri.conf.json`、Cargo package、Cargo.lock 根包和 README 当前版本。`pnpm version:check` 检查一致性；PR 和 `merge_group` 的 Version job 读取实时默认分支的不可变快照、最新 Release 与精确标签，要求版本严格增加且未被使用。Windows 安装包约束为稳定三段版本，范围不超过 `255.255.65535`。

CI 在 PR opened/synchronize/reopened/ready_for_review 和 merge_group checks_requested 执行，只使用只读令牌，不给 PR 注入签名证书、写令牌或 attestation 权限。质量检查包括前端/发布脚本回归测试、格式、版本、IPC、Rust 测试/Clippy、依赖审计、Windows 生产构建和五种产物验证。每周计划只复查依赖告警。

## 管理员必须设置的仓库规则

本次实际登录的 GitHub Settings 页面显示 **You don't have access to repository options**；API 也没有 admin 权限。以下规则没有被自动配置，不能把工作流存在等同于分支保护已生效。

在 Settings → Rules → Rulesets，为默认分支设置 active 规则（或等价分支保护）：

1. Require a pull request before merging。
2. Require status checks to pass，并要求分支基于最新 base，选择三个检查：
   - `Version consistency and uniqueness`
   - `Audit JavaScript and Rust dependencies`
   - `Validate and package Windows x64`
3. 如果启用 Merge Queue，**Maximum pull requests to merge 必须是 1**；required checks 已支持 `merge_group`。不要把多个各自携带版本号的 PR 一次合入。
4. 不设置允许绕过上述版本/质量门禁的自动发布路径。Actions 必须允许仓库中的固定 SHA actions；Release job 必须能请求 contents:write、id-token:write、attestations:write。

没有这些保护时，拥有直接写权限的用户仍可能合入重复版本；工作流会拒绝抢占现有标签，但不能替代仓库合并规则。

## 合并事件与受信任发布

`merge-notification.yml` 使用 `pull_request_target: closed`，只有 merged=true 且 base 为默认分支才成功通知。它不 checkout、不执行依赖、不读取 secrets、不生成待执行 artifact，只把 GitHub 给出的 PR number 与 merge_commit_sha 固定在 run-name。

`release.yml` 由该通知的 `workflow_run: completed` 触发。只读 resolve job 从本次 workflow 的不可变定义加载校验器，调用 API 核实通知 run ID、workflow ID/path、事件、仓库、成功状态、PR merged 状态、默认 base、精确 merge SHA 与默认分支祖先关系。PR head 和 fork 的代码不会被 checkout；移动的 main 只用于祖先验证，不用作构建源码。

GitHub 官方文档说明 `workflow_run` 后续工作流可获得 secrets 和写令牌，即使上游没有这些权限。Dependabot 的 pull_request_target 可能受只读和 secrets 限制，因此使用无权限通知 → GitHub 原生 workflow_run 的两阶段衔接。fork 来源本身不是拒绝理由；可信身份来自目标工作流和已经进入默认分支的提交。该设计不依赖 PAT，不依赖 `GITHUB_TOKEN` 创建 tag 后再触发另一个 workflow。[事件与 workflow_run 权限](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#workflow_run)、[Dependabot 权限限制](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-on-actions)。

merge_commit_sha 在 merged 后分别表示 merge commit、squash 后 base commit 或 rebase 更新到的最终 base commit；校验器比对通知中的快照与 PR API。Merge Queue 必须一次一 PR，并依赖同样的最终 PR merge 身份。[PR API 的 merge SHA 语义](https://docs.github.com/en/rest/pulls/pulls#get-a-pull-request)。本地测试覆盖这些身份约束、fork head 与 base 分离以及 main 后续前进；实际远端合并事件和 secrets 权限仍以合并后运行结果为准。

## 排队、幂等与恢复

全仓库共享 `floepod-release` concurrency group，`cancel-in-progress: false`、`queue: max`。GitHub 当前最多保存 100 个 pending，按进入队列顺序处理；不是无限队列，也不保证所有 runner 的调度顺序等于 PR merge 时间。旧版本后发布时不会错误替代较新 Latest Release。[当前并发队列语法](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#concurrency)。

标签只允许不存在时创建，或已存在且最终指向同一个精确 SHA；不移动已占用标签。公开 Release 重跑时验证来源标记和全部附件摘要，成功即结束。中断的 draft 可以重新构建并覆盖 draft 附件，但公开附件不覆盖。上传后核对七个文件的数量、大小、uploaded 状态和 GitHub SHA-256 digest，全部符合才发布 draft，再次验证远端资产。

恢复方式：重跑失败的 Release run；或从默认分支手动触发 Release Windows，输入原来的成功 **Merged PR notification run ID**。不能输入 PR head、任意 branch 或任意 SHA 绕过来源验证。100 pending 上限溢出等 GitHub 外部失败需要管理员用原通知 ID 恢复；不会谎报已发布。

## 打包、签名与来源证明

合并后不重复执行 lint/test/audit/契约检查；只验证源码身份、版本和发布状态，生产构建、打包、签名、校验、证明和发布。

先构建 EXE，有证书时通过 Tauri `signCommand` 回调签署各包修改了包类型标记后的程序，以及安装器和卸载器。Tauri 打包结束会恢复原始 EXE，因此随后再签署裸 EXE，用它制作 MSIX 和 ZIP；最终逐一验证签名，核对 ZIP/MSIX 内程序与裸 EXE 的摘要。MSIX Publisher 从证书主题取得。签名使用现有两个 secrets：`WINDOWS_SIGN_CERTIFICATE_BASE64`、`WINDOWS_SIGN_CERTIFICATE_PASSWORD`。缺少证书允许 unsigned 发布，manifest 和 Release Notes 明确标注；有证书却签名失败会终止。当前本地验证没有生产证书，不声称已完成真实签名验证。[Tauri 自定义签名命令](https://v2.tauri.app/distribute/sign/windows/#custom-sign-commands)。

最终五种包加 `release-manifest.json`、`SHA256SUMS.txt` 统一生成 attestation。自定义 SLSA v1 predicate 的 `buildType` 必须使用服务端为该 predicate 类型唯一放行的官方 `https://actions.github.io/buildtypes/workflow/v1`，自造 buildType 会在上传时被以 unsupported build type 拒绝（[actions/attest#195](https://github.com/actions/attest/issues/195)）；服务端还会按该 buildType 的格式校验 predicate——`externalParameters.workflow.ref` 等 ref 类字段必须等于本次 run 的 git ref（如 `refs/heads/main`），commit SHA 只能出现在 `digest.gitCommit`，`internalParameters.github` 必须包含 run 的 OIDC 身份字段，否则上传被以 values do not match / required predicate value is unset 拒绝。predicate 因此由 job 的 OIDC token claims 按官方生成器的形状构造，本次实际构建的 immutable source SHA 记录在 externalParameters.source 与 `digest.gitCommit`，执行 workflow 的定义 SHA 记录在 `internalParameters`，避免 workflow_run 默认 github.sha 把后续 main 误认成源码。来源证明签署最终字节，不把 Authenticode 未签名描述成已签名。未合并关闭 PR、普通 push main、普通 tag push 均不触发发布。

工作流使用固定完整 action SHA。`contracts/github-workflow.schema.json` 是 2026-09-08 从 SchemaStore 取得的快照，用于离线 PR 校验，已包含 queue:max；actionlint 1.7.12 尚未包含这个新字段，其唯一 queue 告警不代表 GitHub 当前语法不支持。其他语法和 action 引用仍由测试检查。
