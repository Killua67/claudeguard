use crate::AppState;
use std::{
    sync::{atomic::Ordering, Arc},
    thread,
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
struct RuntimeSink<'a>(&'a AppState);
impl guard_service::alerts::NotificationSink for RuntimeSink<'_> {
    fn notify(&self, notice: guard_core::monitoring::Notice) {
        let enabled = self
            .0
            .settings
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .notifications_enabled;
        self.0.notifications.post(notice, enabled);
    }
}
pub fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_menu = MenuItem::with_id(app, "show", "显示检查窗口", true, Some("CmdOrCtrl+0"))?;
    let scan_menu = MenuItem::with_id(app, "scan", "重新检测", true, Some("CmdOrCtrl+R"))?;
    let hide_menu = MenuItem::with_id(app, "hide", "收起窗口", true, Some("CmdOrCtrl+W"))?;
    let quit_menu = MenuItem::with_id(app, "quit", "退出守护工具", true, Some("CmdOrCtrl+Q"))?;
    let separator = PredefinedMenuItem::separator(app)?;
    let application = Submenu::with_items(
        app,
        crate::branding::APP_NAME,
        true,
        &[&show_menu, &scan_menu, &hide_menu, &separator, &quit_menu],
    )?;
    let edit = Submenu::with_items(
        app,
        "编辑",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("撤销"))?,
            &PredefinedMenuItem::redo(app, Some("重做"))?,
            &PredefinedMenuItem::cut(app, Some("剪切"))?,
            &PredefinedMenuItem::copy(app, Some("复制"))?,
            &PredefinedMenuItem::paste(app, Some("粘贴"))?,
            &PredefinedMenuItem::select_all(app, Some("全选"))?,
        ],
    )?;
    app.set_menu(Menu::with_items(app, &[&application, &edit])?)?;
    app.on_menu_event(handle_menu);
    let show = MenuItem::with_id(app, "show", "显示检查窗口", true, None::<&str>)?;
    let recheck = MenuItem::with_id(app, "scan", "重新检测", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出守护工具", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &recheck, &quit])?;
    TrayIconBuilder::with_id("guard-tray")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("ClaudeGuard · 环境观察")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .build(app)?;
    Ok(())
}
fn handle_menu(app: &tauri::AppHandle, event: MenuEvent) {
    match event.id.as_ref() {
        "show" => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        "hide" => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
        }
        "scan" => {
            let app = app.clone();
            let state = app.state::<Arc<AppState>>().inner().clone();
            thread::spawn(move || {
                let _ = app.emit(
                    "guard://action",
                    serde_json::json!({"kind":"info","message":"正在重新检查…"}),
                );
                let confirmed = state
                    .settings
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .require_confirmed();
                let result = if let Some(error) = &state.settings_error {
                    Err(error.clone())
                } else if let Err(error) = confirmed {
                    Err(error)
                } else {
                    state
                        .controller
                        .scan_with_progress(&|view| {
                            let _ = app.emit("guard://state", state.decorate_snapshot(view));
                        })
                        .map(|_| ())
                        .map_err(|e| e.detail())
                };
                let (kind, message) = match result {
                    Ok(()) => ("success", "本次检测已完成".to_string()),
                    Err(error) => ("error", error),
                };
                let _ = app.emit(
                    "guard://action",
                    serde_json::json!({"kind":kind,"message":message}),
                );
            });
        }
        "quit" => app.exit(0),
        _ => {}
    }
}
pub fn start(app: tauri::AppHandle) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    thread::spawn(move || {
        guard_platform::background::run_with_activity(state.demo, || {
            let mut worker: Option<thread::JoinHandle<()>> = None;
            let mut badge = None;
            let mut ticks = 0_u64;
            while !state.stop.load(Ordering::Acquire) {
                state.controller.heartbeat();
                // UI events remain responsive during bounded OS/network work.
                if worker.as_ref().is_none_or(thread::JoinHandle::is_finished) {
                    if let Some(job) = worker.take() {
                        let _ = job.join();
                    }
                    let controller = state.controller.clone();
                    worker = Some(thread::spawn(move || controller.tick()));
                }
                let snapshot = state.snapshot();
                let next_badge = !snapshot.checks.is_empty() && snapshot.requires_recheck;
                if badge != Some(next_badge) {
                    badge = Some(next_badge);
                    #[cfg(target_os = "macos")]
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.set_badge_label(next_badge.then(|| "!".into()));
                    }
                }
                ticks += 1;
                if ticks.is_multiple_of(30) {
                    state.notifications.refresh();
                }
                let _ = app.emit("guard://state", &snapshot);
                if let Some(tray) = app.tray_by_id("guard-tray") {
                    let status = if state.demo {
                        "离线演示"
                    } else if snapshot.issue.is_some() {
                        "环境有变化"
                    } else if snapshot.monitor_active {
                        "后台观察中"
                    } else {
                        "等待检测"
                    };
                    let _ = tray.set_tooltip(Some(format!("ClaudeGuard · {status}")));
                    // A persistent visible fallback when system banners are disabled.
                    let _ = tray.set_title(if snapshot.issue.is_some() {
                        Some("!")
                    } else {
                        None::<&str>
                    });
                }
                guard_service::alerts::deliver_pending(&state.controller, &RuntimeSink(&state));
                thread::sleep(Duration::from_secs(1));
            }
        });
    });
}
