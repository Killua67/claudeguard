//! Thin desktop shell: typed IPC, application lifecycle and local UI events.
mod branding;
mod commands;
mod diagnostics;
mod instance;
mod mode;
mod notifications;
mod runtime;
mod settings;
use guard_platform::{demo::DemoPlatform, Platform, SystemClock};
use guard_service::controller::Controller;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::Manager;
struct AppState {
    controller: Arc<Controller>,
    _instance: Option<instance::Lease>,
    settings: Mutex<settings::Settings>,
    settings_path: Option<PathBuf>,
    settings_error: Option<String>,
    demo: bool,
    launch_allowed: bool,
    notifications: Arc<notifications::Notifications>,
    native_error: Mutex<Option<String>>,
    stop: AtomicBool,
}
impl AppState {
    fn snapshot(&self) -> guard_service::state::Snapshot {
        self.decorate_snapshot(self.controller.snapshot())
    }
    fn decorate_snapshot(
        &self,
        mut view: guard_service::state::Snapshot,
    ) -> guard_service::state::Snapshot {
        let confirmed = self
            .settings
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .setup_confirmed;
        view.setup_required = !confirmed;
        if !confirmed {
            view.can_launch = false;
            view.requires_recheck = true;
            view.launch_message = Some("请先识别并确认本机代理".into());
        }
        let notification_enabled = self
            .settings
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .notifications_enabled;
        view.notification_diagnostics = Some(self.notifications.diagnostics(notification_enabled));
        view.notification_status = Some(commands::notification_text(self));
        view.notification_action = Some(
            self.notifications.action(
                self.settings
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .notifications_enabled,
            ),
        );
        if let Some(error) = self
            .native_error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            view.can_launch = false;
            view.requires_recheck = true;
            view.launch_message = Some(error.clone());
        }
        if let Some(error) = &self.settings_error {
            view.can_launch = false;
            view.launch_message = Some(format!(
                "{error}；启动已禁用。请检查本工具设置，原文件未覆盖。"
            ));
        }
        view
    }
}
fn platform(demo: bool) -> Arc<dyn Platform> {
    if demo {
        return Arc::new(DemoPlatform::paced());
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(guard_platform::macos::MacPlatform::default())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(guard_platform::windows::WindowsPlatform)
    }
}
pub fn run() {
    let mode = match mode::parse(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2)
        }
    };
    if let Some(code) = diagnostics::run(&mode) {
        std::process::exit(code);
    }
    let demo_first_run = matches!(mode, mode::Mode::DemoFirstRun);
    let demo = matches!(mode, mode::Mode::Demo | mode::Mode::DemoFirstRun);
    let lease = if demo {
        None
    } else {
        match diagnostics::settings_path()
            .and_then(|path| instance::acquire(path.parent().ok_or("设置目录无效")?))
        {
            Ok(file) => Some(file),
            Err(error) => {
                eprintln!("{error}");
                return;
            }
        }
    };
    let launch_allowed =
        cfg!(target_os = "macos") && !demo && !matches!(mode, mode::Mode::ObserveOnly);
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::snapshot,
            commands::scan,
            commands::configure,
            commands::discover_proxy,
            commands::launch,
            commands::enable_notifications,
            commands::notification_status,
            commands::refresh_notifications,
            commands::test_notification,
            commands::confirm_test_notification,
            commands::hide_main
        ])
        .setup(move |app| {
            let path = if demo {
                None
            } else {
                Some(app.path().app_config_dir()?.join("settings.json"))
            };
            let loaded = path.as_deref().map(settings::load).unwrap_or_else(|| {
                Ok(if demo_first_run {
                    settings::Settings::default()
                } else {
                    settings::Settings::demo()
                })
            });
            let (saved, error) = match loaded {
                Ok(s) => (s, None),
                Err(e) => (settings::Settings::default(), Some(e)),
            };
            let controller = Arc::new(
                if !saved.setup_confirmed {
                    Ok(Controller::unconfigured(
                        platform(demo),
                        Arc::new(SystemClock::default()),
                        demo,
                        launch_allowed,
                    ))
                } else if launch_allowed || demo {
                    Controller::new(
                        platform(demo),
                        Arc::new(SystemClock::default()),
                        saved.profile.clone(),
                        demo,
                    )
                } else {
                    Controller::observer(
                        platform(false),
                        Arc::new(SystemClock::default()),
                        saved.profile.clone(),
                    )
                }
                .map_err(std::io::Error::other)?,
            );
            let notify_app = app.handle().clone();
            let show_app = app.handle().clone();
            let notifications = notifications::Notifications::new(
                demo,
                Arc::new(move |job| {
                    let _ = notify_app.run_on_main_thread(job);
                }),
                Arc::new(move || {
                    let app = show_app.clone();
                    let _ = show_app.run_on_main_thread(move || {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    });
                }),
            );
            let state = Arc::new(AppState {
                controller,
                _instance: lease,
                settings: Mutex::new(saved),
                settings_path: path,
                settings_error: error,
                demo,
                launch_allowed,
                notifications,
                native_error: Mutex::new(None),
                stop: AtomicBool::new(false),
            });
            app.manage(state.clone());
            state.notifications.initialize();
            #[cfg(target_os = "macos")]
            if !demo {
                let weak = Arc::downgrade(&state.controller);
                if let Err(error) = guard_platform::macos_events::start(Arc::new(move |reason| {
                    if let Some(controller) = weak.upgrade() {
                        controller.invalidate(reason);
                    }
                })) {
                    *state.native_error.lock().unwrap_or_else(|p| p.into_inner()) =
                        Some(error.into());
                }
            }
            runtime::setup_tray(app)?;
            runtime::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
            if let tauri::WindowEvent::Focused(true) = event {
                if let Some(state) = window.try_state::<Arc<AppState>>() {
                    state.notifications.refresh();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("无法初始化本地桌面应用");
    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } = event
        {
            // A user reopening from Dock may show the window; alerts never do.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        if matches!(event, tauri::RunEvent::Exit) {
            let state = app.state::<Arc<AppState>>();
            state.stop.store(true, Ordering::Release);
            state.notifications.stop();
            #[cfg(target_os = "macos")]
            guard_platform::macos_events::stop();
        }
    });
}
