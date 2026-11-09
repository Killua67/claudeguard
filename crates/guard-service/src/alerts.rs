//! Shared runtime boundary. Production and fixture sinks consume each notice once.
use crate::controller::Controller;
use guard_core::monitoring::Notice;
pub trait NotificationSink {
    fn notify(&self, notice: Notice);
}
pub fn deliver_pending(controller: &Controller, sink: &dyn NotificationSink) -> usize {
    let notices = controller.take_notices();
    let count = notices.len();
    for notice in notices {
        sink.notify(notice);
    }
    count
}
