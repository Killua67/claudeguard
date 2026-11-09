use crate::{settings, AppState};
use guard_core::{config::Profile, domain::ClientTarget};
use guard_service::state::Snapshot;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub fn snapshot(state: State<'_, Arc<AppState>>) -> Snapshot {
    state.snapshot()
}
#[tauri::command]
pub async fn scan(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Snapshot, String> {
    if let Some(error) = &state.settings_error {
        return Err(format!("{error}；未使用默认配置发起检测"));
    }
    state
        .settings
        .lock()
        .map_err(|_| "设置状态不可用")?
        .require_confirmed()?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state
            .controller
            .scan_with_progress(&|view| {
                let _ = app.emit("guard://state", state.decorate_snapshot(view));
            })
            .map_err(|e| e.detail())?;
        Ok(state.snapshot())
    })
    .await
    .map_err(|_| "检查任务未完成".to_string())?
}
#[tauri::command]
pub async fn discover_proxy(
    state: State<'_, Arc<AppState>>,
) -> Result<guard_core::discovery::Discovery, String> {
    let demo = state.demo;
    tauri::async_runtime::spawn_blocking(move || crate::platform(demo).discover_proxy())
        .await
        .map_err(|_| "代理识别任务未完成".into())
}
#[tauri::command]
pub fn configure(profile: Profile, state: State<'_, Arc<AppState>>) -> Result<Snapshot, String> {
    configure_state(profile, &state)
}
fn configure_state(profile: Profile, state: &AppState) -> Result<Snapshot, String> {
    if let Some(error) = &state.settings_error {
        return Err(format!("{error}；请先处理本工具的设置文件"));
    }
    profile.validate().map_err(str::to_string)?;
    let mut saved = state.settings.lock().map_err(|_| "设置状态不可用")?;
    let mut next = saved.clone();
    next.profile = profile.clone();
    next.setup_confirmed = true;
    state.controller.configure_with(profile, || {
        if let Some(path) = &state.settings_path {
            settings::save(path, &next)
        } else {
            Ok(())
        }
    })?;
    *saved = next;
    drop(saved);
    Ok(state.snapshot())
}
#[tauri::command]
pub async fn launch(
    target: ClientTarget,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Snapshot, String> {
    let state = state.inner().clone();
    state
        .settings
        .lock()
        .map_err(|_| "设置状态不可用")?
        .require_confirmed()?;
    if !state.launch_allowed {
        return Err("本次只允许检测，后端已禁止执行客户端".into());
    }
    if state.settings_error.is_some() {
        return Err("设置无法读取，不能启动客户端".into());
    }
    if state
        .native_error
        .lock()
        .map_err(|_| "原生事件状态不可用")?
        .is_some()
    {
        return Err("原生网络监测不可用，不能启动客户端".into());
    }
    // Only this explicit UI command reaches launch. Pick the folder before scanning.
    let directory = if target == ClientTarget::Cli {
        let (sender, mut receiver) = tauri::async_runtime::channel(1);
        app.dialog()
            .file()
            .set_title("选择 CLI 工作目录")
            .pick_folder(move |path| {
                let _ = sender.try_send(path);
            });
        let path = receiver.recv().await.flatten().ok_or("已取消启动")?;
        Some(path.into_path().map_err(|_| "工作目录不可用")?)
    } else {
        None
    };
    let checker = std::env::current_exe().map_err(|_| "无法确认检查器路径")?;
    tauri::async_runtime::spawn_blocking(move || {
        state
            .controller
            .launch(target, &checker, directory.as_deref())
            .map_err(|e| e.detail())?;
        Ok(state.snapshot())
    })
    .await
    .map_err(|_| "启动任务未完成".to_string())?
}
pub fn notification_text(state: &AppState) -> String {
    let enabled = state
        .settings
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .notifications_enabled;
    state.notifications.preference_status(enabled)
}
#[tauri::command]
pub fn refresh_notifications(state: State<'_, Arc<AppState>>) -> String {
    // A read-only refresh never prompts, writes preferences or sends a test alert.
    state.notifications.refresh();
    "正在刷新系统通知状态…".into()
}
#[tauri::command]
pub fn notification_status(state: State<'_, Arc<AppState>>) -> String {
    notification_text(&state)
}
#[tauri::command]
pub fn enable_notifications(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    if state.demo {
        return Err("离线演示不发送通知".into());
    }
    if state.settings_error.is_some() {
        return Err("设置无法读取，不能保存通知偏好".into());
    }
    let mut saved = state.settings.lock().map_err(|_| "设置状态不可用")?;
    let mut next = saved.clone();
    next.notifications_enabled = true;
    if let Some(path) = &state.settings_path {
        settings::save(path, &next)?;
    }
    *saved = next;
    drop(saved);
    state.notifications.enable()
}
#[tauri::command]
pub async fn test_notification(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let notifications = state.notifications.clone();
    tauri::async_runtime::spawn_blocking(move || {
        notifications.send_test()?;
        Ok("测试通知已交给系统，请以实际收到为准。".to_string())
    })
    .await
    .map_err(|_| "测试通知任务未完成".to_string())?
}
#[tauri::command]
pub fn confirm_test_notification(
    sequence: u64,
    received: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<Snapshot, String> {
    state.notifications.confirm_test(sequence, received)?;
    Ok(state.snapshot())
}
#[tauri::command]
pub fn hide_main(app: tauri::AppHandle) -> Result<(), String> {
    app.get_webview_window("main")
        .ok_or("主窗口不存在")?
        .hide()
        .map_err(|_| "收起窗口失败".into())
}

#[cfg(test)]
fn fixture_profile() -> guard_core::config::Profile {
    guard_core::config::Profile {
        schema_version: 1,
        proxy_url: "http://127.0.0.1:17890".into(),
        expected_country: Some("US".into()),
        expected_proxy_executable: "/fixture/proxy".into(),
        non_mainland_timezone: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configure_returns_a_snapshot_without_relocking_its_own_settings_guard() {
        use std::sync::{atomic::AtomicBool, Mutex};
        let state = AppState {
            _instance: None,
            controller: Arc::new(
                guard_service::controller::Controller::new(
                    Arc::new(guard_platform::demo::DemoPlatform::default()),
                    Arc::new(guard_platform::SystemClock::default()),
                    fixture_profile(),
                    true,
                )
                .unwrap(),
            ),
            settings: Mutex::new(settings::Settings::demo()),
            settings_path: None,
            settings_error: None,
            demo: true,
            launch_allowed: false,
            notifications: crate::notifications::Notifications::new(
                true,
                Arc::new(|_| {}),
                Arc::new(|| {}),
            ),
            native_error: Mutex::new(None),
            stop: AtomicBool::new(false),
        };
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let profile = Profile {
                proxy_url: "http://127.0.0.1:9999".into(),
                ..fixture_profile()
            };
            send.send(configure_state(profile, &state).unwrap())
                .unwrap();
        });
        let view = receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("configure must return without a nested settings lock");
        assert_eq!(view.profile.proxy_url, "http://127.0.0.1:9999");
        assert!(!view.launch_allowed);
        worker.join().unwrap();
    }
}
