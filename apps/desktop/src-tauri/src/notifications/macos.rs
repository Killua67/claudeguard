use super::*;
use block2::{DynBlock, RcBlock};
use guard_core::notifications::diagnostics::{AlertStyle, Setting};
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject},
    AnyThread, DefinedClass,
};
use objc2_foundation::{NSArray, NSError, NSString};
use objc2_user_notifications::*;
use std::{cell::RefCell, ptr::NonNull};
struct Ivars {
    service: Arc<Notifications>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "CEGNotificationDelegate"]
    #[ivars=Ivars]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn foreground(
            &self,
            _: &UNUserNotificationCenter,
            _: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::Sound,));
        }
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn received(
            &self,
            _: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            let action = response.actionIdentifier();
            if action.to_string() == unsafe { UNNotificationDefaultActionIdentifier }.to_string() {
                (self.ivars().service.show)();
            }
            completion.call(());
        }
    }
);
impl Delegate {
    fn new(service: Arc<Notifications>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { service });
        unsafe { msg_send![super(this), init] }
    }
}
thread_local! {static DELEGATE:RefCell<Option<Retained<Delegate>>>=const{RefCell::new(None)};}
pub fn install(service: Arc<Notifications>) {
    let delegate = Delegate::new(service);
    UNUserNotificationCenter::currentNotificationCenter()
        .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DELEGATE.with(|slot| *slot.borrow_mut() = Some(delegate));
}
pub fn remove_delegate() {
    UNUserNotificationCenter::currentNotificationCenter().setDelegate(None);
    DELEGATE.with(|slot| {
        slot.borrow_mut().take();
    });
}
pub fn refresh(service: Arc<Notifications>, explicit: bool) {
    refresh_center(service.clone());
    let revision = service.query_revision.fetch_add(1, Ordering::SeqCst) + 1;
    let completion = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // UN owns the settings for this callback. Only scalar authorization escapes.
        let view = read_settings(unsafe { settings.as_ref() });
        let auth = view.authorization;
        if !service.accept_settings(revision, view) {
            return;
        }
        if auth.may_request(explicit) {
            let service = service.clone();
            let done = RcBlock::new(move |_: Bool, error: *mut NSError| {
                if service.finish_authorization(revision, !error.is_null()) {
                    service.refresh();
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .requestAuthorizationWithOptions_completionHandler(
                    UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                    &done,
                );
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getNotificationSettingsWithCompletionHandler(&completion);
}
fn read_settings(settings: &UNNotificationSettings) -> SystemSettings {
    SystemSettings {
        authorization: Authorization::from_macos(settings.authorizationStatus().0),
        alert: Setting::from_macos(settings.alertSetting().0),
        sound: Setting::from_macos(settings.soundSetting().0),
        notification_center: Setting::from_macos(settings.notificationCenterSetting().0),
        lock_screen: Setting::from_macos(settings.lockScreenSetting().0),
        alert_style: AlertStyle::from_macos(settings.alertStyle().0),
        queried_at: Some(unix_ms()),
    }
}
pub fn refresh_center(service: Arc<Notifications>) {
    let Some(test) = service
        .latest_test()
        .filter(|r| matches!(r.phase, TestPhase::Accepted | TestPhase::Unconfirmed))
    else {
        return;
    };
    let id = service.test_request_id(test.sequence);
    let revision = service.center_revision.fetch_add(1, Ordering::SeqCst) + 1;
    let queried_at = unix_ms();
    let done = RcBlock::new(move |notifications: NonNull<NSArray<UNNotification>>| {
        let present = unsafe { notifications.as_ref() }
            .iter()
            .any(|n| n.request().identifier().to_string() == id);
        service.center_result(test.sequence, revision, present, queried_at);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getDeliveredNotificationsWithCompletionHandler(&done);
}
pub fn send_test(service: Arc<Notifications>, sequence: u64) -> Result<(), String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let query = service.clone();
    let revision = service.query_revision.fetch_add(1, Ordering::SeqCst) + 1;
    let completion = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        let settings = read_settings(unsafe { settings.as_ref() });
        let allowed = settings.authorization.may_present();
        query.accept_settings(revision, settings);
        if !allowed {
            query.complete_test(sequence, false);
            let _ = sender.try_send(Err("系统未授权通知；请在系统设置中允许后再测试".to_string()));
            return;
        }
        let service = query.clone();
        let dispatch = service.dispatch.clone();
        let sender = sender.clone();
        dispatch(Box::new(move || {
            // Claim and timeout cancellation arbitrate under the same mutex.
            if !service.claim_test(sequence) {
                let _ = sender.try_send(Err("测试通知已取消；未提交新通知".to_string()));
                return;
            }
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str("ClaudeGuard · 测试通知"));
            content.setBody(&NSString::from_str(
                "这是一条本地测试通知。未启动 Claude，也未请求 Anthropic。",
            ));
            content.setSound(Some(&UNNotificationSound::defaultSound()));
            let id = NSString::from_str(&service.test_request_id(sequence));
            let request =
                UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
            let done = RcBlock::new(move |error: *mut NSError| {
                let accepted = error.is_null();
                service.complete_test(sequence, accepted);
                if accepted {
                    refresh_center(service.clone());
                }
                let _ = sender.try_send(if accepted {
                    Ok(())
                } else {
                    Err("系统未接受测试通知；请查看通知诊断".to_string())
                });
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .addNotificationRequest_withCompletionHandler(&request, Some(&done));
        }));
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getNotificationSettingsWithCompletionHandler(&completion);
    let result = receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap_or_else(|_| Err("未及时确认系统接收结果；不会自动重发".to_string()));
    service.expire_test(sequence);
    result
}
pub fn post(service: Arc<Notifications>, notice: guard_core::monitoring::Notice, revision: u64) {
    let completion = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        let auth = Authorization::from_macos(unsafe { settings.as_ref() }.authorizationStatus().0);
        let service = service.clone();
        let dispatch = service.dispatch.clone();
        let notice = notice.clone();
        // All replacement/submission operations run serially on the main queue.
        dispatch(Box::new(move || {
            if !service.active.load(Ordering::Acquire)
                || service.notice_revision.load(Ordering::SeqCst) != revision
            {
                return;
            }
            // Permission refresh owns the displayed authorization. A late
            // delivery callback cannot overwrite a newer Settings response.
            let problem = matches!(notice, guard_core::monitoring::Notice::Problem(_));
            if !auth.may_present() {
                if problem {
                    guard_platform::background::problem_sound();
                }
                return;
            }
            let (title, body) = match notice {
                guard_core::monitoring::Notice::Problem(reason) => (
                    "网络环境有变化",
                    format!(
                        "{reason}。{} 本工具未中断网络。",
                        guard_core::guidance::issue(&reason)
                            .steps
                            .first()
                            .copied()
                            .unwrap_or("请查看工具中的处理建议。")
                    ),
                ),
                guard_core::monitoring::Notice::Recovered => (
                    "代理采样已恢复",
                    "后台采样恢复稳定；启动放行仍需完整复检，不会自动启动客户端。".into(),
                ),
            };
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&format!("ClaudeGuard · {title}")));
            content.setBody(&NSString::from_str(&body));
            if problem {
                content.setSound(Some(&UNNotificationSound::defaultSound()));
            }
            let id = NSString::from_str("runtime-environment");
            let ids = NSArray::from_slice(&[&*id]);
            let center = UNUserNotificationCenter::currentNotificationCenter();
            center.removePendingNotificationRequestsWithIdentifiers(&ids);
            center.removeDeliveredNotificationsWithIdentifiers(&ids);
            let request =
                UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
            let completion = RcBlock::new(move |error: *mut NSError| {
                let failed = !error.is_null();
                if service.finish_delivery(revision, failed) && failed && problem {
                    let dispatch = service.dispatch.clone();
                    dispatch(Box::new(guard_platform::background::problem_sound));
                }
            });
            center.addNotificationRequest_withCompletionHandler(&request, Some(&completion));
        }));
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getNotificationSettingsWithCompletionHandler(&completion);
}
