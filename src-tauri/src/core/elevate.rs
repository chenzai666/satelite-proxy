//! Launch an elevated (admin) child process on Windows via UAC, returning a
//! process handle we can track. Used for TUN mode, which needs admin rights to
//! create the virtual network adapter.
//!
//! `ShellExecuteW("runas", ...)` triggers the UAC consent dialog. With
//! `SEE_MASK_NOCLOSEPROCESS` we get back an `hProcess` we can wait on / poll /
//! close — same lifecycle surface as a normal `Command::spawn` child, just
//! running elevated.

// Field names mirror the Win32 `SHELLEXECUTEINFOW` layout exactly (the struct
// is passed to the OS by pointer), so the snake_case lint is intentionally
// silenced here.
#![cfg(target_os = "windows")]
#![allow(non_snake_case)]

use crate::error::{AppError, AppResult};
use core::ffi::c_void as CVoid;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

type BOOL = i32;
type DWORD = u32;
type HANDLE = *mut CVoid;
type HWND = HANDLE;
type HINSTANCE = HANDLE;
type LPCWSTR = *const u16;

const SEE_MASK_NOCLOSEPROCESS: DWORD = 0x0000_0040;
const SW_HIDE: i32 = 0;
const STILL_ACTIVE: DWORD = 259;
const SYNCHRONIZE: DWORD = 0x0010_0000;
const PROCESS_QUERY_LIMITED_INFORMATION: DWORD = 0x1000;

#[repr(C)]
#[derive(Default)]
struct SHELLEXECUTEINFOW {
    cbSize: DWORD,
    fMask: ULONG,
    hwnd: HWND,
    lpVerb: LPCWSTR,
    lpFile: LPCWSTR,
    lpParameters: LPCWSTR,
    lpDirectory: LPCWSTR,
    nShow: i32,
    hInstApp: HINSTANCE,
    lpIDList: *mut CVoid,
    lpClass: LPCWSTR,
    hkeyClass: HANDLE,
    dwHotKey: DWORD,
    hIcon: HANDLE,
    hProcess: HANDLE,
}

// ULONG must come before SHELLEXECUTEINFOW uses it.
type ULONG = u32;

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteExW(lpExecInfo: *mut SHELLEXECUTEINFOW) -> BOOL;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetExitCodeProcess(hProcess: HANDLE, lpExitCode: *mut DWORD) -> BOOL;
    fn CloseHandle(hObject: HANDLE) -> BOOL;
    fn GetProcessId(Process: HANDLE) -> DWORD;
    fn OpenProcess(dwDesiredAccess: DWORD, bInheritHandle: BOOL, dwProcessId: DWORD) -> HANDLE;
    fn TerminateProcess(hProcess: HANDLE, uExitCode: u32) -> BOOL;
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Result of launching an elevated process: the OS handle (for polling/closing)
/// and the PID (for kill / identity).
pub struct ElevatedChild {
    pub handle: HANDLE,
    pub pid: u32,
}

impl Drop for ElevatedChild {
    fn drop(&mut self) {
        // We own the handle returned with SEE_MASK_NOCLOSEPROCESS; close it on drop.
        unsafe { CloseHandle(self.handle) };
    }
}
unsafe impl Send for ElevatedChild {}
unsafe impl Sync for ElevatedChild {}

/// Launch `binary` with `args` elevated (UAC prompt). `working_dir` may be empty.
/// Returns the process handle + PID on success.
///
/// `cancelled` in the error message is set when the user dismissed UAC.
pub fn run_elevated(
    binary: &Path,
    args: &str,
    working_dir: Option<&Path>,
) -> AppResult<ElevatedChild> {
    let verb = wide("runas");
    let file = wide(&binary.to_string_lossy());
    let params = wide(args);
    let dir = wide(
        &working_dir
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as DWORD;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.hwnd = core::ptr::null_mut();
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.lpDirectory = if dir.is_empty() {
        core::ptr::null()
    } else {
        dir.as_ptr()
    };
    info.nShow = SW_HIDE; // sing-box is a console app; we hide any flash.

    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        let err = std::io::Error::last_os_error();
        // ERROR_CANCELLED (1223) = user clicked No on the UAC prompt.
        let cancelled = err.raw_os_error() == Some(1223);
        let msg = if cancelled {
            "已取消管理员授权。TUN 模式需要管理员权限以创建虚拟网卡。".to_string()
        } else {
            format!("请求管理员权限失败 (UAC): {err}")
        };
        return Err(AppError::Core(msg));
    }
    if info.hProcess.is_null() {
        return Err(AppError::Core(
            "UAC succeeded but no process handle returned".into(),
        ));
    }

    let pid = unsafe { GetProcessId(info.hProcess) };
    Ok(ElevatedChild {
        handle: info.hProcess,
        pid,
    })
}

/// Poll whether a process PID is still running (Windows).
pub fn pid_alive(pid: u32) -> bool {
    unsafe {
        let h = OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false; // can't open → not running (or no access)
        }
        let mut code: DWORD = 0;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        // GetExitCodeProcess returns STILL_ACTIVE (259) for a running process.
        ok != 0 && code == STILL_ACTIVE
    }
}

/// Terminate a process by PID. The caller (this app) launched the elevated
/// sing-box via ShellExecuteEx, which grants us PROCESS_TERMINATE access on it
/// even though the child runs with a higher integrity level. We open the handle
/// directly and call TerminateProcess — no second UAC prompt needed.
///
/// Returns true if the process was (or already is) gone.
pub fn terminate_pid(pid: u32) -> bool {
    const PROCESS_TERMINATE: DWORD = 0x0001;
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if h.is_null() {
            // Can't open — either already dead or we lack access. Treat as gone
            // so the caller proceeds (force_free_port will re-verify).
            return !pid_alive(pid);
        }
        // TerminateProcess returns nonzero on success. If the process already
        // exited, OpenProcess may still succeed but TerminateProcess fails —
        // check liveness after.
        let ok = TerminateProcess(h, 1);
        CloseHandle(h);
        ok != 0 || !pid_alive(pid)
    }
}
