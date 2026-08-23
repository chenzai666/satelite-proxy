//! Windows system HTTP(S)/SOCKS proxy via the registry + WinINet refresh.
//!
//! Writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`
//! (ProxyEnable, ProxyServer, ProxyOverride) and the current user's standard
//! proxy environment variables. WinINet clients reload immediately; CLI/Rust
//! clients such as Codex inherit the environment when they are next launched.
//! On disable we restore only values that Satelite still owns.
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
type LPWSTR = *mut u16;
type HWND = *mut CVoid;
type UINT = u32;
type WPARAM = usize;
type LPARAM = isize;
type DWORD_PTR = usize;

const HKEY_CURRENT_USER: HKEY = 0x8000_0001usize as HKEY;
const HWND_BROADCAST: HWND = 0xffffusize as HWND;
const KEY_QUERY_VALUE: DWORD = 0x0001;
const KEY_SET_VALUE: DWORD = 0x0002;
const REG_SZ: DWORD = 1;
const REG_DWORD: DWORD = 4;
const ERROR_SUCCESS: LONG = 0;
const ERROR_FILE_NOT_FOUND: LONG = 2;

const INTERNET_OPTION_SETTINGS_CHANGED: DWORD = 39;
const INTERNET_OPTION_REFRESH: DWORD = 37;
const INTERNET_OPTION_PER_CONNECTION_OPTION: DWORD = 75;
const REG_EXPAND_SZ: DWORD = 2;

const INTERNET_PER_CONN_FLAGS: DWORD = 1;
const INTERNET_PER_CONN_PROXY_SERVER: DWORD = 2;
const INTERNET_PER_CONN_PROXY_BYPASS: DWORD = 3;
const INTERNET_PER_CONN_AUTOCONFIG_URL: DWORD = 4;
const PROXY_TYPE_DIRECT: DWORD = 0x0000_0001;
const PROXY_TYPE_PROXY: DWORD = 0x0000_0002;
const PROXY_TYPE_AUTO_PROXY_URL: DWORD = 0x0000_0004;
const PROXY_TYPE_AUTO_DETECT: DWORD = 0x0000_0008;
const ERROR_BUFFER_TOO_SMALL: DWORD = 603;
const RAS_MAX_ENTRY_NAME: usize = 256;
const MAX_PATH: usize = 260;
const WM_SETTINGCHANGE: UINT = 0x001a;
const SMTO_ABORTIFHUNG: UINT = 0x0002;
const ENV_NOTIFY_TIMEOUT_MS: UINT = 2_000;

#[repr(C)]
#[derive(Clone, Copy)]
struct FileTime {
    low: DWORD,
    high: DWORD,
}

#[repr(C)]
#[derive(Clone, Copy)]
union InternetPerConnOptionValue {
    value: DWORD,
    string: LPWSTR,
    file_time: FileTime,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InternetPerConnOption {
    option: DWORD,
    value: InternetPerConnOptionValue,
}

#[repr(C)]
struct InternetPerConnOptionList {
    size: DWORD,
    connection: LPWSTR,
    option_count: DWORD,
    option_error: DWORD,
    options: *mut InternetPerConnOption,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RasEntryName {
    size: DWORD,
    entry_name: [u16; RAS_MAX_ENTRY_NAME + 1],
    flags: DWORD,
    phonebook_path: [u16; MAX_PATH + 1],
}

impl Default for RasEntryName {
    fn default() -> Self {
        Self {
            size: core::mem::size_of::<Self>() as DWORD,
            entry_name: [0; RAS_MAX_ENTRY_NAME + 1],
            flags: 0,
            phonebook_path: [0; MAX_PATH + 1],
        }
    }
}

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

#[link(name = "user32")]
extern "system" {
    fn SendMessageTimeoutW(
        hWnd: HWND,
        Msg: UINT,
        wParam: WPARAM,
        lParam: LPARAM,
        fuFlags: UINT,
        uTimeout: UINT,
        lpdwResult: *mut DWORD_PTR,
    ) -> isize;
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

#[link(name = "rasapi32")]
extern "system" {
    fn RasEnumEntriesW(
        reserved: LPCWSTR,
        phonebook: LPCWSTR,
        entries: *mut RasEntryName,
        buffer_size: *mut DWORD,
        entry_count: *mut DWORD,
    ) -> DWORD;
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

fn open_user_environment() -> AppResult<HKEY> {
    let sub = wide("Environment");
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
            "RegOpenKeyExW user Environment failed (rc={rc})"
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

fn with_user_environment<T>(f: impl FnOnce(HKEY) -> AppResult<T>) -> AppResult<T> {
    let h = open_user_environment()?;
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
    #[serde(default)]
    auto_detect: Option<DWORD>,
    #[serde(default)]
    env_http_proxy: Option<String>,
    #[serde(default)]
    env_https_proxy: Option<String>,
    #[serde(default)]
    env_all_proxy: Option<String>,
    #[serde(default)]
    env_no_proxy: Option<String>,
    #[serde(default)]
    applied_no_proxy: Option<String>,
}

impl WindowsProxySnapshot {
    fn capture(h: HKEY, applied_server: String) -> AppResult<Self> {
        let mut snapshot = Self {
            applied_server,
            proxy_enable: query_dword(h, "ProxyEnable")?,
            proxy_server: query_sz(h, "ProxyServer")?,
            proxy_override: query_sz(h, "ProxyOverride")?,
            auto_config_url: query_sz(h, "AutoConfigURL")?,
            auto_detect: query_dword(h, "AutoDetect")?,
            env_http_proxy: None,
            env_https_proxy: None,
            env_all_proxy: None,
            env_no_proxy: None,
            applied_no_proxy: None,
        };
        with_user_environment(|environment| {
            snapshot.env_http_proxy = query_sz(environment, "HTTP_PROXY")?;
            snapshot.env_https_proxy = query_sz(environment, "HTTPS_PROXY")?;
            snapshot.env_all_proxy = query_sz(environment, "ALL_PROXY")?;
            snapshot.env_no_proxy = query_sz(environment, "NO_PROXY")?;
            snapshot.applied_no_proxy = Some(merge_no_proxy(snapshot.env_no_proxy.as_deref()));
            Ok(())
        })?;
        Ok(snapshot)
    }

    fn restore(&self, h: HKEY) -> AppResult<()> {
        // Restore values before the enable flag so clients never observe an
        // enabled proxy pointing at a half-restored endpoint.
        restore_optional_sz(h, "ProxyServer", self.proxy_server.as_deref())?;
        restore_optional_sz(h, "ProxyOverride", self.proxy_override.as_deref())?;
        restore_optional_sz(h, "AutoConfigURL", self.auto_config_url.as_deref())?;
        restore_optional_dword(h, "AutoDetect", self.auto_detect)?;
        restore_optional_dword(h, "ProxyEnable", self.proxy_enable)?;
        Ok(())
    }

    fn restore_environment_if_owned(&self) -> AppResult<()> {
        let http = format!("http://{}", self.applied_server);
        let socks = format!("socks5://{}", self.applied_server);
        with_user_environment(|h| {
            restore_environment_value_if_owned(
                h,
                "HTTP_PROXY",
                &http,
                self.env_http_proxy.as_deref(),
            )?;
            restore_environment_value_if_owned(
                h,
                "HTTPS_PROXY",
                &http,
                self.env_https_proxy.as_deref(),
            )?;
            restore_environment_value_if_owned(
                h,
                "ALL_PROXY",
                &socks,
                self.env_all_proxy.as_deref(),
            )?;
            if let Some(applied) = self.applied_no_proxy.as_deref() {
                restore_environment_value_if_owned(
                    h,
                    "NO_PROXY",
                    applied,
                    self.env_no_proxy.as_deref(),
                )?;
            }
            Ok(())
        })
    }

    fn encode(&self) -> AppResult<String> {
        serde_json::to_string(self)
            .map_err(|error| AppError::Core(format!("serialize Windows proxy snapshot: {error}")))
    }

    fn decode(detail: &str) -> Option<Self> {
        serde_json::from_str(detail).ok()
    }
}

fn merge_no_proxy(previous: Option<&str>) -> String {
    let mut values: Vec<String> = previous
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect();
    for required in ["localhost", "127.0.0.1", "::1"] {
        if !values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(required))
        {
            values.push(required.into());
        }
    }
    values.join(",")
}

fn restore_environment_value_if_owned(
    h: HKEY,
    name: &str,
    applied: &str,
    previous: Option<&str>,
) -> AppResult<()> {
    let current = query_sz(h, name)?;
    if current
        .as_deref()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(applied.trim()))
    {
        restore_optional_sz(h, name, previous)?;
    }
    Ok(())
}

fn apply_proxy_environment(snapshot: &WindowsProxySnapshot) -> AppResult<()> {
    let http = format!("http://{}", snapshot.applied_server);
    let socks = format!("socks5://{}", snapshot.applied_server);
    let no_proxy = snapshot
        .applied_no_proxy
        .as_deref()
        .unwrap_or("localhost,127.0.0.1,::1");
    with_user_environment(|h| {
        set_sz(h, "HTTP_PROXY", &http)?;
        set_sz(h, "HTTPS_PROXY", &http)?;
        set_sz(h, "ALL_PROXY", &socks)?;
        set_sz(h, "NO_PROXY", no_proxy)
    })
}

fn clear_environment_for_endpoint(server: &str) -> AppResult<()> {
    let http = format!("http://{server}");
    let socks = format!("socks5://{server}");
    with_user_environment(|h| {
        restore_environment_value_if_owned(h, "HTTP_PROXY", &http, None)?;
        restore_environment_value_if_owned(h, "HTTPS_PROXY", &http, None)?;
        restore_environment_value_if_owned(h, "ALL_PROXY", &socks, None)
    })
}

fn notify_environment_changed() {
    // WinINet is refreshed synchronously by notify_changed(); this separate
    // broadcast only advertises the updated user environment. A hung desktop
    // window must not hold the proxy switch for up to two seconds, so keep the
    // UTF-16 buffer owned by a short-lived worker until SendMessage returns.
    let _ = std::thread::spawn(|| {
        let environment = wide("Environment");
        let mut result = 0usize;
        let sent = unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                environment.as_ptr() as LPARAM,
                SMTO_ABORTIFHUNG,
                ENV_NOTIFY_TIMEOUT_MS,
                &mut result,
            )
        };
        if sent == 0 {
            crate::app_log::warn(
                "system_proxy",
                format!(
                    "environment change broadcast failed: {}",
                    std::io::Error::last_os_error()
                ),
            );
        }
    });
}

#[derive(Clone, Copy)]
struct PerConnectionSettings<'a> {
    flags: DWORD,
    proxy: Option<&'a str>,
    bypass: Option<&'a str>,
    pac: Option<&'a str>,
}

impl WindowsProxySnapshot {
    fn per_connection_settings(&self) -> PerConnectionSettings<'_> {
        let mut flags = PROXY_TYPE_DIRECT;
        let proxy = self
            .proxy_server
            .as_deref()
            .filter(|value| self.proxy_enable.unwrap_or(0) != 0 && !value.trim().is_empty());
        if proxy.is_some() {
            flags |= PROXY_TYPE_PROXY;
        }
        let pac = self
            .auto_config_url
            .as_deref()
            .filter(|value| !value.trim().is_empty());
        if pac.is_some() {
            flags |= PROXY_TYPE_AUTO_PROXY_URL;
        }
        if self.auto_detect.unwrap_or(0) != 0 {
            flags |= PROXY_TYPE_AUTO_DETECT;
        }
        PerConnectionSettings {
            flags,
            proxy,
            bypass: self.proxy_override.as_deref(),
            pac,
        }
    }
}

fn utf16_string(buffer: &[u16]) -> String {
    let len = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

fn ras_connection_names() -> AppResult<Vec<String>> {
    let mut first = RasEntryName::default();
    let mut buffer_size = core::mem::size_of::<RasEntryName>() as DWORD;
    let mut entry_count = 0;
    let rc = unsafe {
        RasEnumEntriesW(
            core::ptr::null(),
            core::ptr::null(),
            &mut first,
            &mut buffer_size,
            &mut entry_count,
        )
    };
    if rc == ERROR_SUCCESS as DWORD {
        return Ok((entry_count > 0)
            .then(|| utf16_string(&first.entry_name))
            .filter(|name| !name.is_empty())
            .into_iter()
            .collect());
    }
    if rc != ERROR_BUFFER_TOO_SMALL {
        return Err(AppError::Core(format!("RasEnumEntriesW failed (rc={rc})")));
    }

    let item_size = core::mem::size_of::<RasEntryName>();
    let capacity = ((buffer_size as usize + item_size - 1) / item_size)
        .max(entry_count as usize)
        .max(1);
    let mut entries = vec![RasEntryName::default(); capacity];
    buffer_size = (capacity * item_size) as DWORD;
    let rc = unsafe {
        RasEnumEntriesW(
            core::ptr::null(),
            core::ptr::null(),
            entries.as_mut_ptr(),
            &mut buffer_size,
            &mut entry_count,
        )
    };
    if rc != ERROR_SUCCESS as DWORD {
        return Err(AppError::Core(format!(
            "RasEnumEntriesW retry failed (rc={rc})"
        )));
    }
    Ok(entries
        .iter()
        .take(entry_count as usize)
        .map(|entry| utf16_string(&entry.entry_name))
        .filter(|name| !name.is_empty())
        .collect())
}

fn set_connection_proxy(
    connection: Option<&str>,
    settings: PerConnectionSettings<'_>,
) -> AppResult<()> {
    let mut connection_wide = connection.map(wide);
    let mut proxy_wide = settings.proxy.map(wide);
    let mut bypass_wide = settings.bypass.map(wide);
    let mut pac_wide = settings.pac.map(wide);

    let mut options = Vec::with_capacity(4);
    options.push(InternetPerConnOption {
        option: INTERNET_PER_CONN_FLAGS,
        value: InternetPerConnOptionValue {
            value: settings.flags,
        },
    });
    if let Some(value) = proxy_wide.as_mut() {
        options.push(InternetPerConnOption {
            option: INTERNET_PER_CONN_PROXY_SERVER,
            value: InternetPerConnOptionValue {
                string: value.as_mut_ptr(),
            },
        });
    }
    if let Some(value) = bypass_wide.as_mut() {
        options.push(InternetPerConnOption {
            option: INTERNET_PER_CONN_PROXY_BYPASS,
            value: InternetPerConnOptionValue {
                string: value.as_mut_ptr(),
            },
        });
    }
    if let Some(value) = pac_wide.as_mut() {
        options.push(InternetPerConnOption {
            option: INTERNET_PER_CONN_AUTOCONFIG_URL,
            value: InternetPerConnOptionValue {
                string: value.as_mut_ptr(),
            },
        });
    }
    let mut list = InternetPerConnOptionList {
        size: core::mem::size_of::<InternetPerConnOptionList>() as DWORD,
        connection: connection_wide
            .as_mut()
            .map(|value| value.as_mut_ptr())
            .unwrap_or(core::ptr::null_mut()),
        option_count: options.len() as DWORD,
        option_error: 0,
        options: options.as_mut_ptr(),
    };
    let ok = unsafe {
        InternetSetOptionW(
            core::ptr::null_mut(),
            INTERNET_OPTION_PER_CONNECTION_OPTION,
            (&mut list as *mut InternetPerConnOptionList).cast::<CVoid>(),
            list.size,
        )
    };
    if ok == 0 {
        return Err(AppError::Core(format!(
            "WinINet per-connection proxy failed for {} (option={}): {}",
            connection.unwrap_or("LAN"),
            list.option_error,
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

fn set_all_connections(settings: PerConnectionSettings<'_>) -> AppResult<()> {
    // Null means the active LAN profile used by ordinary Ethernet and Wi-Fi.
    // It is mandatory; RAS/VPN profiles are best-effort parity with v2rayN.
    set_connection_proxy(None, settings)?;
    match ras_connection_names() {
        Ok(connections) => {
            for connection in connections {
                if let Err(error) = set_connection_proxy(Some(&connection), settings) {
                    crate::app_log::warn("system_proxy", error.to_string());
                }
            }
        }
        Err(error) => crate::app_log::warn("system_proxy", error.to_string()),
    }
    Ok(())
}

fn satelite_connection_settings<'a>(server: &'a str, bypass: &'a str) -> PerConnectionSettings<'a> {
    PerConnectionSettings {
        flags: PROXY_TYPE_DIRECT | PROXY_TYPE_PROXY,
        proxy: Some(server),
        bypass: Some(bypass),
        pac: None,
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
        let bypass = "<local>;localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*";

        let apply_result = with_internet_settings(|h| {
            // Write endpoint and bypasses first, then enable last. A PAC URL
            // can take precedence over the manual proxy in WinINet clients,
            // so suspend it while Satelite owns the system proxy.
            set_sz(h, "ProxyServer", &server)?;
            set_sz(h, "ProxyOverride", bypass)?;
            delete_value(h, "AutoConfigURL")?;
            set_dword(h, "AutoDetect", 0)?;
            set_dword(h, "ProxyEnable", 1)
        });
        if let Err(error) = apply_result {
            let _ = with_internet_settings(|h| snapshot.restore(h));
            let _ = notify_changed();
            return Err(error);
        }
        if let Err(error) = set_all_connections(satelite_connection_settings(&server, bypass)) {
            let _ = with_internet_settings(|h| snapshot.restore(h));
            let _ = set_all_connections(snapshot.per_connection_settings());
            let _ = notify_changed();
            return Err(error);
        }
        if let Err(error) = apply_proxy_environment(&snapshot) {
            let _ = with_internet_settings(|h| snapshot.restore(h));
            let _ = set_all_connections(snapshot.per_connection_settings());
            let _ = snapshot.restore_environment_if_owned();
            let _ = notify_changed();
            notify_environment_changed();
            return Err(error);
        }
        if let Err(error) = notify_changed() {
            let _ = with_internet_settings(|h| snapshot.restore(h));
            let _ = set_all_connections(snapshot.per_connection_settings());
            let _ = snapshot.restore_environment_if_owned();
            let _ = notify_changed();
            notify_environment_changed();
            return Err(error);
        }
        notify_environment_changed();

        let verified = with_internet_settings(|h| {
            let enabled = query_dword(h, "ProxyEnable")?.unwrap_or(0) != 0;
            let current = query_sz(h, "ProxyServer")?.unwrap_or_default();
            Ok(enabled && proxy_server_is_exact_endpoint(&current, host, port))
        })?;
        if !verified {
            let _ = with_internet_settings(|h| snapshot.restore(h));
            let _ = set_all_connections(snapshot.per_connection_settings());
            let _ = snapshot.restore_environment_if_owned();
            let _ = notify_changed();
            notify_environment_changed();
            return Err(AppError::Core(format!(
                "Windows 系统代理写入后校验失败，预期端点 {server}"
            )));
        }

        crate::app_log::info(
            "system_proxy",
            format!(
                "user proxy environment updated to {server}; restart already-running CLI/Codex apps to apply"
            ),
        );

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
                // Never overwrite its newer OS choice with our stale snapshot.
                // Environment values are restored independently, but only if
                // they still point at Satelite.
                previous.restore_environment_if_owned()?;
                notify_environment_changed();
                return Ok(());
            }
            with_internet_settings(|h| previous.restore(h))?;
            set_all_connections(previous.per_connection_settings())?;
            previous.restore_environment_if_owned()?;
            notify_changed()?;
            notify_environment_changed();
            return Ok(());
        }

        // Compatibility path for pre-snapshot ownership markers. Disable only
        // when the current endpoint still matches the marker; with no marker,
        // retain the legacy best-effort behaviour.
        let expected = snapshot
            .map(|snapshot| snapshot.detail.trim())
            .filter(|s| !s.is_empty());
        let disabled = with_internet_settings(|h| {
            if let Some(expected) = expected {
                let enabled = query_dword(h, "ProxyEnable")?.unwrap_or(0) != 0;
                let current = query_sz(h, "ProxyServer")?.unwrap_or_default();
                if !enabled || !proxy_server_is_exact_endpoint_value(&current, expected) {
                    return Ok(false);
                }
            }
            set_dword(h, "ProxyEnable", 0)?;
            Ok(true)
        })?;
        if !disabled {
            return Ok(());
        }
        set_all_connections(PerConnectionSettings {
            flags: PROXY_TYPE_DIRECT,
            proxy: None,
            bypass: None,
            pac: None,
        })?;
        if let Some(expected) = expected {
            clear_environment_for_endpoint(expected)?;
        }
        notify_changed()?;
        notify_environment_changed();
        Ok(())
    }

    fn refresh(&self, host: &str, port: u16) -> AppResult<()> {
        let server = format!("{host}:{port}");
        let bypass = with_internet_settings(|h| {
            Ok(query_sz(h, "ProxyOverride")?.unwrap_or_else(|| "<local>".into()))
        })?;
        set_all_connections(satelite_connection_settings(&server, &bypass))?;
        // Refresh may run after a core restart. Keep CLI proxy variables in
        // sync too, without changing the original restore snapshot.
        let no_proxy =
            with_user_environment(|h| Ok(merge_no_proxy(query_sz(h, "NO_PROXY")?.as_deref())))?;
        let refresh_snapshot = WindowsProxySnapshot {
            applied_server: server,
            proxy_enable: None,
            proxy_server: None,
            proxy_override: None,
            auto_config_url: None,
            auto_detect: None,
            env_http_proxy: None,
            env_https_proxy: None,
            env_all_proxy: None,
            env_no_proxy: None,
            applied_no_proxy: Some(no_proxy),
        };
        apply_proxy_environment(&refresh_snapshot)?;
        notify_changed()?;
        notify_environment_changed();
        Ok(())
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
            auto_detect: Some(1),
            env_http_proxy: Some("http://127.0.0.1:10808".into()),
            env_https_proxy: Some("http://127.0.0.1:10808".into()),
            env_all_proxy: Some("socks5://127.0.0.1:10808".into()),
            env_no_proxy: Some("localhost".into()),
            applied_no_proxy: Some("localhost,127.0.0.1,::1".into()),
        };
        let encoded = snapshot.encode().unwrap();
        assert_eq!(WindowsProxySnapshot::decode(&encoded), Some(snapshot));
    }

    #[test]
    fn no_proxy_merge_preserves_existing_and_adds_loopback() {
        assert_eq!(
            super::merge_no_proxy(Some("example.test,LOCALHOST")),
            "example.test,LOCALHOST,127.0.0.1,::1"
        );
    }
}
