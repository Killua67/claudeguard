use super::Buffer;
use crate::Fault;
use guard_core::config::windows_proxy_path;
use std::{ffi::c_void, mem::size_of};
use windows_sys::{
    s, w,
    Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, FILETIME,
            HANDLE, STATUS_INFO_LENGTH_MISMATCH,
        },
        Storage::FileSystem::QueryDosDeviceW,
        System::{
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            Threading::{
                GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    },
};

struct Process(HANDLE);
impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Owner {
    pub(super) pid: u32,
    pub(super) path: String,
    pub(super) created: u64,
}

pub(super) fn process_owner(pid: u32) -> Result<Owner, Fault> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return if unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
            kernel_process_owner(pid)
        } else {
            Err(Fault::ProcessIdentity)
        };
    }
    let process = Process(handle);
    let mut path = [0_u16; 4097];
    let mut len = path.len() as u32;
    let [mut created, mut exit, mut kernel, mut user] = [FILETIME::default(); 4];
    // Keep the process handle open while reading both pieces of identity.
    if unsafe { QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut len) } == 0
        || unsafe { GetProcessTimes(process.0, &mut created, &mut exit, &mut kernel, &mut user) }
            == 0
    {
        return Err(Fault::ProcessIdentity);
    }
    let path = String::from_utf16(path.get(..len as usize).ok_or(Fault::ProcessIdentity)?)
        .map_err(|_| Fault::ProcessIdentity)?;
    let created = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    owner(pid, path, created)
}

fn owner(pid: u32, path: String, created: u64) -> Result<Owner, Fault> {
    if created == 0 || !windows_proxy_path(&path) {
        return Err(Fault::ProcessIdentity);
    }
    Ok(Owner { pid, path, created })
}

type QuerySystemInformation = unsafe extern "system" fn(u32, *mut c_void, u32, *mut u32) -> i32;
const SYSTEM_PROCESS_INFORMATION: u32 = 5;
const SYSTEM_PROCESS_ID_INFORMATION: u32 = 88;

#[repr(C)]
struct UnicodeString {
    length: u16,
    capacity: u16,
    buffer: *mut u16,
}

// Prefix of SYSTEM_PROCESS_INFORMATION through UniqueProcessId. The public
// Windows header reserves the three timestamps. The NT layout uses the first
// for creation time, in the same units as GetProcessTimes.
#[repr(C)]
struct ProcessRow {
    next: u32,
    _threads: u32,
    _reserved: [i64; 3],
    created: i64,
    _times: [i64; 2],
    _image: UnicodeString,
    _priority: i32,
    pid: usize,
}

#[repr(C)]
struct ProcessIdInformation {
    pid: usize,
    image: UnicodeString,
}

fn kernel_process_owner(pid: u32) -> Result<Owner, Fault> {
    // The NT queries work for service processes without opening a process
    // handle. Resolve at runtime so an unavailable API can fail closed.
    let module = unsafe { GetModuleHandleW(w!("ntdll.dll")) };
    if module.is_null() {
        return Err(Fault::ProcessIdentity);
    }
    let address = unsafe { GetProcAddress(module, s!("NtQuerySystemInformation")) }
        .ok_or(Fault::ProcessIdentity)?;
    // The named ntdll export has the signature of QuerySystemInformation.
    let query = unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, QuerySystemInformation>(address)
    };
    read_kernel_owner(
        pid,
        || process_created(query, pid),
        || {
            let native_path = process_image(query, pid)?;
            dos_image_path(&native_path, |drive, device| {
                let drive = char::from(drive);
                device_matches(&format!("Global\\{drive}:"), device)
                    && device_matches(&format!("{drive}:"), device)
            })
            .ok_or(Fault::ProcessIdentity)
        },
    )
}

fn read_kernel_owner(
    pid: u32,
    created: impl Fn() -> Result<u64, Fault>,
    image: impl FnOnce() -> Result<String, Fault>,
) -> Result<Owner, Fault> {
    let before = created()?;
    let path = image()?;
    if created()? != before {
        return Err(Fault::Drift);
    }
    owner(pid, path, before)
}

fn process_created(query: QuerySystemInformation, pid: u32) -> Result<u64, Fault> {
    let buffer = Buffer::read(
        256 * 1024,
        |data, size| match unsafe { query(SYSTEM_PROCESS_INFORMATION, data, *size, size) } {
            0 => 0,
            STATUS_INFO_LENGTH_MISMATCH => ERROR_INSUFFICIENT_BUFFER,
            _ => ERROR_ACCESS_DENIED,
        },
        Fault::ProcessIdentity,
    )?;
    find_creation_time(buffer.bytes(), pid).ok_or(Fault::ProcessIdentity)
}

fn find_creation_time(mut bytes: &[u8], pid: u32) -> Option<u64> {
    while bytes.len() >= size_of::<ProcessRow>() {
        // All members are integers or raw pointers. No table pointer is followed.
        let row = unsafe { bytes.as_ptr().cast::<ProcessRow>().read_unaligned() };
        let next = row.next as usize;
        if next != 0
            && (next < size_of::<ProcessRow>() || next > bytes.len() - size_of::<ProcessRow>())
        {
            return None;
        }
        if row.pid == pid as usize {
            return u64::try_from(row.created).ok().filter(|time| *time != 0);
        }
        if next == 0 {
            break;
        }
        bytes = &bytes[next..];
    }
    None
}

fn process_image(query: QuerySystemInformation, pid: u32) -> Result<String, Fault> {
    let mut path = [0_u16; 4097];
    let mut info = ProcessIdInformation {
        pid: pid as usize,
        image: UnicodeString {
            length: 0,
            capacity: size_of_val(&path) as u16,
            buffer: path.as_mut_ptr(),
        },
    };
    let status = unsafe {
        query(
            SYSTEM_PROCESS_ID_INFORMATION,
            (&mut info as *mut ProcessIdInformation).cast(),
            size_of::<ProcessIdInformation>() as u32,
            std::ptr::null_mut(),
        )
    };
    let length = info.image.length as usize;
    if status != 0
        || info.pid != pid as usize
        || info.image.buffer != path.as_mut_ptr()
        || length == 0
        || !length.is_multiple_of(size_of::<u16>())
        || length > usize::from(info.image.capacity)
        || length > size_of_val(&path)
    {
        return Err(Fault::ProcessIdentity);
    }
    String::from_utf16(&path[..length / size_of::<u16>()]).map_err(|_| Fault::ProcessIdentity)
}

fn device_matches(drive: &str, device: &str) -> bool {
    let drive: Vec<_> = drive.encode_utf16().chain([0]).collect();
    let mut target = [0_u16; 1024];
    let length =
        unsafe { QueryDosDeviceW(drive.as_ptr(), target.as_mut_ptr(), target.len() as u32) };
    length != 0
        && target[..length as usize]
            .split(|unit| *unit == 0)
            .next()
            .is_some_and(|current| current.iter().copied().eq(device.encode_utf16()))
}

fn dos_image_path(native_path: &str, matches: impl Fn(u8, &str) -> bool) -> Option<String> {
    let prefix = r"\Device\HarddiskVolume";
    let (volume, relative) = native_path.strip_prefix(prefix)?.split_once('\\')?;
    if volume.is_empty() || !volume.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let device = &native_path[..prefix.len() + volume.len()];
    let mut drives = (b'A'..=b'Z').filter(|drive| matches(*drive, device));
    let drive = char::from(drives.next()?);
    if drives.next().is_some() {
        return None;
    }
    let path = format!("{drive}:\\{relative}");
    windows_proxy_path(&path).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn kernel_and_handle_observations_agree_for_the_test_process() {
        let pid = std::process::id();
        let kernel = kernel_process_owner(pid).unwrap();
        assert_eq!(kernel, process_owner(pid).unwrap());
    }

    #[test]
    fn kernel_read_rejects_pid_reuse_or_unavailable_evidence() {
        let generation = Cell::new(40);
        let result = read_kernel_owner(
            17,
            || Ok(generation.get()),
            || {
                generation.set(41);
                Ok(r"C:\Test\proxy.exe".into())
            },
        );
        assert!(matches!(result, Err(Fault::Drift)));
        let unavailable = read_kernel_owner(17, || Ok(40), || Err(Fault::ProcessIdentity));
        assert!(matches!(unavailable, Err(Fault::ProcessIdentity)));
    }

    #[test]
    fn drive_mapping_requires_exact_volume_and_one_unambiguous_local_drive() {
        let map = |drive, device: &str| drive == b'D' && device == r"\Device\HarddiskVolume12";
        let path = r"\Device\HarddiskVolume12\Proxy\MixedCase.exe";
        assert_eq!(
            dos_image_path(path, map).as_deref(),
            Some(r"D:\Proxy\MixedCase.exe")
        );
        assert_eq!(dos_image_path(path, |_, _| true), None);
        assert_eq!(
            dos_image_path(r"\Device\HarddiskVolume123\proxy.exe", map),
            None
        );
        assert_eq!(dos_image_path(r"\Device\Mup\server\proxy.exe", map), None);
        assert_eq!(
            dos_image_path(r"\Device\HarddiskVolume12\..\proxy.exe", map),
            None
        );
    }
}
