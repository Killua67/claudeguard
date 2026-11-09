//! Minimal desktop lifecycle helpers, separate from network inspection.
pub fn run_with_activity(demo: bool, run: impl FnOnce()) {
    #[cfg(target_os = "macos")]
    let activity = if demo {
        None
    } else {
        use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
        Some(
            NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                &NSString::from_str("后台代理环境观察"),
            ),
        )
    };
    #[cfg(not(target_os = "macos"))]
    let _ = demo;
    run();
    #[cfg(target_os = "macos")]
    if let Some(activity) = activity {
        // Token comes from the matching beginActivity call above.
        unsafe { objc2_foundation::NSProcessInfo::processInfo().endActivity(&activity) };
    }
}
pub fn problem_sound() {
    #[cfg(target_os = "macos")]
    objc2_app_kit::NSBeep();
    // Windows notification delivery and sound require Windows acceptance tests.
}
