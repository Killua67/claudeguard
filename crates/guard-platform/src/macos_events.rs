//! Read-only Network.framework path notifications and local workspace events.
//! FFI declarations match the SDK's path_monitor.h/path.h/interface.h/endpoint.h.
use block2::{DynBlock, RcBlock};
use dispatch2::{DispatchQueue, DispatchRetained};
use guard_core::events::{PathIdentity, PathTracker};
use objc2::{
    rc::Retained,
    runtime::{Bool, NSObjectProtocol, ProtocolObject},
};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSSystemClockDidChangeNotification};
use std::{
    cell::RefCell,
    ffi::{c_char, c_void, CStr},
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
#[link(name = "Network", kind = "framework")]
extern "C" {
    fn nw_path_monitor_create() -> *mut c_void;
    fn nw_path_monitor_set_update_handler(m: *mut c_void, handler: &DynBlock<dyn Fn(*mut c_void)>);
    fn nw_path_monitor_set_queue(m: *mut c_void, queue: *mut c_void);
    fn nw_path_monitor_start(m: *mut c_void);
    fn nw_path_monitor_cancel(m: *mut c_void);
    fn nw_release(object: *mut c_void);
    fn nw_path_get_status(path: *mut c_void) -> i32;
    fn nw_path_has_ipv4(path: *mut c_void) -> bool;
    fn nw_path_has_ipv6(path: *mut c_void) -> bool;
    fn nw_path_has_dns(path: *mut c_void) -> bool;
    fn nw_path_is_expensive(path: *mut c_void) -> bool;
    fn nw_path_is_constrained(path: *mut c_void) -> bool;
    fn nw_path_enumerate_interfaces(
        path: *mut c_void,
        handler: &DynBlock<dyn Fn(*mut c_void) -> Bool + '_>,
    );
    fn nw_interface_get_index(interface: *mut c_void) -> u32;
    fn nw_interface_get_name(interface: *mut c_void) -> *const c_char;
    fn nw_path_enumerate_gateways(
        path: *mut c_void,
        handler: &DynBlock<dyn Fn(*mut c_void) -> Bool + '_>,
    );
    fn nw_endpoint_get_address(endpoint: *mut c_void) -> *const libc::sockaddr;
}
type Observer = (
    Retained<NSNotificationCenter>,
    Retained<ProtocolObject<dyn NSObjectProtocol>>,
);
struct Events {
    monitor: NonNull<c_void>,
    _queue: DispatchRetained<DispatchQueue>,
    active: Arc<AtomicBool>,
    observers: Vec<Observer>,
}
thread_local! {static EVENTS:RefCell<Option<Events>>=const {RefCell::new(None)};}
impl Drop for Events {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        // A copied handler may finish after cancel. It owns only Arc data and checks
        // active before publishing; no Rust pointer or borrowed native path escapes.
        unsafe {
            nw_path_monitor_cancel(self.monitor.as_ptr());
            nw_release(self.monitor.as_ptr());
        }
        for (center, token) in &self.observers {
            unsafe {
                center.removeObserver((**token).as_ref());
            }
        }
    }
}
pub fn start(changed: Arc<dyn Fn(&'static str) + Send + Sync>) -> Result<(), &'static str> {
    stop();
    let monitor =
        NonNull::new(unsafe { nw_path_monitor_create() }).ok_or("原生网络路径监测不可用")?;
    let active = Arc::new(AtomicBool::new(true));
    let queue = DispatchQueue::new("guard.native-path", None);
    let tracker = Arc::new(Mutex::new(PathTracker::default()));
    let (flag, callback) = (active.clone(), changed.clone());
    let update = RcBlock::new(move |path: *mut c_void| {
        if path.is_null() || !flag.load(Ordering::Acquire) {
            return;
        }
        let interfaces = RefCell::new(Vec::new());
        let enumerate = RcBlock::new(|interface: *mut c_void| {
            // Network owns each interface for the duration of this noescape callback.
            let name = unsafe { nw_interface_get_name(interface) };
            if !name.is_null() {
                interfaces.borrow_mut().push((
                    unsafe { nw_interface_get_index(interface) },
                    unsafe { CStr::from_ptr(name) }
                        .to_string_lossy()
                        .into_owned(),
                ));
            }
            Bool::YES
        });
        let gateways = RefCell::new(Vec::new());
        let enumerate_gateways = RcBlock::new(|endpoint: *mut c_void| {
            let address = unsafe { nw_endpoint_get_address(endpoint) };
            if let Some(identity) = unsafe { gateway_identity(address) } {
                gateways.borrow_mut().push(identity);
            }
            Bool::YES
        });
        unsafe {
            nw_path_enumerate_interfaces(path, &enumerate);
            nw_path_enumerate_gateways(path, &enumerate_gateways);
        }
        drop(enumerate);
        drop(enumerate_gateways);
        let next = PathIdentity {
            status: unsafe { nw_path_get_status(path) },
            flags: unsafe {
                [
                    nw_path_has_ipv4(path),
                    nw_path_has_ipv6(path),
                    nw_path_has_dns(path),
                    nw_path_is_expensive(path),
                    nw_path_is_constrained(path),
                ]
            },
            interfaces: interfaces.into_inner(),
            gateways: gateways.into_inner(),
        };
        if tracker.lock().is_ok_and(|mut t| t.changed(next)) && flag.load(Ordering::Acquire) {
            callback("网络路径发生变化，旧检测已失效");
        }
    });
    unsafe {
        nw_path_monitor_set_update_handler(monitor.as_ptr(), &update);
        nw_path_monitor_set_queue(
            monitor.as_ptr(),
            DispatchRetained::as_ptr(&queue).as_ptr().cast(),
        );
        nw_path_monitor_start(monitor.as_ptr());
    }
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    let default = NSNotificationCenter::defaultCenter();
    let mut observers = Vec::new();
    for (center, name, reason) in [
        (
            &workspace,
            unsafe { NSWorkspaceWillSleepNotification },
            "系统即将休眠，旧检测已失效",
        ),
        (
            &workspace,
            unsafe { NSWorkspaceDidWakeNotification },
            "系统已唤醒，旧检测已失效",
        ),
        (
            &default,
            unsafe { NSSystemClockDidChangeNotification },
            "系统时钟发生变化，旧检测已失效",
        ),
    ] {
        let (flag, callback) = (active.clone(), changed.clone());
        let block = RcBlock::new(move |_: NonNull<NSNotification>| {
            if flag.load(Ordering::Acquire) {
                callback(reason)
            }
        });
        // Closure captures thread-safe Arc state only; notification objects are unused.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        observers.push((center.clone(), token));
    }
    EVENTS.with(|slot| {
        *slot.borrow_mut() = Some(Events {
            monitor,
            _queue: queue,
            active,
            observers,
        })
    });
    Ok(())
}
pub fn stop() {
    EVENTS.with(|slot| {
        slot.borrow_mut().take();
    });
}

// Normalize only address and scope. sockaddr padding and unused flow labels are
// not part of route identity and may change without any path change.
unsafe fn gateway_identity(address: *const libc::sockaddr) -> Option<Vec<u8>> {
    if address.is_null() {
        return None;
    }
    let length = unsafe { (*address).sa_len } as usize;
    let family = unsafe { (*address).sa_family } as i32;
    match family {
        libc::AF_INET if length >= std::mem::size_of::<libc::sockaddr_in>() => {
            let value = unsafe { address.cast::<libc::sockaddr_in>().read_unaligned() };
            let mut result = vec![4];
            result.extend(value.sin_addr.s_addr.to_ne_bytes());
            Some(result)
        }
        libc::AF_INET6 if length >= std::mem::size_of::<libc::sockaddr_in6>() => {
            let value = unsafe { address.cast::<libc::sockaddr_in6>().read_unaligned() };
            let mut result = vec![6];
            result.extend(value.sin6_addr.s6_addr);
            result.extend(value.sin6_scope_id.to_ne_bytes());
            Some(result)
        }
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gateway_padding_is_ignored_but_real_address_changes_are_visible() {
        let mut a: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        a.sin_len = std::mem::size_of_val(&a) as u8;
        a.sin_family = libc::AF_INET as u8;
        a.sin_addr.s_addr = 0x01020304;
        let identity = unsafe { gateway_identity((&a as *const libc::sockaddr_in).cast()) };
        a.sin_zero = [42; 8];
        a.sin_port = 80;
        assert_eq!(identity, unsafe {
            gateway_identity((&a as *const libc::sockaddr_in).cast())
        });
        a.sin_addr.s_addr += 1;
        assert_ne!(identity, unsafe {
            gateway_identity((&a as *const libc::sockaddr_in).cast())
        });
        a.sin_len = 1;
        assert!(unsafe { gateway_identity((&a as *const libc::sockaddr_in).cast()) }.is_none());
    }
}
