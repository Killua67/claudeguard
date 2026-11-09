# 开源与发行状态

ClaudeGuard 当前为 macOS 源码预览。Windows 已接入只读代理预检，`main` 的 GitHub Actions 构建通过后会提供 x64 测试安装包，仍需 Windows 实机验收。

## 本地公开准备

- 用 `public-files.json` 精确列举公开文件；导出器拒绝重复路径、路径穿越、软链接、非白名单文件类型和常见敏感文本。它是辅助检查，不替代人工审查或完整秘密扫描。
- 公开目录不包括个人交接、Swift 历史实现、隔离实验、缓存、截图、诊断产物或旧图标；开发工作区中的这些内容保持原样。
- 公开目录必须从清单重新生成，不能整目录上传；导出仅写入不存在的新目录，出错清理本轮临时导出，不覆盖已有文件。
- 锁文件和原创图标随源码保留；依赖元数据及素材来源见 `THIRD_PARTY_NOTICES.md`。

## 发布前仍需完成

- 已选 Apache-2.0 并包含 LICENSE / NOTICE；源码发布目标为 `MaxRouterAI/claudeguard`。私密漏洞报告是否启用以仓库 Security 页面实际状态为准，不在公开 Issue 中提交可利用细节。
- 在干净 macOS 环境运行构建及演示验收；公开远端 CI 要有实际通过记录。当前 CI 文件准备不等于已经在远端运行。
- 在对应实机验证支持矩阵，未经验证的平台不提供可用性承诺。
- 对广泛分发的安装包完成 Developer ID 签名、公证、校验值、安装/升级/卸载和回滚验收；发布凭据不能提供给不可信 PR。
- 真实客户端启动未由代理测试，需维护者另行决定验收，不将这项缺口隐藏为“全面支持”。

此测试仓库只提交审核后的源码清单。Windows 安装包保存在对应 GitHub Actions 运行的 Artifacts 中，用于实机验收。上游项目为 [MaxRouterAI/claudeguard](https://github.com/MaxRouterAI/claudeguard)。
