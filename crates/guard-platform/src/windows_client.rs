use super::{
    process::process_owner,
    trust::{self, VerifiedExecutable},
};
use crate::*;
use std::{
    fs::{File, OpenOptions},
    mem::size_of,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_NO_MORE_FILES, FILETIME, HANDLE, INVALID_HANDLE_VALUE,
    },
    Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        Ioctl::FSCTL_GET_REPARSE_POINT,
        Threading::{
            GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        IO::DeviceIoControl,
    },
};

const FAMILY: &str = "Claude_pzs8sxrjxfjjc";

pub(super) struct Executable {
    pub(super) image: VerifiedExecutable,
    pub(super) entry: PathBuf,
    _alias: Option<File>,
}

fn cli_candidate(home: &Path) -> PathBuf {
    home.join(".local").join("bin").join("claude.exe")
}

fn desktop_candidates(root: &Path) -> [(PathBuf, bool); 2] {
    [
        (
            root.join("Microsoft")
                .join("WindowsApps")
                .join(FAMILY)
                .join("claude-desktop.exe"),
            true,
        ),
        (root.join("AnthropicClaude").join("claude.exe"), false),
    ]
}

fn cli_path() -> Option<PathBuf> {
    let path = cli_candidate(&PathBuf::from(std::env::var_os("USERPROFILE")?));
    path.symlink_metadata().ok().map(|_| path)
}

fn desktop_path() -> Option<(PathBuf, bool)> {
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    desktop_candidates(&root)
        .into_iter()
        .find(|(path, _)| path.symlink_metadata().is_ok())
}

pub(super) fn availability() -> ClientAvailability {
    ClientAvailability {
        cli: cli_path().is_some(),
        desktop: desktop_path().is_some(),
    }
}

fn alias_target(bytes: &[u8], program_files: &Path) -> Result<PathBuf, Fault> {
    let tag = bytes.get(..4).ok_or(Fault::Client)?;
    let length = bytes.get(4..6).ok_or(Fault::Client)?;
    let length = usize::from(u16::from_le_bytes(
        length.try_into().map_err(|_| Fault::Client)?,
    ));
    if u32::from_le_bytes(tag.try_into().map_err(|_| Fault::Client)?) != 0x8000001b
        || length + 8 != bytes.len()
        || bytes.get(8..12) != Some(&3_u32.to_le_bytes()[..])
        || !(length - 4).is_multiple_of(2)
    {
        return Err(Fault::Client);
    }
    let units: Vec<_> = bytes[12..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let text = String::from_utf16(&units).map_err(|_| Fault::Client)?;
    let fields: Vec<_> = text.split('\0').collect();
    if fields.len() != 5
        || fields[0] != FAMILY
        || fields[1] != format!("{FAMILY}!Claude")
        || fields[3] != "0"
        || !fields[4].is_empty()
    {
        return Err(Fault::Client);
    }
    let target = PathBuf::from(fields[2]);
    let package_root = program_files.join("WindowsApps");
    let relative = target
        .strip_prefix(&package_root)
        .map_err(|_| Fault::Client)?;
    let parts: Vec<_> = relative.components().collect();
    if parts.len() != 3 || parts[1].as_os_str() != "app" || parts[2].as_os_str() != "Claude.exe" {
        return Err(Fault::Client);
    }
    let package = parts[0].as_os_str().to_str().ok_or(Fault::Client)?;
    if !package.starts_with("Claude_")
        || !package.ends_with("_x64__pzs8sxrjxfjjc")
        || package.contains(['\\', '/', ':'])
    {
        return Err(Fault::Client);
    }
    Ok(target)
}

fn open_alias(path: &Path) -> Result<(File, PathBuf), Fault> {
    let alias = OpenOptions::new()
        .read(true)
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| Fault::Client)?;
    let mut bytes = [0_u8; 16384];
    let mut used = 0;
    if unsafe {
        DeviceIoControl(
            alias.as_raw_handle(),
            FSCTL_GET_REPARSE_POINT,
            std::ptr::null(),
            0,
            bytes.as_mut_ptr().cast(),
            bytes.len() as u32,
            &mut used,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(Fault::Client);
    }
    let root = PathBuf::from(std::env::var_os("ProgramFiles").ok_or(Fault::Client)?);
    let target = alias_target(bytes.get(..used as usize).ok_or(Fault::Client)?, &root)?;
    Ok((alias, target))
}

pub(super) fn verify(target: ClientTarget) -> Result<Executable, Fault> {
    let (entry, alias) = match target {
        ClientTarget::Cli => (cli_path().ok_or(Fault::Client)?, false),
        ClientTarget::Desktop => desktop_path().ok_or(Fault::Client)?,
    };
    if alias {
        let (alias, target) = open_alias(&entry)?;
        Ok(Executable {
            image: trust::verify(&target)?,
            entry,
            _alias: Some(alias),
        })
    } else {
        let image = trust::verify(&entry)?;
        Ok(Executable {
            entry: image.path.clone(),
            image,
            _alias: None,
        })
    }
}

pub(super) fn signatures() -> Check {
    let clients = availability();
    let ok = (!clients.cli || verify(ClientTarget::Cli).is_ok())
        && (!clients.desktop || verify(ClientTarget::Desktop).is_ok());
    Check::new(RuleId::ClientSignature, if !clients.cli && !clients.desktop { Status::Info } else if ok { Status::Pass } else { Status::Fail }, "离线静态验证已安装原生客户端的 Authenticode 签名和 Anthropic 发布者；不运行客户端，未联网查询证书撤销")
}

fn process_list() -> Result<Vec<PROCESSENTRY32W>, Fault> {
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(Fault::Client);
    }
    struct Snapshot(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
    let snapshot = Snapshot(handle);
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
        return Err(Fault::Client);
    }
    let mut pids = Vec::new();
    loop {
        pids.push(entry);
        if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
            if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
                return Err(Fault::Client);
            }
            break;
        }
    }
    Ok(pids)
}

fn related_processes() -> Result<Vec<u32>, Fault> {
    let mut pids = Vec::new();
    for entry in process_list()? {
        let units = entry
            .szExeFile
            .split(|c| *c == 0)
            .next()
            .ok_or(Fault::Client)?;
        let name = String::from_utf16(units).map_err(|_| Fault::Client)?;
        if name.eq_ignore_ascii_case("claude.exe")
            || name.eq_ignore_ascii_case("claude-desktop.exe")
        {
            pids.push(entry.th32ProcessID);
        }
    }
    Ok(pids)
}

pub struct GuardParent(HANDLE);
impl Drop for GuardParent {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
impl GuardParent {
    pub fn require_alive(&self) -> Result<(), Fault> {
        let mut code = 0;
        if unsafe { GetExitCodeProcess(self.0, &mut code) } == 0 || code != 259 {
            return Err(Fault::Client);
        }
        Ok(())
    }
}

fn parent_identity_matches(
    parent_image: &str,
    image: &str,
    parent_created: u64,
    child_created: u64,
) -> bool {
    parent_created != 0
        && parent_created <= child_created
        && parent_image == image.strip_prefix(r"\\?\").unwrap_or(image)
}

pub fn guard_parent() -> Result<GuardParent, Fault> {
    let current = process_list()?
        .into_iter()
        .find(|entry| entry.th32ProcessID == std::process::id())
        .ok_or(Fault::Client)?;
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            current.th32ParentProcessID,
        )
    };
    if handle.is_null() {
        return Err(Fault::Client);
    }
    let parent = GuardParent(handle);
    let mut path = [0_u16; 4097];
    let mut length = path.len() as u32;
    let [mut created, mut exit, mut kernel, mut user] = [FILETIME::default(); 4];
    if unsafe { QueryFullProcessImageNameW(parent.0, 0, path.as_mut_ptr(), &mut length) } == 0
        || unsafe { GetProcessTimes(parent.0, &mut created, &mut exit, &mut kernel, &mut user) }
            == 0
    {
        return Err(Fault::Client);
    }
    let path = String::from_utf16(path.get(..length as usize).ok_or(Fault::Client)?)
        .map_err(|_| Fault::Client)?;
    let created = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    let image = std::env::current_exe().map_err(|_| Fault::Client)?;
    let child = process_owner(std::process::id()).map_err(|_| Fault::Client)?;
    if !parent_identity_matches(
        &path,
        image.to_str().ok_or(Fault::Client)?,
        created,
        child.created,
    ) {
        return Err(Fault::Client);
    }
    parent.require_alive()?;
    Ok(parent)
}

pub(super) fn existing() -> Check {
    match related_processes() {
        Ok(pids) => Check::new(
            RuleId::ExistingClients,
            Status::Info,
            format!(
                "观察到 {} 个相关进程；旧进程流量未验证，不自动终止",
                pids.len()
            ),
        ),
        Err(_) => Check::new(
            RuleId::ExistingClients,
            Status::Warning,
            "无法确认 Windows 已有客户端进程",
        ),
    }
}

pub(super) fn ensure_desktop_not_running(executable: &Executable) -> Result<(), Fault> {
    let installed = executable.image.path.parent().ok_or(Fault::Client)?;
    for pid in related_processes()? {
        let owner = process_owner(pid).map_err(|_| Fault::Client)?;
        let path = Path::new(&owner.path);
        let same = path
            .to_string_lossy()
            .eq_ignore_ascii_case(&executable.image.path.to_string_lossy());
        let old_desktop = path.parent().is_some_and(|p| {
            p.to_string_lossy()
                .eq_ignore_ascii_case(&installed.to_string_lossy())
        }) || owner
            .path
            .to_ascii_lowercase()
            .contains("\\anthropicclaude\\")
            || owner
                .path
                .to_ascii_lowercase()
                .contains("\\windowsapps\\claude_");
        if same || old_desktop {
            return Err(Fault::Client);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn alias(family: &str, app: &str, target: &str) -> Vec<u8> {
        let text = format!("{family}\0{app}\0{target}\0{zero}\0", zero = "0");
        let mut payload = 3_u32.to_le_bytes().to_vec();
        payload.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let mut bytes = 0x8000001b_u32.to_le_bytes().to_vec();
        bytes.extend((payload.len() as u16).to_le_bytes());
        bytes.extend([0, 0]);
        bytes.extend(payload);
        bytes
    }
    #[test]
    fn msix_alias_requires_exact_family_application_and_package_path() {
        let path = r"C:\Program Files\WindowsApps\Claude_4.1.0.0_x64__pzs8sxrjxfjjc\app\Claude.exe";
        let app = format!("{FAMILY}!Claude");
        let bytes = alias(FAMILY, &app, path);
        let root = Path::new(r"C:\Program Files");
        assert_eq!(alias_target(&bytes, root).unwrap(), Path::new(path));
        assert!(alias_target(&alias("Other", &app, path), root).is_err());
        assert!(alias_target(&alias(FAMILY, "Other", path), root).is_err());
        for path in [
            r"C:\Temp\Claude.exe",
            r"C:\Program Files\WindowsApps\Other_x64__pzs8sxrjxfjjc\app\Claude.exe",
            r"C:\Program Files\WindowsApps\Claude_4_x64__pzs8sxrjxfjjc\app\Other.exe",
        ] {
            assert!(alias_target(&alias(FAMILY, &app, path), root).is_err());
        }
        for length in 0..bytes.len() {
            assert!(alias_target(&bytes[..length], root).is_err());
        }
    }
    #[test]
    fn replacement_parent_pid_or_different_executable_is_rejected() {
        let image = r"C:\Fixture\guard.exe";
        assert!(parent_identity_matches(image, image, 10, 20));
        assert!(!parent_identity_matches(image, image, 21, 20));
        assert!(!parent_identity_matches(image, image, 0, 20));
        assert!(!parent_identity_matches(
            r"C:\Other\guard.exe",
            image,
            10,
            20
        ));
    }
    #[test]
    fn installation_candidates_use_native_absolute_windows_paths() {
        let root = Path::new(r"C:\Fixture User");
        let cli = cli_candidate(root);
        assert_eq!(cli, Path::new(r"C:\Fixture User\.local\bin\claude.exe"));
        for path in std::iter::once(cli).chain(desktop_candidates(root).map(|(path, _)| path)) {
            assert!(guard_core::config::windows_proxy_path(
                path.to_str().unwrap()
            ));
        }
    }
    #[test]
    fn process_observation_does_not_execute_clients() {
        assert!(related_processes().is_ok());
    }
}
