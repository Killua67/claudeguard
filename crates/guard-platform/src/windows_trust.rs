use crate::Fault;
use guard_core::config::windows_proxy_path;
use std::{
    fs::{File, OpenOptions},
    mem::size_of,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Security::{
        Cryptography::{CertGetNameStringW, CERT_CONTEXT, CERT_NAME_ATTR_TYPE},
        WinTrust::{
            WTHelperGetProvCertFromChain, WTHelperGetProvSignerFromChain,
            WTHelperProvDataFromStateData, WinVerifyTrust, CRYPT_PROVIDER_CERT,
            CRYPT_PROVIDER_DATA, CRYPT_PROVIDER_SGNR, WINTRUST_ACTION_GENERIC_VERIFY_V2,
            WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CACHE_ONLY_URL_RETRIEVAL,
            WTD_CHOICE_FILE, WTD_REVOCATION_CHECK_NONE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
            WTD_STATEACTION_VERIFY, WTD_UI_NONE,
        },
    },
    Storage::FileSystem::FILE_SHARE_READ,
};

pub(super) struct VerifiedExecutable {
    pub(super) path: PathBuf,
    _file: File,
}

fn publisher_matches(common_name: &str, organization: &str) -> bool {
    common_name == "Anthropic, PBC" && organization == "Anthropic, PBC"
}

fn validate_input(path: &Path) -> Result<(), Fault> {
    let text = path.to_str().ok_or(Fault::Client)?;
    if !windows_proxy_path(text)
        || !path.is_absolute()
        || !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err(Fault::Client);
    }
    Ok(())
}

fn normalize_canonical(path: &Path) -> Result<PathBuf, Fault> {
    let text = path.to_str().ok_or(Fault::Client)?;
    let normalized = PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(text));
    // Only a plain local drive remainder is accepted, never UNC or device namespaces.
    validate_input(&normalized)?;
    Ok(normalized)
}

struct TrustState(WINTRUST_DATA);

impl Drop for TrustState {
    fn drop(&mut self) {
        self.0.dwStateAction = WTD_STATEACTION_CLOSE;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        // The file info, UTF-16 path and locked file still live when state is closed.
        unsafe {
            WinVerifyTrust(
                null_mut(),
                &mut action,
                (&mut self.0 as *mut WINTRUST_DATA).cast(),
            );
        }
    }
}

fn certificate_attribute(certificate: &CERT_CONTEXT, oid: &'static [u8]) -> Result<String, Fault> {
    // The provider owns this certificate until TrustState closes. OIDs are static NUL strings.
    let length = unsafe {
        CertGetNameStringW(
            certificate,
            CERT_NAME_ATTR_TYPE,
            0,
            oid.as_ptr().cast(),
            null_mut(),
            0,
        )
    };
    if !(2..=256).contains(&length) {
        return Err(Fault::Client);
    }
    let mut buffer = vec![0u16; length as usize];
    let written = unsafe {
        CertGetNameStringW(
            certificate,
            CERT_NAME_ATTR_TYPE,
            0,
            oid.as_ptr().cast(),
            buffer.as_mut_ptr(),
            length,
        )
    };
    if written != length || buffer.last() != Some(&0) || buffer[..buffer.len() - 1].contains(&0) {
        return Err(Fault::Client);
    }
    String::from_utf16(&buffer[..buffer.len() - 1]).map_err(|_| Fault::Client)
}

fn verify_publisher(state: &TrustState) -> Result<(), Fault> {
    if state.0.hWVTStateData.is_null() {
        return Err(Fault::Client);
    }
    // These helpers return provider-owned objects, valid only while state remains open.
    let provider_ptr = unsafe { WTHelperProvDataFromStateData(state.0.hWVTStateData) };
    let provider = unsafe { provider_ptr.as_ref() }.ok_or(Fault::Client)?;
    if provider.cbStruct < size_of::<CRYPT_PROVIDER_DATA>() as u32
        || provider.csSigners == 0
        || provider.pasSigners.is_null()
    {
        return Err(Fault::Client);
    }
    let signer_ptr = unsafe { WTHelperGetProvSignerFromChain(provider_ptr, 0, 0, 0) };
    let signer = unsafe { signer_ptr.as_ref() }.ok_or(Fault::Client)?;
    if signer.cbStruct < size_of::<CRYPT_PROVIDER_SGNR>() as u32
        || signer.csCertChain == 0
        || signer.pasCertChain.is_null()
    {
        return Err(Fault::Client);
    }
    let certificate_ptr = unsafe { WTHelperGetProvCertFromChain(signer_ptr, 0) };
    let certificate = unsafe { certificate_ptr.as_ref() }.ok_or(Fault::Client)?;
    if certificate.cbStruct < size_of::<CRYPT_PROVIDER_CERT>() as u32 {
        return Err(Fault::Client);
    }
    let context = unsafe { certificate.pCert.as_ref() }.ok_or(Fault::Client)?;
    if context.pCertInfo.is_null() || context.pbCertEncoded.is_null() || context.cbCertEncoded == 0
    {
        return Err(Fault::Client);
    }
    let common_name = certificate_attribute(context, b"2.5.4.3\0")?;
    let organization = certificate_attribute(context, b"2.5.4.10\0")?;
    if !publisher_matches(&common_name, &organization) {
        return Err(Fault::Client);
    }
    Ok(())
}

pub(super) fn verify(path: &Path) -> Result<VerifiedExecutable, Fault> {
    validate_input(path)?;
    let canonical = path.canonicalize().map_err(|_| Fault::Client)?;
    let path = normalize_canonical(&canonical)?;
    // Deny write/delete sharing; the caller keeps this handle through CreateProcess.
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&path)
        .map_err(|_| Fault::Client)?;
    if !file.metadata().map_err(|_| Fault::Client)?.is_file() {
        return Err(Fault::Client);
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut info = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: wide.as_ptr(),
        hFile: file.as_raw_handle(),
        ..Default::default()
    };
    let mut state = TrustState(WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut info },
        dwStateAction: WTD_STATEACTION_VERIFY,
        // Offline cache avoids network access; it cannot guarantee current revocation status.
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_NONE,
        ..Default::default()
    });
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // All pointers refer to live local buffers or the held file; no UI or network retrieval.
    let status = unsafe {
        WinVerifyTrust(
            null_mut(),
            &mut action,
            (&mut state.0 as *mut WINTRUST_DATA).cast(),
        )
    };
    if status != 0 {
        return Err(Fault::Client);
    }
    verify_publisher(&state)?;
    drop(state);
    Ok(VerifiedExecutable { path, _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publisher_requires_exact_common_name_and_organization() {
        assert!(publisher_matches("Anthropic, PBC", "Anthropic, PBC"));
        for name in [
            "",
            "Anthropic",
            "anthropic, pbc",
            "Anthropic, PBC ",
            " Anthropic, PBC",
            "Anthropic, PBC\0",
            "Anthropic, PBC attacker",
            "Other Publisher",
        ] {
            assert!(!publisher_matches(name, "Anthropic, PBC"));
            assert!(!publisher_matches("Anthropic, PBC", name));
        }
    }

    #[test]
    fn canonical_prefix_is_removed_only_for_local_drive_paths() {
        for (input, expected) in [
            (
                r"C:\Program Files\Claude\claude.exe",
                r"C:\Program Files\Claude\claude.exe",
            ),
            (
                r"\\?\C:\Program Files\Claude\claude.exe",
                r"C:\Program Files\Claude\claude.exe",
            ),
            (r"\\?\d:\客户端\CLAUDE.EXE", r"d:\客户端\CLAUDE.EXE"),
        ] {
            assert_eq!(
                normalize_canonical(Path::new(input)).unwrap(),
                PathBuf::from(expected)
            );
        }
    }

    #[test]
    fn executable_paths_reject_scripts_relative_shares_devices_and_controls() {
        for input in [
            "",
            "claude.exe",
            r"C:claude.exe",
            r"\claude.exe",
            r"C:/Claude/claude.exe",
            r"\\server\share\claude.exe",
            r"\\?\UNC\server\share\claude.exe",
            r"\\.\C:\Claude\claude.exe",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\claude.exe",
            r"\\?\Volume{00000000-0000-0000-0000-000000000000}\claude.exe",
            r"C:\Claude\..\claude.exe",
            r"C:\Claude\.\claude.exe",
            r"C:\Claude\\claude.exe",
            r"C:\Claude\claude.exe:stream",
            r"C:\Claude\claude.exe ",
            r"C:\Claude\claude.cmd",
            r"C:\Claude\claude.CMD",
            r"C:\Claude\claude.bat",
            r"C:\Claude\claude.BAT",
            "C:\\Claude\\claude\0.exe",
            "C:\\Claude\\claude\n.exe",
        ] {
            assert!(
                matches!(normalize_canonical(Path::new(input)), Err(Fault::Client)),
                "{input:?}"
            );
        }
        assert!(matches!(
            validate_input(Path::new(r"\\?\C:\Claude\claude.exe")),
            Err(Fault::Client)
        ));
        assert!(validate_input(Path::new(r"C:\Claude\claude.exe")).is_ok());
    }

    #[test]
    fn native_verification_rejects_the_rust_test_executable() {
        let current = std::env::current_exe().unwrap();
        let current = normalize_canonical(&current.canonicalize().unwrap()).unwrap();
        assert!(matches!(verify(&current), Err(Fault::Client)));
    }
}
