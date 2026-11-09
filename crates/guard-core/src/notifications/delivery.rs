//! Local self-test evidence. Submission, system acceptance and human receipt are
//! separate facts; none is a launch authority or an account safety claim.
use serde::Serialize;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPhase {
    Pending,
    Submitted,
    Accepted,
    Rejected,
    Cancelled,
    Unconfirmed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Receipt {
    Unconfirmed,
    Received,
    NotSeen,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestRecord {
    pub sequence: u64,
    pub requested_at: u64,
    pub phase: TestPhase,
    pub receipt: Receipt,
    pub in_notification_center: Option<bool>,
    pub center_queried_at: Option<u64>,
}
#[derive(Default)]
pub struct TestTracker {
    next: u64,
    pub last: Option<TestRecord>,
}
impl TestTracker {
    pub fn begin(&mut self, now: u64) -> Result<u64, &'static str> {
        if self
            .last
            .as_ref()
            .is_some_and(|r| matches!(r.phase, TestPhase::Pending | TestPhase::Submitted))
        {
            return Err("测试通知正在发送，请稍候");
        }
        self.next = self.next.wrapping_add(1);
        self.last = Some(TestRecord {
            sequence: self.next,
            requested_at: now,
            phase: TestPhase::Pending,
            receipt: Receipt::Unconfirmed,
            in_notification_center: None,
            center_queried_at: None,
        });
        Ok(self.next)
    }
    pub fn claim_submission(&mut self, sequence: u64) -> bool {
        let Some(record) = self
            .last
            .as_mut()
            .filter(|r| r.sequence == sequence && r.phase == TestPhase::Pending)
        else {
            return false;
        };
        record.phase = TestPhase::Submitted;
        true
    }
    pub fn complete(&mut self, sequence: u64, accepted: bool) -> bool {
        let Some(record) = self.last.as_mut().filter(|r| {
            r.sequence == sequence
                && (matches!(r.phase, TestPhase::Submitted | TestPhase::Unconfirmed)
                    || (!accepted && r.phase == TestPhase::Pending))
        }) else {
            return false;
        };
        record.phase = if accepted {
            TestPhase::Accepted
        } else {
            TestPhase::Rejected
        };
        true
    }
    pub fn expire(&mut self, sequence: u64) {
        if let Some(record) = self.last.as_mut().filter(|r| r.sequence == sequence) {
            record.phase = match record.phase {
                TestPhase::Pending => TestPhase::Cancelled,
                TestPhase::Submitted => TestPhase::Unconfirmed,
                phase => phase,
            };
        }
    }
    pub fn cancel_pending(&mut self) {
        if let Some(id) = self.last.as_ref().map(|r| r.sequence) {
            self.expire(id);
        }
    }
    pub fn center_result(&mut self, sequence: u64, present: bool, queried_at: u64) -> bool {
        let Some(r) = self.last.as_mut().filter(|r| {
            r.sequence == sequence
                && matches!(r.phase, TestPhase::Accepted | TestPhase::Unconfirmed)
        }) else {
            return false;
        };
        r.in_notification_center = Some(present);
        r.center_queried_at = Some(queried_at);
        true
    }
    pub fn confirm(&mut self, sequence: u64, received: bool) -> Result<(), &'static str> {
        let Some(r) = self.last.as_mut().filter(|r| {
            r.sequence == sequence
                && matches!(r.phase, TestPhase::Accepted | TestPhase::Unconfirmed)
        }) else {
            return Err("这条测试尚未提交或已经过期，请核对当前测试记录");
        };
        r.receipt = if received {
            Receipt::Received
        } else {
            Receipt::NotSeen
        };
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_acceptance_does_not_claim_human_receipt_or_center_presence() {
        let mut t = TestTracker::default();
        let id = t.begin(1000).unwrap();
        assert!(t.claim_submission(id));
        assert!(t.complete(id, true));
        let r = t.last.as_ref().unwrap();
        assert_eq!(r.receipt, Receipt::Unconfirmed);
        assert_eq!(r.in_notification_center, None);
        assert!(t.center_result(id, true, 1));
        assert_eq!(t.last.as_ref().unwrap().receipt, Receipt::Unconfirmed);
        t.confirm(id, false).unwrap();
        assert_eq!(t.last.as_ref().unwrap().receipt, Receipt::NotSeen);
    }
    #[test]
    fn timeout_arbitrates_before_submit_and_late_callbacks_cannot_overwrite_a_new_test() {
        let mut t = TestTracker::default();
        let old = t.begin(0).unwrap();
        t.expire(old);
        assert!(!t.claim_submission(old));
        assert!(!t.complete(old, true));
        let new = t.begin(1).unwrap();
        assert!(!t.complete(old, true));
        assert!(!t.center_result(old, true, 1));
        assert!(t.confirm(old, true).is_err());
        assert!(t.claim_submission(new));
        t.expire(new);
        assert_eq!(t.last.as_ref().unwrap().phase, TestPhase::Unconfirmed);
        assert!(t.complete(new, true));
        assert_eq!(t.last.as_ref().unwrap().phase, TestPhase::Accepted);
    }
    #[test]
    fn duplicates_and_cancelled_tests_cannot_be_confirmed() {
        let mut t = TestTracker::default();
        let id = t.begin(0).unwrap();
        assert!(t.begin(1).is_err());
        assert!(t.confirm(id, true).is_err());
        t.cancel_pending();
        assert!(!t.claim_submission(id));
        assert!(t.confirm(id, false).is_err());
    }
    #[test]
    fn acceptance_requires_prior_submission_but_permission_rejection_does_not() {
        let mut tracker = TestTracker::default();
        let id = tracker.begin(0).unwrap();
        assert!(!tracker.complete(id, true));
        assert_eq!(tracker.last.as_ref().unwrap().phase, TestPhase::Pending);
        assert!(tracker.complete(id, false));
        assert_eq!(tracker.last.as_ref().unwrap().phase, TestPhase::Rejected);
    }
}
