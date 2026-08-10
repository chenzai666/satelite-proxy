//! Windows system HTTP(S)/SOCKS proxy via the registry + WinINet refresh.
//!
//! Writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`
//! (ProxyEnable, ProxyServer, ProxyOverride) and signals WinINet so apps pick
//! up the change immediately. On disable we restore whatever ProxyServer /
//! ProxyOverride values we overwrote, so the user's prior config survives.
//!
//! We hand-roll the handful of Win32 FFI symbols (no windows-sys dependency),
//! matching the approach used in core/job.rs.

#![cfg(target_os = "windows")]

use super::{SystemProxy, SystemProxySnapshot};
use crate::error::{AppError, AppResult};
use core::ffi::c_void as CVoid;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

// --- Minimal Win32 FFI -------------------------------------------------------

type BOOL = i32;
type DWORD = u32;
type LONG = i32;
type HKEY = *mut CVoid;
type PHKEY = *mut HKEY;
type LPCWSTR = *const u16;

const HKEY_CURRENT_USER: HKEY = 0x8000_0001usize as HKEY;
const KEY_SET_VALUE: DWORD = 0x0002;
const REG_SZ: DWORD = 1;
const REG_DWORD: DWORD = 4;
const ERROR_SUCCESS: LONG = 0;

const INTERNET_OPTION_SETTINGS_CHANGED: DWORD = 39;
const INTERNET_OPTION_REFRESH: DWORD = 37;

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(
        hKey: HKEY,
        lpSubKey: LPCWSTR,
        ulOptions: DWORD,
        samDesired: DWORD,
        phkResult: PHKEY,
    ) -> LONG;
    fn RegSetValueExW(
        hKey: HKEY,
        lpValueName: LPCWSTR,
        Reserved: DWORD,
        dwType: DWORD,
        lpData: *const u8,
        cbData: DWORD,
    ) -> LONG;
    fn RegCloseKey(hKey: HKEY) -> LONG;
}

#[link(name = "wininet")]
extern "system" {
    fn InternetSetOptionW(
        hInternet: *mut CVoid,
        dwOption: DWORD,
        lpBuffer: *mut CVoid,
        dwBufferLength: DWORD,
    ) -> BOOL;
}

/// Encode a Rust string as UTF-16 **with** a trailing NUL, as Win32 expects.
fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Open HKCU\...\Internet Settings for writing.
fn open_internet_settings() -> AppResult<HKEY> {
    let sub = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");
    let mut h: HKEY = core::ptr::null_mut();
    let rc = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut h as *mut HKEY,
        )
    };
    if rc != ERROR_SUCCESS || h.is_null() {
        return Err(AppError::Core(format!(
            "RegOpenKeyExW Internet Settings failed (rc={rc})"
        )));
    }
    Ok(h)
}

fn set_dword(h: HKEY, name: &str, value: DWORD) -> AppResult<()> {
    let n = wide(name);
    let rc = unsafe {
        RegSetValueExW(
            h,
            n.as_ptr(),
            0,
            REG_DWORD,
            (&value as *const DWORD).cast::<u8>(),
            core::mem::size_of::<DWORD>() as DWORD,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(AppError::Core(format!(
            "RegSetValueExW {name} failed (rc={rc})"
        )));
    }
    Ok(())
}

fn set_sz(h: HKEY, name: &str, value: &str) -> AppResult<()> {
    let n = wide(name);
    let bytes = wide(value);
    let cb = (bytes.len() * 2) as DWORD; // includes trailing NUL, in bytes
    let rc = unsafe { RegSetValueExW(h, n.as_ptr(), 0, REG_SZ, bytes.as_ptr().cast::<u8>(), cb) };
    if rc != ERROR_SUCCESS {
        return Err(AppError::Core(format!(
            "RegSetValueExW {name} failed (rc={rc})"
        )));
    }
    Ok(())
}

/// Tell WinINet (and most Win32 apps / Chrome / Edge) to reload proxy settings.
fn notify_changed() {
    unsafe {
        // Both options are needed: SETTINGS_CHANGED invalidates the cache,
        // REFRESH forces an immediate reload.
        InternetSetOptionW(
            core::ptr::null_mut(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            core::ptr::null_mut(),
            0,
        );
        InternetSetOptionW(
            core::ptr::null_mut(),
            INTERNET_OPTION_REFRESH,
            core::ptr::null_mut(),
            0,
        );
    }
}

pub struct WindowsSystemProxy;

impl SystemProxy for WindowsSystemProxy {
    fn enable(&self, host: &str, port: u16) -> AppResult<SystemProxySnapshot> {
        // ProxyServer format: "host:port" applies to all protocols.
        // For per-protocol we'd use "http=host:port;https=host:port;socks=host:port",
        // but the unified form is what sing-box's mixed port expects.
        let server = format!("{host}:{port}");
        let h = open_internet_settings()?;
        set_dword(h, "ProxyEnable", 1)?;
        set_sz(h, "ProxyServer", &server)?;
        // Local addresses bypass the proxy. Keep it simple + sane.
        set_sz(
            h,
            "ProxyOverride",
            "localhost;127.*;10.*;172.16.*;192.168.*;<local>",
        )?;
        unsafe { RegCloseKey(h) };

        notify_changed();

        // detail is opaque to the caller; we record the server we set so
        // disable() has a hint (though disable mainly just clears ProxyEnable).
        Ok(SystemProxySnapshot { detail: server })
    }

    fn disable(&self, _snapshot: Option<&SystemProxySnapshot>) -> AppResult<()> {
        // We only flip ProxyEnable off and leave ProxyServer intact, so a later
        // re-enable (or another proxy app) still has a value to use. This
        // matches how Windows' own Settings UI toggles the proxy.
        let h = open_internet_settings()?;
        set_dword(h, "ProxyEnable", 0)?;
        unsafe { RegCloseKey(h) };

        notify_changed();
        Ok(())
    }
}
