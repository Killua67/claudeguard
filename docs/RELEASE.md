# 开源与发行状态

ClaudeGuard `0.6.0` 的正式发行范围为 Windows x64。本仓库基于上游 [MaxRouterAI/claudeguard](https://github.com/MaxRouterAI/claudeguard)，macOS 仍为源码预览；不发布未经验收的 macOS 安装包，也不支持 WSL / Linux。

## 验证范围与已知限制

- 用户已在一台 Windows 实机通过 ClaudeGuard 启动官方 Desktop，并确认可联网使用。开发自动化测试仅使用规则、模拟和无害替身，不运行真实 Claude。
- Windows 原生 CLI 已实现签名校验、冻结配置复检和原生控制台启动，尚未完成真实 CLI 手动验收；其他机器、安装/升级/卸载和回滚仍待验收。
- 出现过代理身份最终复核失败、重新检查后通过的情况，根因尚未定位。此错误也包含身份读取失败或路径不匹配；失败仍阻断，不放宽身份、时效或最终授权要求。
- 安装包未代码签名，可能触发 Windows 安全提示。正式版本号不代表已完成签名、全平台兼容或广泛分发验收。
- Desktop 联网成功与固定 Cloudflare 预检均不证明所有 Claude 流量经过代理，IPv4/IPv6 实际客户端流量覆盖仍未验证；Windows 原生通知未支持。

## 发布流程

- 将正式版本同步到 Cargo、前端及 Tauri 配置，保留现有应用标识和用户配置，不自动替换本机安装。
- 使用 `public-files.json` 精确审核公开文件，重新生成 `PUBLIC-SHA256SUMS`，运行 Rust、前端和公开导出检查。
- 推送审核后的源码至 `main`，确认对应提交的 GitHub Actions 检查和 Windows 安装包构建成功；不能用旧构建或未执行的 CI 代替。
- 从成功构建下载 Windows x64 NSIS 安装包，生成并核对 `SHA256SUMS.txt`；以相同源码提交创建 `v0.6.0` 正式 Release，附件仅包含审核后的安装包与校验值。
- 发布是否完成以本仓库 GitHub Releases 的实际状态为准。Actions 中 `ClaudeGuard-Windows-x64-preview` 是构建产物名称，不等于已经发布 Release。

## 公开边界

- 已选 Apache-2.0 并包含 LICENSE / NOTICE。私密漏洞报告是否启用以仓库 Security 页面实际状态为准，不在公开 Issue 中提交可利用细节。
- 公开目录不包括个人交接、Swift 历史实现、隔离实验、缓存、截图、诊断产物或旧图标；开发工作区中的这些内容保持原样。
- 公开目录必须从清单重新生成，不能整目录上传；导出仅写入不存在的新目录，出错清理本轮临时导出，不覆盖已有文件。
- 导出器拒绝重复路径、路径穿越、软链接、非白名单文件类型和常见敏感文本；它是辅助检查，不替代人工审查或完整秘密扫描。
- 锁文件和原创图标随源码保留；依赖元数据及素材来源见 `THIRD_PARTY_NOTICES.md`。发布凭据不能提供给不可信 PR。
