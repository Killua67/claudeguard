//! Same-user GUI lease. No argv forwarding, sockets or mode promotion.
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};
pub struct Lease {
    file: File,
}
impl Drop for Lease {
    fn drop(&mut self) {
        // Explicit unlock also releases a lock whose descriptor is transiently
        // inherited by another thread's fork before that child reaches exec.
        let _ = self.file.unlock();
    }
}
pub fn acquire(root: &Path) -> Result<Lease, String> {
    fs::create_dir_all(root).map_err(|_| "无法建立本工具目录")?;
    let path = root.join("desktop.lock");
    if path.symlink_metadata().is_ok_and(|m| !m.is_file()) {
        return Err("实例锁文件类型异常".into());
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|_| "无法建立本工具实例锁")?;
    file.try_lock().map_err(|_| {
        "环境守护工具已在运行，或实例锁不可用；请使用现有窗口。模式不会被切换，也不会转发启动参数。"
    })?;
    Ok(Lease { file })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_child() {
        if let Some(root) = std::env::var_os("CLAUDEGUARD_TEST_LOCK_DIR") {
            assert!(acquire(Path::new(&root)).is_err());
        }
    }
    #[test]
    fn cross_process_lease_rejects_second_process_without_starting_a_gui() {
        let root = std::env::temp_dir().join(format!(
            "guard-cross-instance-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let lease = acquire(&root).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "instance::tests::lease_child"])
            .env_clear()
            .env("CLAUDEGUARD_TEST_LOCK_DIR", &root)
            .status()
            .unwrap();
        assert!(status.success());
        drop(lease);
        assert!(acquire(&root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn second_gui_cannot_acquire_shared_configuration_until_first_exits() {
        let root = std::env::temp_dir().join(format!(
            "guard-instance-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = acquire(&root).unwrap();
        assert!(acquire(&root).is_err());
        drop(first);
        assert!(acquire(&root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
}
