# ClaudeGuard 架构

依赖从 React 界面经类型化 Tauri IPC 到 guard-service；guard-core 只包含类型、纯规则、启动意图和监测状态机，guard-platform 承载系统副作用。

- `guard-core`：稳定 RuleId、Profile、策略版本、Report、单次启动意图与时效、通知证据、恢复状态和处理指引。显示文案不控制授权；`Check::blocks_launch` 统一决定预检阻断影响，`CheckView` 仅序列化这个计算结果，输入的同名字段不能覆盖后端判断。CLI/Desktop 及模拟客户端复用 `proxy::ClientProxy` 纯配置。
- `guard-platform`：Mac 受限命令和原生检查、静态验签、网络与生命周期事件、启动适配。Windows 用系统 API 只读检查当前用户代理、监听进程、IPv4 默认路由、DNS 配置和时区，复用共享预检及轮询；启动始终禁用。
- `guard-service`：完整检查顺序、配置冻结、Controller、锁和代际、独立时效维护、进度快照、共享告警 sink。
- `desktop`：持久化设置、人工确认、同用户单实例、通知/托盘/窗口、UI。前端不持有启动票据，导入报告不能授权。

## 权限与失败语义

新安装空配置不能执行检查或启动。坏配置不能静默回退。完整检查无论成功与否都必须具有可信配置；演示和仅观察进程由后端禁止真实启动，不因前端参数或配置修改解除。

只有明确手动操作才能建立一次性启动意图。完整报告与实际出口采样均须新鲜，最后提交时再次检查代际、时限、进程与应用身份；后台恢复、报告或 UI 不能生成意图。

检查器只接受固定系统可执行白名单，无通用 shell、URL、下载器或规则执行接口。配置中的代理程序路径只用于身份比对，不作为检测命令运行。客户端静态签名校验保留官方 team ID 与 Desktop bundle ID；这些是身份约束，不是个人偏好。

## 监测

本地与出口采样分开节流。独立心跳负责过期判定，不等待阻塞网络；迟到采样以代际丢弃。路径/休眠事件撤销旧许可。异常保持采样并去重，连续本地观测和两次实际出口采样跨度至少 30 秒后可确认观察恢复，仍需完整复检才能授权新启动。

## 验收

核心、服务和平台使用注入 Clock、Platform、CommandRunner、ApplicationRegistry 与 NotificationSink。默认离线/回环，无账号和真实客户端。界面使用 jsdom/Testing Library；原生 GUI 验收只能使用 demo 或用户授权的 observe-only。

Windows 固定 Cloudflare 探测使用内置根证书的 Rustls，应用只连接已确认的 IPv4 回环 HTTP 代理，不在本地解析目标域名、不跟随重定向。Windows 实机验收、Authenticode、原生通知、路径事件、启动适配和 WSL 尚未完成；客户端签名和实例在观察报告中标明未验证。

## 代理预检范围

当前策略版本 3、模式 `proxy_preflight`。IPv6 `Warning` 为非阻断覆盖提醒，未知仍保持 Warning，不改成 Pass；其他 Warning、Fail、Skipped 与核心项非 Pass 均阻断。报告必须包含全部唯一规则、有效基线、未过期出口证据及手动启动意图。网络异常不能由覆盖提醒掩盖。

IPv6 的读取/解析失败只影响该覆盖观察，不阻止固定代理诊断。IPv6 观察不进入代理本地基线；完整扫描刷新快照，后台不宣称逐周期检测 IPv6 流量。系统代理、进程、IPv4 默认路由、出口和已有网络事件的失效保护保留。代理预检是合作客户端配置及链路检查，不是应用级强制网络隔离。

`proxy_transport_contract` 使用真实 curl 与仅回环的假代理/IPv4、IPv6 上游。共享生产代理环境、固定回环解析、服务端连接计数及可达性对照验证未回退；不使用生产 Cloudflare 探测的任意 URL 覆写接口，不放松 TLS，也不运行真实客户端。
