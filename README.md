# ClaudeGuard · Claude 环境守护

以代理预检模式检查本机代理，运行期间安静观察变化，在异常时提供通知和处理建议。保留 IPv6，明确提醒 IPv4/IPv6 实际客户端流量覆盖未验证。

这是独立社区工具，与 Anthropic 无隶属或背书关系。它不保证账号安全、不验证客户端全部流量，也不实施网络拦截。

此仓库发行 Windows 代理预检与原生启动适配，基于上游 [MaxRouterAI/claudeguard](https://github.com/MaxRouterAI/claudeguard)。问题反馈请使用仓库 Issues，并只提交脱敏信息。

## 当前状态

`0.6.0`，Windows x64 正式版本，使用 Tauri 2、Rust、React 和 TypeScript。安装包未代码签名；macOS 仍为源码预览，不提供 macOS 正式安装包。

| 平台 | 状态 |
| --- | --- |
| macOS ARM64 | 已完成本机检测、界面和模拟客户端验收 |
| macOS Intel | 尚未完成实机验收 |
| Windows x64 | 代理预检与原生客户端手动启动；已在一台实机确认 Desktop 经启动器启动并可联网使用。原生 CLI 尚未完成实机验收 |
| WSL / Linux | 未支持 |

用户已独立手动确认 Windows Desktop 真实启动与联网使用；开发测试仍只使用无害替身，不运行真实 Claude，也不将单机验收宣传为全部客户端全面兼容。

Windows 下载安装、首次检测和 macOS 常见问题见 [桌面使用与排查](docs/DESKTOP-GUIDE.md)。

## 使用流程

1. 打开代理软件，启用系统 HTTP / HTTPS 代理。
2. 首次打开 ClaudeGuard，自动读取系统代理及对应监听进程，核对地址和程序路径，再点击“确认代理并开始检测”。
3. 查看逐项结果。在支持启动的 macOS / Windows 正常模式下，系统代理、旁路、监听身份、HTTPS/TLS、出口等阻断项通过并建立有效基线后，才允许用户手动选择启动 CLI 或 Desktop；点击后再次完整复检。IPv6 覆盖未知以非阻断提醒单列，不要求关闭 IPv6。
4. 收起窗口后继续监测。相同异常去重通知，恢复后仍须完整复检；不会自动启动、结束或重启客户端。

首次识别仅为本地只读操作，不发起外网请求、不扫描常见端口、不执行 PAC、不读取代理订阅。没有发现明确代理时保留空白，由用户人工填写；不使用猜测值或演示数据回退。识别到的程序不等于可信程序，须由用户核对。

此版本只支持无认证、明确端口的 `127.0.0.1` HTTP 代理，系统 HTTP 与 HTTPS 必须一致；PAC、远程代理、仅 SOCKS、仅 TUN 等配置暂不自动适配。地区和时区要求为可选个人偏好，新用户默认不指定；这不代表账号资格或官方风控规则。

已有有效配置继续使用，不会因自动发现而覆盖。设置内可手动“重新识别”，候选仅填入草稿，保存才生效。异常文件保留原样，禁止以默认值静默替代。

Windows 只观察当前用户的系统代理。需要明确的 IPv4 回环监听和可读取的完整代理程序路径；如果开启了自动检测、PAC、监听在所有地址，或旁路里含 `127.*`、`192.168.*` 一类文本通配（固定放行的 `10.*`、`192.168.5.*`、`*.dazhidayong.cn` 除外），预检会报告异常并跳过出口探测。检测不会改系统设置。Windows 启动适配只接受 `%USERPROFILE%\.local\bin\claude.exe` 中的官方原生 CLI，以及常规安装或 `Claude_pzs8sxrjxfjjc` MSIX 的 Desktop；不执行 npm 包装脚本、WSL 或安装器。启动前离线校验 Authenticode 签名及 Anthropic 发布者，并拒绝已有 Desktop 实例。CLI 子进程使用冻结配置完整复检，仍需主进程再次授权；不会生成 shell 脚本。离线签名校验不保证当前证书撤销状态；原生 CLI 仍待实机验收，Windows 原生通知未支持。

## 检查与提醒

15 项检查覆盖系统代理、旁路/PAC、端口进程身份、HTTPS/TLS、出口地区及短时稳定性、DNS、路由/TUN、本地 IPv6、系统与进程时区、时间参考、客户端静态签名、环境冲突、已有进程和最终配置复核。

- 完整检查最多四次固定 Cloudflare trace 请求，全部经过已确认代理；严格 TLS，不跟随重定向、不失败回退直连。
- 后台名义每 5 秒检查本地配置，每 30 秒采样出口；实际延迟包含系统调度及网络超时。
- 网络路径变化、休眠恢复、配置/进程/出口变化或采样过期使旧结果失效；后台恢复不能自动解锁启动。
- 提供事件时间线、脱敏报告、具体处置建议和通知诊断。系统接收、通知中心记录、人工确认分别显示，不能相互冒充。
- 桌面真实模式与仅观察模式共享同用户实例锁，不转发命令行参数、不切换已有进程模式。离线演示不持久化配置或发送系统通知。

本工具不抓包、不解密 TLS、不更改系统代理/DNS/VPN/时区/防火墙，不读取账号、对话、Keychain、官方客户端配置或 shell 启动文件。出口 IP 只在内存比较，不进入界面、日志或报告；事件只保留最近 8 条，退出清除。

Cloudflare 的请求路径不能证明代理对所有 Claude 域名采用相同策略。后台采样不是 kill switch，两次采样之间有空窗，异常通知不能阻止已运行客户端发出请求。

**“代理预检通过”不等于“流量保护已验证”。** IPv6 的地址/路由观察、读取或解析失败均保留“覆盖未验证”提醒，但不会单独阻止预检启动；只有 `IPv6 + Warning` 是非阻断例外，其他警告、异常、缺项及过期状态仍按启动规则阻断。不能通过导入报告、前端状态或手动填写“已覆盖”解除代理异常。

IPv6 快照仅在完整检查时刷新，不承诺每 5 秒监控 IPv6 路由或检测客户端直连。后台继续检查代理、监听身份、IPv4 默认路由、固定代理出口和网络/休眠事件。TUN、全局模式和代理出口成功都不是客户端流量覆盖证明。详见 [IPv6 与代理预检边界](docs/IPV6.md)。

## 构建与测试

Windows x64 正式安装包位于本仓库 **Releases → v0.6.0**，下载 `*-setup.exe` 并按同页 `SHA256SUMS.txt` 核对 SHA-256。安装包未代码签名，可能触发 Windows 安全提示；安装/升级/卸载尚未完成实机验收。开发构建仍在 **Actions → Desktop contracts and build → Artifacts** 中以 `ClaudeGuard-Windows-x64-preview` 保存。

已知限制：出现过代理身份最终复核失败、重新检查后通过的情况，根因尚未定位；失败时继续阻断启动，不跳过检查。

工具链版本见 `rust-toolchain.toml`；前端使用 Node.js 26。macOS 需要 Xcode Command Line Tools。没有账号、密钥或发布凭据要求。

```sh
# 先生成前端资源，干净源码目录还没有 dist。
cd apps/desktop
npm ci --ignore-scripts
npm test
npm run build
npm run format:check
cd ../..

cargo test --workspace --locked --features custom-protocol
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked --features custom-protocol -- -D warnings
node --test scripts/export-public.test.mjs
node scripts/export-public.mjs --check

./scripts/desktop.sh build
./scripts/desktop.sh demo
# 真实本机诊断，但后端永久禁止本次进程启动客户端：
./scripts/desktop.sh observe
```

回归测试包含真实 curl 与本机 `127.0.0.1` / `::1` 临时端口的模拟 CONNECT 代理；不访问公网、不做 DNS 查询、不运行 Claude，不修改网络配置。这些测试验证合作模拟客户端的 IPv4/IPv6 代理行为，不替代真实 Claude/Electron/TLS 验收。

`demo` 完全离线；`observe` 在用户确认后只读本机并经指定代理访问固定 Cloudflare；不得用真实 Claude 执行、版本查询、登录或 Anthropic 请求代替测试。

构建结果为 `target/debug/bundle/macos/ClaudeGuard.app`，仅本地 ad-hoc 签名，尚未完成 Developer ID 签名、公证或正式安装器。不会接管旧版安装或 Dock。不要按正式发行包的标准理解此本地预览。

CLI 的 `--check`、`--json`、`--check-for-launch`、`--monitor-check` 都只检测；首次使用未确认配置时直接拒绝，不联网。`--list-network-dependencies` 仅展示捆绑域名清单，没有 HTTP、TLS 或 DNS 探测。

## 工程与协作

```text
apps/desktop/src/             界面、交互、脱敏报告
apps/desktop/src-tauri/src/   IPC、配置、通知、窗口及实例生命周期
crates/guard-core/           纯规则、配置、启动门槛、监测状态机
crates/guard-platform/       系统只读采集、代理发现、受限启动适配
crates/guard-service/        检测编排、并发和告警出口
resources/                  离线网络依赖快照
profiles/                   人工配置示例（不作为默认）
```

见 [架构](docs/ARCHITECTURE.md)、[首次配置](docs/ONBOARDING.md)、[隐私说明](docs/PRIVACY.md)、[通知诊断](docs/NOTIFICATION_DIAGNOSTICS.md)、[贡献约定](CONTRIBUTING.md)、[安全报告](SECURITY.md) 和 [发布状态](docs/RELEASE.md)。

公开源码由 `public-files.json` 精确列举；`node scripts/export-public.mjs --check` 检查公开文件，`node scripts/export-public.mjs --destination /path/to/new/claudeguard` 导出到新目录。不会初始化 Git、上传数据或发布代码。

## 许可证

项目源码与原创图标采用 [Apache-2.0](LICENSE)。第三方依赖分别适用其自身许可，见 [NOTICE](NOTICE) 与 [第三方记录](THIRD_PARTY_NOTICES.md)。
