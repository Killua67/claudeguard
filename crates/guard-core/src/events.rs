//! Native events carry comparable observations only; no IPs leave this boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct PathIdentity {
    pub status: i32,
    pub flags: [bool; 5],
    pub interfaces: Vec<(u32, String)>,
    pub gateways: Vec<Vec<u8>>,
}
#[derive(Default)]
pub struct PathTracker(Option<PathIdentity>);
impl PathTracker {
    pub fn changed(&mut self, mut next: PathIdentity) -> bool {
        next.interfaces.sort();
        next.interfaces.dedup();
        next.gateways.sort();
        next.gateways.dedup();
        let changed = self.0.as_ref().is_some_and(|old| *old != next);
        self.0 = Some(next);
        changed
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn path() -> PathIdentity {
        PathIdentity {
            status: 1,
            flags: [true, false, true, false, false],
            interfaces: vec![(4, "en0".into()), (5, "utun1".into())],
            gateways: vec![vec![4, 192, 0, 2, 1]],
        }
    }
    #[test]
    fn first_observation_and_order_only_changes_do_not_invalidate() {
        let mut tracker = PathTracker::default();
        assert!(!tracker.changed(path()));
        let mut same = path();
        same.interfaces.reverse();
        same.interfaces.push((4, "en0".into()));
        assert!(!tracker.changed(same));
        let mut changed = path();
        changed.flags[1] = true;
        assert!(tracker.changed(changed));
        assert!(tracker.changed(path()));
        let mut changed = path();
        changed.gateways = vec![vec![4, 192, 0, 2, 2]];
        assert!(tracker.changed(changed));
    }
}
