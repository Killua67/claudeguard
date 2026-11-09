use crate::domain::{ClientTarget, Report};
#[derive(Default)]
pub struct LaunchGate {
    pending: Option<ClientTarget>,
}
impl LaunchGate {
    pub fn request(&mut self, target: ClientTarget) -> Result<(), &'static str> {
        if self.pending.is_some() {
            return Err("已有启动复检正在进行");
        };
        self.pending = Some(target);
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.pending = None;
    }
    pub fn consume(&mut self, report: &Report, now: u64) -> Result<ClientTarget, &'static str> {
        let target = self.pending.take().ok_or("没有手动启动意图")?;
        if !report.fresh(now) {
            return Err("启动复检未完整通过或已经过期");
        };
        Ok(target)
    }
}
#[derive(Debug, Clone, Copy)]
pub struct LaunchTicket {
    pub revision: u64,
    pub checked_at: u64,
}
impl LaunchTicket {
    pub fn permits(self, revision: u64, healthy: bool, now: u64) -> bool {
        healthy
            && revision == self.revision
            && now >= self.checked_at
            && now - self.checked_at < 20_000
    }
}
