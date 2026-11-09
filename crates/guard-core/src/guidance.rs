//! Presentation only. Advice cannot change severity, launch intent or policy.
use crate::domain::{RuleId, Status};
use serde::Serialize;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Guidance {
    pub title: &'static str,
    pub explanation: &'static str,
    pub steps: Vec<&'static str>,
}
pub fn issue(reason: &str) -> Guidance {
    let (title, explanation, steps) = if reason.contains("休眠")
        || reason.contains("唤醒")
        || reason.contains("过期")
        || reason.contains("时钟")
    {
        (
            "系统恢复后需要重新确认环境",
            "休眠、时钟变化或采样空窗会使旧检查失效。",
            vec![
                "等待网络恢复，确认代理已连接。",
                "后台会继续采样；启动前点击“重新检测”。",
            ],
        )
    } else if reason.contains("IPv6") {
        return check(RuleId::Ipv6, Status::Warning).expect("IPv6 warning has guidance");
    } else if reason.contains("出口 IP") || reason.contains("出口地区") {
        (
            "代理出口发生变化",
            "采样出口与上一次稳定观察不同；这条记录不判断账号状态。",
            vec![
                "核对代理节点与出口地区偏好。",
                "保持节点稳定，等待后台完成恢复观察。",
                "恢复提醒不解除启动锁，使用启动入口前仍需完整复检。",
            ],
        )
    } else if reason.contains("出口") || reason.contains("curl") || reason.contains("TLS") {
        (
            "暂时无法确认代理出口",
            "检查器通过当前代理访问固定 Cloudflare 未取得有效结果；没有回退直连。",
            vec![
                "先确认代理上游连接正常。",
                "等待网络稳定后重新检测；无需向 Anthropic 反复探测。",
            ],
        )
    } else if reason.contains("监听")
        || reason.contains("代理")
        || reason.contains("进程")
        || reason.contains("路由")
        || reason.contains("路径")
    {
        (
            "本地代理或网络路径需要检查",
            "代理监听、进程身份、系统代理或路由改变，旧检查暂时不能作为启动依据。",
            vec![
                "确认代理程序正在运行、已连接，且地址与本工具配置相符。",
                "如刚切换网络或重启代理，等待稳定后点击“重新检测”。",
            ],
        )
    } else if reason.contains("完整检查") || reason.contains("检测期间") {
        (
            "完整检查尚未放行",
            "有检查未完成或检测期间环境改变。",
            vec![
                "在检测列表中选择橙色或红色项目查看原因。",
                "处理后重新检测；后台恢复不会自动启动客户端。",
            ],
        )
    } else {
        (
            "环境暂时无法确认",
            "本次观察不足以维持原来的启动放行状态。",
            vec!["查看当前检查详情与最近事件。", "确认代理稳定后重新检测。"],
        )
    };
    Guidance {
        title,
        explanation,
        steps,
    }
}
pub fn check(id: RuleId, status: Status) -> Option<Guidance> {
    if matches!(status, Status::Pass | Status::Info) {
        return None;
    }
    let (title, explanation, steps) = match id {
        RuleId::SystemProxy | RuleId::ProxyBypass | RuleId::ProxyOwner => (
            "核对本机代理配置",
            "这项本地前置检查未通过。",
            vec![
                "确认 HTTP/HTTPS 代理已开启，地址与本工具一致。",
                "核对自动代理、绕过规则及端口监听程序；本工具不会自动修改它们。",
            ],
        ),
        RuleId::Ipv6 => (
            "IPv6 覆盖未验证 · 非阻断提醒",
            "当前存在可用公网 IPv6 地址、外网路由，或本地证据不完整；不能据此断言已发生泄漏。",
            vec![
                "先查看本项的地址状态、活动接口与路由结果；公网 IPv6 本身是正常网络能力。",
                "核对代理的 IPv6 支持；TUN、全局模式或 Cloudflare 成功均不能单独证明覆盖。",
                "保留 IPv6；本项只作覆盖提醒，不影响代理预检放行，不要求修改网络配置。",
                "代理预检通过只证明已检查的指定代理链路可用，不保证真实客户端全部流量走代理。",
                "代理关闭、旁路配置、监听身份、HTTPS/TLS 或最终复核异常仍会阻止启动。",
            ],
        ),
        RuleId::Timezone => (
            "核对系统与进程时区",
            "检查时区有效性、一致性和你保存的偏好，不要求与出口国家相同。",
            vec![
                "核对 macOS 日期与时间中的时区。",
                "确认配置偏好后重新检测；无需手动改动系统时钟。",
            ],
        ),
        RuleId::ClientSignature => (
            "客户端文件身份未确认",
            "仅静态验证文件签名，不执行客户端来判断安装是否正常。",
            vec!["核对安装来源与目标文件；此工具不自动安装或运行客户端。"],
        ),
        RuleId::Environment => (
            "启动环境存在冲突",
            "已发现可能改变上游、证书或进程行为的非认证环境设置。",
            vec![
                "按检查详情核对冲突键名。",
                "确认使用方式后重新检查；不读取账号或密钥来排障。",
            ],
        ),
        _ => {
            return Some(issue(match id {
                RuleId::ExitRegion | RuleId::ExitStability => "出口 IP 或出口地区发生变化",
                RuleId::HttpsTls => "出口 TLS 无法确认",
                RuleId::Route => "网络路由无法确认",
                RuleId::Clock => "时钟参考无法确认",
                _ => "完整检查未通过",
            }))
        }
    };
    Some(Guidance {
        title,
        explanation,
        steps,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advice_does_not_invent_leaks_or_change_passed_checks() {
        assert!(check(RuleId::SystemProxy, Status::Pass).is_none());
        assert!(check(RuleId::Ipv6, Status::Warning)
            .unwrap()
            .explanation
            .contains("不能据此断言"));
        assert!(issue("出口 IP 发生变化")
            .steps
            .iter()
            .any(|s| s.contains("不解除启动锁")));
        assert!(issue("Cloudflare curl 28 出口超时")
            .explanation
            .contains("没有回退直连"));
    }
}
