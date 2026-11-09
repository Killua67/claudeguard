# 参与 ClaudeGuard

阅读 README、docs/ARCHITECTURE.md 和 AGENTS.md。保持规则与系统副作用分层，不以增加文件数量代替职责清晰。

- 新增行为提供可观察回归，覆盖失败、过期、乱序和配置保存失败；不要仅断言 mock 调用。
- RuleId、策略版本、配置 schema、IPC 快照和脱敏报告为独立契约；文案变化不得改变授权。
- 不运行真实 Claude、版本命令、登录或安装器，不向 Anthropic/Claude 发 HTTP/TLS/DNS 探测；测试使用纯输入或无害本地替身。
- 不读取账号、对话、Keychain、官方配置或 shell 启动文件；不改系统网络。
- 不添加通用 shell、任意 URL、远程可执行规则或关闭 TLS 验证的入口。
- 启动、网络、授权与隐私改动须独立审查；未知平台能力不伪装通过。
- 改源码时同步更新 `public-files.json`。所有测试、Clippy、格式和前端构建通过后才提交；提交不等于发布。

命令见 README。Issue 和测试材料须脱敏，不能含完整 IP、真实项目路径、令牌或客户数据。
