//! OS authorization is distinct from the user's preference and submission.
pub mod delivery;
pub mod diagnostics;
use serde::Serialize;
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationAction {
    Enable,
    EnableReminders,
    Refresh,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Authorization {
    #[default]
    Unknown,
    NotDetermined,
    Denied,
    Authorized,
    Provisional,
}
impl Authorization {
    pub fn from_macos(value: isize) -> Self {
        match value {
            0 => Self::NotDetermined,
            1 => Self::Denied,
            2 => Self::Authorized,
            3 => Self::Provisional,
            _ => Self::Unknown,
        }
    }
    pub fn may_present(self) -> bool {
        matches!(self, Self::Authorized | Self::Provisional)
    }
    pub fn may_request(self, explicit_enable: bool) -> bool {
        explicit_enable && self == Self::NotDetermined
    }
    pub fn action(self, enabled: bool) -> NotificationAction {
        match self {
            Self::NotDetermined => NotificationAction::Enable,
            Self::Authorized | Self::Provisional if !enabled => NotificationAction::EnableReminders,
            _ => NotificationAction::Refresh,
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Unknown => "正在读取系统通知权限；当前仅状态栏提醒",
            Self::NotDetermined => "系统通知未开启，点击开启；当前仅声音与状态栏提醒",
            Self::Denied => "系统通知已被拒绝，请在系统设置中允许；当前仅声音与状态栏提醒",
            Self::Authorized | Self::Provisional => "系统通知已授权（横幅和声音由系统设置控制）",
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permissions_are_never_assumed_or_requested_implicitly() {
        for raw in [-1, 0, 1, 2, 3, 99] {
            let auth = Authorization::from_macos(raw);
            assert!(!auth.may_request(false));
            assert_eq!(auth.may_request(true), raw == 0);
            assert_eq!(auth.may_present(), raw == 2 || raw == 3);
        }
    }
    #[test]
    fn button_action_follows_os_authorization_and_separate_app_preference() {
        assert!(Authorization::Authorized.action(true) == NotificationAction::Refresh);
        assert!(Authorization::Provisional.action(true) == NotificationAction::Refresh);
        assert!(Authorization::Authorized.action(false) == NotificationAction::EnableReminders);
        assert!(Authorization::NotDetermined.action(false) == NotificationAction::Enable);
        assert!(Authorization::Denied.action(true) == NotificationAction::Refresh);
        assert!(Authorization::Unknown.action(false) == NotificationAction::Refresh);
    }
}
