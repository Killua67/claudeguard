use super::{delivery::*, Authorization};
use serde::Serialize;
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    #[default]
    Unknown,
    Unsupported,
    Disabled,
    Enabled,
}
impl Setting {
    pub fn from_macos(raw: isize) -> Self {
        match raw {
            0 => Self::Unsupported,
            1 => Self::Disabled,
            2 => Self::Enabled,
            _ => Self::Unknown,
        }
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertStyle {
    #[default]
    Unknown,
    None,
    Banner,
    Alert,
}
impl AlertStyle {
    pub fn from_macos(raw: isize) -> Self {
        match raw {
            0 => Self::None,
            1 => Self::Banner,
            2 => Self::Alert,
            _ => Self::Unknown,
        }
    }
}
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSettings {
    pub authorization: Authorization,
    pub alert: Setting,
    pub sound: Setting,
    pub notification_center: Setting,
    pub lock_screen: Setting,
    pub alert_style: AlertStyle,
    pub queried_at: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationDiagnostics {
    pub app_enabled: bool,
    pub service_active: bool,
    pub demo: bool,
    pub application_name: &'static str,
    pub application_id: &'static str,
    pub system: SystemSettings,
    pub last_test: Option<TestRecord>,
    pub suggestions: Vec<&'static str>,
}
impl NotificationDiagnostics {
    pub fn suggestions(&self) -> Vec<&'static str> {
        let mut result = Vec::new();
        if self.demo {
            return vec!["当前为离线演示，显示模拟状态，不会发送系统通知。"];
        }
        if !self.app_enabled {
            result.push("本工具后台提醒尚未启用；如需异常提醒，点击“启用提醒”。");
        }
        match self.system.authorization {
            Authorization::Unknown => result.push("权限尚未读到，请刷新状态；未知状态不能算通过。"),
            Authorization::NotDetermined => {
                result.push("请点击“开启通知”，由你决定是否授予系统权限。")
            }
            Authorization::Denied => {
                result.push("请在系统设置 → 通知中找到 ClaudeLauncher Next，允许通知后再刷新。")
            }
            _ => {}
        }
        if self.system.authorization.may_present()
            && (self.system.alert == Setting::Disabled
                || self.system.alert_style == AlertStyle::None)
        {
            result.push(
                "系统允许通知，但当前横幅/提醒显示已关闭；请在系统设置中选择“横幅”或“提醒”。",
            );
        }
        if self.system.notification_center == Setting::Disabled {
            result.push("通知中心显示已关闭；即使提交成功，也可能无法在通知中心找到记录。");
        }
        if self.system.sound == Setting::Disabled {
            result.push("通知声音已关闭；测试通知可能没有提示音。");
        }
        if let Some(test) = &self.last_test {
            if test.receipt == Receipt::NotSeen {
                result.push(if test.in_notification_center==Some(true){"系统通知中心已找到这条测试通知；横幅未出现时请检查提醒样式、专注模式和屏幕共享期间的通知设置。"}else{"你已标记“没看到”；系统接收不等于你收到，请核对当前应用的通知显示设置后再测试。"});
            }
        }
        result.push("专注模式、锁屏或屏幕共享可能影响展示；此处没有读取或修改这些状态。最终以你实际看到为准。");
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authorized_with_hidden_alerts_is_explained_without_claiming_delivery() {
        let view = NotificationDiagnostics {
            app_enabled: true,
            service_active: true,
            demo: false,
            application_name: "fixture",
            application_id: "fixture",
            system: SystemSettings {
                authorization: Authorization::Authorized,
                alert: Setting::Disabled,
                alert_style: AlertStyle::None,
                ..Default::default()
            },
            last_test: None,
            suggestions: vec![],
        };
        assert!(view
            .suggestions()
            .iter()
            .any(|s| s.contains("横幅/提醒显示已关闭")));
        assert!(view
            .suggestions()
            .iter()
            .any(|s| s.contains("没有读取或修改")));
    }
}
