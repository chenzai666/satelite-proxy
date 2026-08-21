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
use serde::{Deserialize, Serialize};
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
const KEY_QUERY_VALUE: DWORD = 0x0001;
const KEY_SET_VALUE: DWORD = 0x0002;
const REG_SZ: DWORD = 1;
const REG_DWORD: DWORD = 4;
const ERROR_SUCCESS: LONG = 0;
const ERROR_FILE_NOT_FOUND: LONG = 2;

const INTERNET_OPTION_SETTINGS_CHANGED: DWORD = 39;
const INTERNET_OPTION_REFRESH: DWORD = 37;
const REG_EXPAND_SZ: DWORD = 2;

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
    fn RegQueryValueExW(
        hKey: HKEY,
        lpValueName: LPCWSTR,
        lpReserved: *mut DWORD,
        lpType: *mut DWORD,
        lpData: *mut u8,
        lpcbData: *mut DWORD,
    ) -> LONG;
    fn RegDeleteValueW(hKey: HKEY, lpValueName: LPCWSTR) -> LONG;
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
            KEY_QUERY_VALUE | KEY_SET_VALUE,
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

fn with_internet_settings<T>(f: impl FnOnce(HKEY) -> AppResult<T>) -> AppResult<T> {
    let h = open_internet_settings()?;
    let result = f(h);
    unsafe { RegCloseKey(h) };
    result
}

fn query_dword(h: HKEY, name: &str) -> AppResult<Option<DWORD>> {
    let name = wide(name);
    let mut kind = 0;
    let mut value = 0;
    let mut size = core::mem::size_of::<DWORD>() as DWORD;
    let rc = unsafe {
        RegQueryValueExW(
            h,
            name.as_ptr(),
            core::ptr::null_mut(),
            &mut kind,
            (&mut value as *mut DWORD).cast::<u8>(),
            &mut size,
        )
    };
    if rc == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if rc != ERROR_SUCCESS || kind != REG_DWORD {
        return Err(AppError::Core(format!(
            "RegQueryValueExW dword failed (rc={rc}, type={kind})"
        )));
    }
    Ok(Some(value))
}

fn query_sz(h: HKEY, name: &str) -> AppResult<Option<String>> {
    let name = wide(name);
    let mut kind = 0;
    let mut size = 0;
    let rc = unsafe {
        RegQueryValueExW(
            h,
            name.as_ptr(),
            core::ptr::null_mut(),
            &mut kind,
            core::ptr::null_mut(),
            &mut size,
        )
    };
    if rc == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if rc != ERROR_SUCCESS || !matches!(kind, REG_SZ | REG_EXPAND_SZ) {
        return Err(AppError::Core(format!(
            "RegQueryValueExW string size failed (rc={rc}, type={kind})"
        )));
    }
    let mut buffer = vec![0u16; (size as usize / 2).max(1)];
    let rc = unsafe {
        RegQueryValueExW(
            h,
            name.as_ptr(),
            core::ptr::null_mut(),
            &mut kind,
            buffer.as_mut_ptr().cast::<u8>(),
            &mut size,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(AppError::Core(format!(
            "RegQueryValueExW string failed (rc={rc})"
        )));
    }
    let len = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    Ok(Some(String::from_utf16_lossy(&buffer[..len])))
}

fn proxy_server_endpoint(value: &str) -> Option<String> {
    let endpoints: Vec<_> = value
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            Some(
                part.split_once('=')
                    .map(|(_, endpoint)| endpoint)
                    .unwrap_or(part)
                    .trim()
                    .to_ascii_lowercase(),
            )
        })
        .collect();
    let first = endpoints.first()?.clone();
    endpoints
        .iter()
        .all(|endpoint| endpoint == &first)
        .then_some(first)
}

fn proxy_server_is_exact_endpoint(value: &str, host: &str, port: u16) -> bool {
    let expected = format!("{}:{port}", host.trim().to_ascii_lowercase());
    proxy_server_endpoint(value).as_deref() == Some(expected.as_str())
}

fn proxy_server_is_exact_endpoint_value(value: &str, expected: &str) -> bool {
    let value = proxy_server_endpoint(value);
    value.is_some() && value == proxy_server_endpoint(expected)
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

fn delete_value(h: HKEY, name: &str) -> AppResult<()> {
    let name_wide = wide(name);
    let rc = unsafe { RegDeleteValueW(h, name_wide.as_ptr()) };
    if rc != ERROR_SUCCESS && rc != ERROR_FILE_NOT_FOUND {
        return Err(AppError::Core(format!(
            "RegDeleteValueW {name} failed (rc={rc})"
        )));
    }
    Ok(())
}

fn restore_optional_dword(h: HKEY, name: &str, value: Option<DWORD>) -> AppResult<()> {
    match value {
        Some(value) => set_dword(h, name, value),
        None => delete_value(h, name),
    }
}

fn restore_optional_sz(h: HKEY, name: &str, value: Option<&str>) -> AppResult<()> {
    match value {
        Some(value) => set_sz(h, name, value),
        None => delete_value(h, name),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WindowsProxySnapshot {
    applied_server: String,
    proxy_enable: Option<DWORD>,
    proxy_server: Option<String>,
    proxy_override: Option<String>,
    auto_config_url: Option<String>,
}

impl WindowsProxySnapshot {
    fn capture(h: HKEY, applied_server: String) -> AppResult<Self> {
        Ok(Self {
            applied_server,
            proxy_enable: query_dword(h, "ProxyEnable")?,
            proxy_server: query_sz(h, "ProxyServer")?,
            proxy_override: query_sz(h, "ProxyOverride")?,
            auto_config_url: query_sz(h, "AutoConfigURL")?,
        })
    }

    fn restore(&self, h: HKEY) -> AppResult<()> {
        // Restore values before the enable flag so clients never observe an
        // enabled proxy pointing at a half-restored endpoint.
        restore_optional_sz(h, "ProxyServer", self.proxy_server.as_deref())?;
        restore_optional_sz(h, "ProxyOverride", self.proxy_override.as_deref())?;
        restore_optional_sz(h, "AutoConfigURL", self.auto_config_url.as_deref())?;
        restore_optional_dword(h, "ProxyEnable", self.proxy_enable)?;
        Ok(())
    }

    fn encode(&self) -> AppResult<String> {
        serde_json::to_string(self)
            .map_err(|error| AppError::Core(format!("serialize Windows proxy snapshot: {error}")))
    }

    fn decode(detail: &str) -> Option<Self> {
        serde_json::from_str(detail).ok()
    }
}

/// Tell WinINet (and most Win32 apps / Chrome / Edge) to reload proxy settings.
fn notify_changed() -> AppResult<()> {
    let (changed, refreshed) = unsafe {
        // Both options are needed: SETTINGS_CHANGED invalidates the cache,
        // REFRESH forces an immediate reload.
        let changed = InternetSetOptionW(
            core::ptr::null_mut(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            core::ptr::null_mut(),
            0,
        );
        let refreshed = InternetSetOptionW(
            core::ptr::null_mut(),
            INTERNET_OPTION_REFRESH,
            core::ptr::null_mut(),
            0,
        );
        (changed, refreshed)
    };
    if changed == 0 || refreshed == 0 {
        return Err(AppError::Core(format!(
            "WinINet proxy refresh failed (settings_changed={changed}, refresh={refreshed}): {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

pub struct WindowsSystemProxy;

impl SystemProxy for WindowsSystemProxy {
    fn enable(&self, host: &str, port: u16) -> AppResult<SystemProxySnapshot> {
        // ProxyServer format: "host:port" applies to all protocols.
        // For per-protocol we'd use "http=host:port;https=host:port;socks=host:port",
        // but the unified form is what sing-box's mixed port expects.
        let server = format!("{host}:{port}");
        let snapshot =
            with_internet_settings(|h| WindowsProxySnapshot::capture(h, server.clone()))?;
        let snapshot_detail = snapshot.encode()?;

        let apply_result = with_internet_settings(|h| {
            // Write endpoint and bypasses first, then enable last. A PAC URL
            // can take precedence over the manual proxy in WinINet clients,
            // so suspend it while Satelite owns the system proxy.
            set_sz(h, "ProxyServer", &server)?;
            set_sz(
                h,
                "ProxyOverride",
                "<local>;localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*",
            )?;
            delete_value(h, "AutoConfigURL")?;
            set_dword(h, "ProxyEnable", 1)
        });
        if let Err(error) = apply_result {
            let _ = with_internet_settings(|h| snapshot.restore(h));
            let _ = notify_changed();
            return Err(error);
        }
        if let Err(error) = notify_changed() {
            let _ = with_internet_settings(|h| snapshot.restore(h));
            let _ = notify_changed();
            return Err(error);
        }

        let verified = with_internet_settings(|h| {
            let enabled = query_dword(h, "ProxyEnable")?.unwrap_or(0) != 0;
            let current = query_sz(h, "ProxyServer")?.unwrap_or_default();
            Ok(enabled && proxy_server_is_exact_endpoint(&current, host, port))
        })?;
        if !verified {
            let _ = with_internet_settings(|h| snapshot.restore(h));
            let _ = notify_changed();
            return Err(AppError::Core(format!(
                "Windows 系统代理写入后校验失败，预期端点 {server}"
            )));
        }

        Ok(SystemProxySnapshot {
            detail: snapshot_detail,
        })
    }

    fn disable(&self, snapshot: Option<&SystemProxySnapshot>) -> AppResult<()> {
        let decoded = snapshot.and_then(|snapshot| WindowsProxySnapshot::decode(&snapshot.detail));

        if let Some(previous) = decoded {
            let still_owned = with_internet_settings(|h| {
                let enabled = query_dword(h, "ProxyEnable")?.unwrap_or(0) != 0;
                let server = query_sz(h, "ProxyServer")?.unwrap_or_default();
                Ok(enabled
                    && proxy_server_is_exact_endpoint_value(&server, &previous.applied_server))
            })?;
            if !still_owned {
                // Another proxy manager changed the OS settings after us.
                // Never overwrite its newer choice with our stale snapshot.
                return Ok(());
            }
            with_internet_settings(|h| previous.restore(h))?;
            return notify_changed();
        }

        // Compatibility path for pre-snapshot ownership markers. Disable only
        // when the current endpoint still matches the marker; with no marker,
        // retain the legacy best-effort behaviour.
        let expected = snapshot
            .map(|snapshot| snapshot.detail.trim())
            .filter(|s| !s.is_empty());
        with_internet_settings(|h| {
            if let Some(expected) = expected {
                let enabled = query_dword(h, "ProxyEnable")?.unwrap_or(0) != 0;
                let current = query_sz(h, "ProxyServer")?.unwrap_or_default();
                if !enabled || !proxy_server_is_exact_endpoint_value(&current, expected) {
                    return Ok(());
                }
            }
            set_dword(h, "ProxyEnable", 0)
        })?;
        notify_changed()
    }

    fn detect_owned(&self, host: &str, port: u16) -> AppResult<Option<SystemProxySnapshot>> {
        let (enabled, server) = with_internet_settings(|h| {
            Ok((
                query_dword(h, "ProxyEnable")?.unwrap_or(0) != 0,
                query_sz(h, "ProxyServer")?.unwrap_or_default(),
            ))
        })?;
        if enabled && proxy_server_is_exact_endpoint(&server, host, port) {
            Ok(Some(SystemProxySnapshot { detail: server }))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        proxy_server_is_exact_endpoint, proxy_server_is_exact_endpoint_value, WindowsProxySnapshot,
    };

    #[test]
    fn recognizes_only_an_exact_owned_proxy_server() {
        assert!(proxy_server_is_exact_endpoint(
            "127.0.0.1:2080",
            "127.0.0.1",
            2080
        ));
        assert!(proxy_server_is_exact_endpoint(
            "http=127.0.0.1:2080;https=127.0.0.1:2080;socks=127.0.0.1:2080",
            "127.0.0.1",
            2080
        ));
        assert!(!proxy_server_is_exact_endpoint(
            "http=127.0.0.1:2080;https=proxy.example:8080",
            "127.0.0.1",
            2080
        ));
        assert!(!proxy_server_is_exact_endpoint(
            "127.0.0.1:2081",
            "127.0.0.1",
            2080
        ));
    }

    #[test]
    fn equivalent_unified_and_per_protocol_endpoints_match() {
        assert!(proxy_server_is_exact_endpoint_value(
            "127.0.0.1:2080",
            "http=127.0.0.1:2080;https=127.0.0.1:2080;socks=127.0.0.1:2080"
        ));
        assert!(!proxy_server_is_exact_endpoint_value(
            "127.0.0.1:2080",
            "127.0.0.1:10808"
        ));
    }

    #[test]
    fn snapshot_round_trip_preserves_previous_proxy_and_pac() {
        let snapshot = WindowsProxySnapshot {
            applied_server: "127.0.0.1:2080".into(),
            proxy_enable: Some(1),
            proxy_server: Some("127.0.0.1:10808".into()),
            proxy_override: Some("<local>".into()),
            auto_config_url: Some("http://127.0.0.1/proxy.pac".into()),
        };
        let encoded = snapshot.encode().unwrap();
        assert_eq!(WindowsProxySnapshot::decode(&encoded), Some(snapshot));
    }
}
