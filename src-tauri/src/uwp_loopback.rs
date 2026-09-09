//! Windows UWP/AppContainer loopback exemptions.
//!
//! Store-packaged applications are not allowed to connect to local loopback
//! endpoints by default.  That prevents them from reaching a local HTTP/SOCKS
//! proxy even when WinINet is configured correctly.  v2rayN's
//! `EnableLoopback.exe` solves the same problem through Windows'
//! `CheckNetIsolation.exe` utility.
//!
//! This module keeps the operation explicit and additive: it enumerates the
//! current user's AppContainer mapping SIDs and adds each one to
//! `LoopbackExempt`.  It never clears existing exemptions and never changes
//! the system proxy or routing mode.

#![cfg(target_os = "windows")]

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::ffi::{c_void, OsStr};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const HELPER_MARKER: &str = "--satelite-uwp-loopback-helper";
const MAPPINGS_KEY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings";
const RESULT_FILE_NAME: &str = "uwp-loopback-result.json";
const RESULT_TMP_FILE_NAME: &str = "uwp-loopback-result.json.tmp";

type Hkey = *mut c_void;
type Lpcwstr = *const u16;
type Lpwstr = *mut u16;
type Dword = u32;
type Long = i32;

const HKEY_CURRENT_USER: Hkey = 0x8000_0001usize as Hkey;
const KEY_READ: Dword = 0x0002_0019;
const ERROR_SUCCESS: Long = 0;
const ERROR_FILE_NOT_FOUND: Long = 2;
const ERROR_NO_MORE_ITEMS: Long = 259;
const ERROR_MORE_DATA: Long = 234;

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(
        h_key: Hkey,
        sub_key: Lpcwstr,
        options: Dword,
        desired: Dword,
        result: *mut Hkey,
    ) -> Long;
    fn RegEnumKeyExW(
        h_key: Hkey,
        index: Dword,
        name: Lpwstr,
        name_len: *mut Dword,
        reserved: *mut Dword,
        class_name: Lpwstr,
        class_len: *mut Dword,
        last_write_time: *mut c_void,
    ) -> Long;
    fn RegCloseKey(h_key: Hkey) -> Long;
}

/// Result returned to the settings page after the elevated helper finishes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UwpLoopbackResult {
    pub packages: usize,
    pub applied: usize,
    pub failed: usize,
    #[serde(default)]
    pub error: Option<String>,
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Read the package AppContainer SIDs from the current user's registry hive.
/// The mapping key is the same source used by the v2rayN helper's bulk mode.
fn app_container_sids() -> AppResult<Vec<String>> {
    let key_name = wide(MAPPINGS_KEY);
    let mut key: Hkey = std::ptr::null_mut();
    let result =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key_name.as_ptr(), 0, KEY_READ, &mut key) };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(Vec::new());
    }
    if result != ERROR_SUCCESS {
        return Err(AppError::Config(format!(
            "打开 UWP AppContainer 映射失败（错误码 {result}）"
        )));
    }

    let mut names = Vec::new();
    let mut index = 0;
    loop {
        // AppContainer package SIDs are short, but leave enough room for a
        // future Windows mapping name without allocating per registry row.
        let mut buffer = [0u16; 512];
        let mut length = (buffer.len() - 1) as Dword;
        let status = unsafe {
            RegEnumKeyExW(
                key,
                index,
                buffer.as_mut_ptr(),
                &mut length,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status == ERROR_MORE_DATA {
            unsafe { RegCloseKey(key) };
            return Err(AppError::Config(
                "UWP AppContainer 映射名称超出预期长度".into(),
            ));
        }
        if status != ERROR_SUCCESS {
            unsafe { RegCloseKey(key) };
            return Err(AppError::Config(format!(
                "读取 UWP AppContainer 映射失败（错误码 {status}）"
            )));
        }
        let name = String::from_utf16_lossy(&buffer[..length as usize]);
        // Package AppContainer SIDs use this authority/sub-authority prefix;
        // filtering prevents unrelated registry mappings from being passed to
        // the isolation utility.
        if name
            .get(..8)
            .map(|prefix| prefix.eq_ignore_ascii_case("S-1-15-2"))
            .unwrap_or(false)
            && name[8..].chars().all(|c| c == '-' || c.is_ascii_digit())
        {
            names.push(name);
        }
        index += 1;
    }
    unsafe { RegCloseKey(key) };
    names.sort_unstable();
    names.dedup();
    Ok(names)
}

fn check_net_isolation_path() -> PathBuf {
    std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("CheckNetIsolation.exe")
}

fn apply_one(check_net_isolation: &Path, sid: &str) -> Result<(), String> {
    let parameter = format!("-p={sid}");
    let output = Command::new(check_net_isolation)
        .args(["LoopbackExempt", "-a", parameter.as_str()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("启动 CheckNetIsolation 失败：{error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    Err(if detail.is_empty() {
        format!("CheckNetIsolation 退出码 {}", output.status)
    } else {
        detail
    })
}

fn write_result(path: &Path, result: &UwpLoopbackResult) -> Result<(), String> {
    let raw =
        serde_json::to_vec(result).map_err(|error| format!("序列化 UWP 结果失败：{error}"))?;
    let tmp = path.with_file_name(RESULT_TMP_FILE_NAME);
    fs::write(&tmp, raw).map_err(|error| format!("写入 UWP 结果失败：{error}"))?;
    fs::rename(&tmp, path).map_err(|error| format!("提交 UWP 结果失败：{error}"))
}

fn read_result(path: &Path) -> AppResult<UwpLoopbackResult> {
    let raw = fs::read(path)
        .map_err(|error| AppError::Core(format!("读取 UWP 回环结果失败：{error}")))?;
    serde_json::from_slice(&raw)
        .map_err(|error| AppError::Core(format!("解析 UWP 回环结果失败：{error}")))
}

fn quote_windows_arg(value: &Path) -> String {
    // The generated path never contains a quote, and the app-data path does
    // not end with a backslash.  Doubling embedded quotes keeps this helper
    // safe even if a custom portable path is unusual.
    format!("\"{}\"", value.to_string_lossy().replace('"', "\\\""))
}

/// Ask Windows for one UAC elevation and apply all current-user UWP entries.
/// This function blocks only its caller; the Tauri command runs it on a
/// blocking worker so the webview remains responsive while the prompt runs.
pub fn enable(app_data_dir: &Path) -> AppResult<UwpLoopbackResult> {
    let data_dir = app_data_dir.join("data");
    fs::create_dir_all(&data_dir)?;
    let result_path = data_dir.join(RESULT_FILE_NAME);
    let _ = fs::remove_file(&result_path);
    let _ = fs::remove_file(data_dir.join(RESULT_TMP_FILE_NAME));

    let helper = std::env::current_exe()
        .map_err(|error| AppError::Core(format!("定位 Satelite 程序失败：{error}")))?;
    let args = format!("{HELPER_MARKER} {}", quote_windows_arg(&result_path));
    let child = crate::core::elevate::run_elevated_with_cancel_message(
        &helper,
        &args,
        None,
        "已取消管理员授权，未修改 UWP 回环豁免。",
    )?;
    let exit_code = child.wait_for_exit(60_000)?;
    let result = read_result(&result_path).or_else(|error| {
        if exit_code == 0 {
            Err(error)
        } else {
            Err(AppError::Core(format!(
                "UWP 回环助手失败（退出码 {exit_code}）：{error}"
            )))
        }
    })?;
    let _ = fs::remove_file(&result_path);
    let _ = fs::remove_file(data_dir.join(RESULT_TMP_FILE_NAME));
    Ok(result)
}

/// Entry point used by the elevated copy of Satelite.  It runs before Tauri
/// initialization, so the helper has no second window or tray instance.
pub fn try_run_helper() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    let marker = args.iter().position(|value| value == HELPER_MARKER)?;
    if args.len() != marker + 2 {
        return Some(2);
    }
    let result_path = PathBuf::from(&args[marker + 1]);
    if result_path.file_name().and_then(|name| name.to_str()) != Some(RESULT_FILE_NAME)
        || result_path
            .parent()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            != Some("data")
    {
        return Some(2);
    }
    let result = match app_container_sids() {
        Ok(sids) => {
            let check_net_isolation = check_net_isolation_path();
            if !check_net_isolation.is_file() {
                UwpLoopbackResult {
                    packages: sids.len(),
                    applied: 0,
                    failed: sids.len(),
                    error: Some(format!(
                        "找不到 Windows 工具：{}",
                        check_net_isolation.display()
                    )),
                }
            } else {
                let mut applied = 0;
                let mut failed = 0;
                let mut first_error = None;
                for sid in &sids {
                    match apply_one(&check_net_isolation, sid) {
                        Ok(()) => applied += 1,
                        Err(error) => {
                            failed += 1;
                            if first_error.is_none() {
                                first_error = Some(error);
                            }
                        }
                    }
                }
                UwpLoopbackResult {
                    packages: sids.len(),
                    applied,
                    failed,
                    error: first_error,
                }
            }
        }
        Err(error) => UwpLoopbackResult {
            packages: 0,
            applied: 0,
            failed: 0,
            error: Some(error.to_string()),
        },
    };
    if write_result(&result_path, &result).is_err() {
        return Some(3);
    }
    // The parent displays partial failures from the result file; a non-zero
    // helper exit is reserved for a result that could not be written at all.
    Some(0)
}
