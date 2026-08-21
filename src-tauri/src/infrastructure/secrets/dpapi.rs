//! Windows DPAPI binding — the encryption primitive behind `SecretStore`
//! (S7: secret values persist only as DPAPI-protected blobs).
//!
//! `CryptProtectData`/`CryptUnprotectData` are pure byte-buffer transforms
//! (current-user scope); they do not read or modify any persistent OS state.

use windows::{
    core::PCWSTR,
    Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB},
};

use crate::error::AppError;

// `kernel32!LocalFree` — the windows crate build for this toolchain does not
// expose it, so the single stable Win32 entry point is declared directly.
// CryptProtect/UnprotectData output is LocalAlloc'd and must be LocalFree'd.
extern "system" {
    fn LocalFree(hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}

/// `CRYPTPROTECT_PROMPT_NONE` (0x1): no interactive prompt.
const CRYPTPROTECT_PROMPT_NONE: u32 = 0x1;

/// DPAPI encryption primitive seam: hidden behind a trait so unit tests can
/// substitute a fake for the OS call (Phase 3 contract).
pub trait DpapiPort: Send + Sync + std::fmt::Debug {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, AppError>;
    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, AppError>;
}

/// The production DPAPI implementation.
#[derive(Debug, Default)]
pub struct WindowsDpapi;

impl WindowsDpapi {
    pub fn new() -> Self {
        Self
    }
}

impl DpapiPort for WindowsDpapi {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut out_blob: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
        unsafe {
            CryptProtectData(
                &in_blob,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_PROMPT_NONE,
                &mut out_blob,
            )
        }
        .map_err(dpapi_error)?;
        let bytes =
            unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize) }
                .to_vec();
        unsafe {
            LocalFree(out_blob.pbData as *mut std::ffi::c_void);
        }
        Ok(bytes)
    }

    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut out_blob: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
        unsafe { CryptUnprotectData(&in_blob, None, None, None, None, 0, &mut out_blob) }
            .map_err(dpapi_error)?;
        let bytes =
            unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize) }
                .to_vec();
        unsafe {
            LocalFree(out_blob.pbData as *mut std::ffi::c_void);
        }
        Ok(bytes)
    }
}

fn dpapi_error(err: windows::core::Error) -> AppError {
    AppError::secret_store_unavailable(format!("DPAPI call failed: {err}"))
}
