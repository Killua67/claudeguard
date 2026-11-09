# ClaudeGuard 桌面使用与排查

## Windows x64 安装和首次检测

1. 登录 GitHub，在 **Actions → Desktop contracts and build** 打开最近一次成功的 `main` 构建。从 **Artifacts** 下载 `ClaudeGuard-Windows-x64-preview`，解压后运行 `*-setup.exe`。这是未签名的测试安装包，产物过期后可从一次更新的成功构建下载。 [构建列表](https://github.com/Killua67/claudeguard/actions/workflows/desktop-checks.yml) · [GitHub 下载说明](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/download-workflow-artifacts)
2. 打开 Clash Verge 或其他代理工具，启用**系统代理**，确认当前用户的手动代理指向 `127.0.0.1` 上的 HTTP 或混合端口。HTTP 与 HTTPS 应使用同一个端口。这里用 `7897` 举例，请以代理软件实际显示为准。可在 Windows **设置 → 网络和 Internet → 代理** 核对手动代理、自动检测和设置脚本。此版要求关闭自动检测和 PAC。 [微软的代理设置说明](https://support.microsoft.com/en-us/windows/experience/connectivity-networking/use-a-proxy-server-in-windows)
3. 打开 ClaudeGuard，核对识别到的地址和完整程序路径，再点 **确认代理并开始检测**。地址示例为 `http://127.0.0.1:7897`。路径应对应监听端口的代理核心，可能和代理软件主界面的路径不同。地区和时区偏好可留空。
4. 查看总览和异常详情。改过代理端口或重装代理软件后，在 ClaudeGuard 里点 **重新识别**，核对候选后点 **保存并重新检查**。重新识别只更新草稿。

Windows 版只做代理预检，客户端启动按钮不可用。首次识别只读取本机信息，确认后才会通过指定代理请求固定的 Cloudflare HTTPS 地址。

### Windows 找不到监听进程

在 PowerShell 里把 `7897` 换成实际端口，查看监听地址和进程编号。 [Microsoft Learn 命令参考](https://learn.microsoft.com/en-us/powershell/module/nettcpip/get-nettcpconnection)

```powershell
$port = 7897
Get-NetTCPConnection -State Listen -LocalPort $port |
    Select-Object LocalAddress, LocalPort, OwningProcess
```

此版要求明确的 `127.0.0.1` 监听。如果只有 `0.0.0.0`，先核对代理工具的监听地址和「允许局域网连接」设置。拿到 `OwningProcess` 后，可将下面的 `1234` 换成该编号查看程序路径。

```powershell
Get-Process -Id 1234 | Select-Object Id, Path
```

### 拿到 Windows 测试机后可以验证什么

| 场景 | 预期结果 |
| --- | --- |
| 手动代理与确认的地址、端口、程序路径一致 | 本地检查通过后，继续检查 HTTPS、出口和短时稳定性 |
| 在代理工具里关闭系统代理后重新检查 | 系统代理检查失败，跳过出口探测 |
| 在可调整的测试环境中临时加入一条不支持的旁路，例如 `*.example.com` | 旁路检查失败，跳过出口探测。测试结束后恢复原设置，再跑一次完整检查 |

## macOS 常见问题

### 系统代理显示开启，但 ClaudeGuard 识别不到

先运行 `scutil --proxy`，核对当前生效的 HTTP 和 HTTPS 是否都开启，是否同时指向 `127.0.0.1` 的同一端口。Clash Verge 的规则模式、全局模式和 TUN 开关并不能代替这一步。系统代理设置改变后，回到 ClaudeGuard 重新识别。

如果端口正确，但 `lsof` 查不到，代理核心可能由系统服务运行，普通用户的 `lsof` 输出可能漏掉其监听。先用下面的只读命令交叉核对，无需先用管理员身份运行 ClaudeGuard。

```sh
port=7897
lsof -nP -iTCP:"$port" -sTCP:LISTEN
netstat -anv -p tcp -f inet |
  awk -v endpoint="127.0.0.1.$port" '$4 == endpoint && $6 == "LISTEN"'
```

如果 `netstat` 显示 `进程名:PID`，可将实际 PID 代入 `ps -p 1234 -o comm=`，核对程序路径。这个仓库的 Mac 版本在 `lsof` 没有找到明确回环监听时会尝试用 `netstat` 确认唯一 PID，再核对路径。如果仍然失败，也可确认 Dock 打开的确实是包含这项修复的版本。

### 系统代理通过，但旁路规则异常

系统代理旁路是交给操作系统的例外列表，Clash 规则页则决定流量到达代理之后如何转发。先去 Clash Verge **设置 → 系统代理旁的齿轮 → 代理绕过设置** 检查，别先改订阅规则。Mac 上也可以看 `scutil --proxy` 的 `ExceptionsList`。如果列表里出现一个你没单独配过的域名，单凭这一条无法判断它来自默认配置、旧设置还是别的工具。

两端都接受明确的本机例外和代码允许的几项本地范围，例如 `localhost`、`127.0.0.1` 和 `192.168.0.0/16`。Windows 的 `127.*`、`192.168.*` 属于文本通配，当前版本会拒绝。Mac 还会检查系统是否单独启用了「排除简单主机名」。修改自己管理的设置后，回到总览重跑完整检查。 [WinINet 旁路规则说明](https://learn.microsoft.com/en-us/windows/win32/wininet/enabling-internet-functionality#listing-the-proxy-bypass)

## 两端都适用的排查顺序

- **先看第一项异常。** 系统代理、旁路和监听进程是出口检查的前置项。前置失败时，后面的「未执行」表示跳过，无需同时排查这些项目。
- **再看 HTTPS 和出口。** 本地检查通过后仍失败，可以核对代理软件的当前节点和连接状态。如果设置了固定出口地区，再比较实际检测结果与该偏好。
- **单独看待 IPv6 覆盖提醒。** 只有 IPv6 覆盖未验证时，它是非阻断提醒，可以保留 IPv6。预检通过说明已采样的代理链路可用，Claude 的实际 IPv4 和 IPv6 流量覆盖仍未验证。 [IPv6 检查范围](IPV6.md)
- **分享脱敏报告。** 优先用总览的「复制报告」，附上操作系统和版本、代理软件版本、失败项及复现步骤。公开提交前，移除个人路径、账号信息和未经处理的代理输出。
