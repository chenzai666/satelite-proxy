//! Bind child processes to a Windows Job Object so they die with the parent.
//!
//! Problem: when the main app exits abnormally (crash, killed by the installer
//! during an upgrade, Task Manager End Task, ...), the normal shutdown path
//! that stops sing-box never runs. sing-box keeps running as an orphan and
//! holds the mixed/api ports, so the next launch fails with "port in use".
//!
//! Fix: create a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and assign
//! every spawned sing-box child to it. The job handle lives for the whole
//! process lifetime. When the process dies for *any* reason, Windows closes
//! the handle, and the kernel kills every process in the job — including
//! sing-box. Orphan ports become impossible.
//!
//! To avoid depending on a versioned `windows-sys` crate, we declare the
//! handful of Win32 FFI symbols we need directly. The job handle is wrapped in
//! a `Send`-only newtype stored behind a `Mutex` (we never send it across
//! threads after creation, but the wrapper silences the Send/Sync lint).

// Field names mirror the Win32 `IO_COUNTERS` /
// `JOBOBJECT_*_LIMIT_INFORMATION` layouts exactly (the structs are passed to
// the OS by pointer), so the snake_case / camel_case lints are intentionally
// silenced here.
#![cfg(target_os = "windows")]
#![allow(non_snake_case, non_camel_case_types)]

use core::ffi::c_void as CVoid;

use std::io;
use std::sync::Mutex;

// --- Minimal Win32 FFI -------------------------------------------------------

type BOOL = i32;
type HANDLE = *mut CVoid;
type LPVOID = *mut CVoid;
type DWORD = u32;

#[repr(C)]
#[derive(Default)]
struct IO_COUNTERS {
    ReadOperationCount: u64,
    WriteOperationCount: u64,
    OtherOperationCount: u64,
    ReadTransferCount: u64,
    WriteTransferCount: u64,
    OtherTransferCount: u64,
}

#[repr(C)]
#[derive(Default)]
struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
    PerProcessUserTimeLimit: i64,
    PerJobUserTimeLimit: i64,
    LimitFlags: DWORD,
    MinimumWorkingSetSize: usize,
    MaximumWorkingSetSize: usize,
    ActiveProcessLimit: DWORD,
    Affinity: usize,
    PriorityClass: DWORD,
    SchedulingClass: DWORD,
}

#[repr(C)]
#[derive(Default)]
struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
    BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION,
    IoInfo: IO_COUNTERS,
    ProcessMemoryLimit: usize,
    JobMemoryLimit: usize,
    PeakProcessMemoryUsed: usize,
    PeakJobMemoryUsed: usize,
}

const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: DWORD = 0x2000;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: DWORD = 9;
const PROCESS_SET_QUOTA: DWORD = 0x0100;
const PROCESS_TERMINATE: DWORD = 0x0001;

#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(lpJobAttributes: LPVOID, lpName: LPVOID) -> HANDLE;
    fn SetInformationJobObject(
        hJob: HANDLE,
        JobObjectInfoClass: DWORD,
        lpJobObjectInfo: LPVOID,
        cbJobObjectInfoLength: DWORD,
    ) -> BOOL;
    fn AssignProcessToJobObject(hJob: HANDLE, hProcess: HANDLE) -> BOOL;
    fn OpenProcess(dwDesiredAccess: DWORD, bInheritHandle: BOOL, dwProcessId: DWORD) -> HANDLE;
    fn CloseHandle(hObject: HANDLE) -> BOOL;
}

// --- Job handle storage ------------------------------------------------------

/// We only ever touch the handle from one place at a time, but a raw pointer
/// isn't `Sync`, so wrap it. The Mutex also serializes the create-once path.
struct JobSlot(Mutex<Option<Handle>>);
struct Handle(HANDLE);

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

static JOB: JobSlot = JobSlot(Mutex::new(None));

/// Create the singleton job (if not already) and assign `child_pid` to it.
/// On any failure we log via the returned Err; the child still runs — graceful
/// degradation (a running proxy beats a killed one).
pub fn ensure_child_killed_on_parent_exit(child_pid: u32) -> io::Result<()> {
    // Ensure job exists.
    {
        let mut guard = JOB.0.lock().unwrap();
        if guard.is_none() {
            let h = create_kill_on_close_job()?;
            *guard = Some(Handle(h));
        }
    }
    // Assign child. Safe to unwrap: we just guaranteed Some above.
    let guard = JOB.0.lock().unwrap();
    let job = guard.as_ref().unwrap().0;
    assign_pid_to_job(job, child_pid)
}

fn create_kill_on_close_job() -> io::Result<HANDLE> {
    unsafe {
        let h = CreateJobObjectW(core::ptr::null_mut(), core::ptr::null_mut());
        if h.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = core::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            h,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
            &info as *const _ as LPVOID,
            core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as DWORD,
        );
        if ok == 0 {
            let e = io::Error::last_os_error();
            CloseHandle(h);
            return Err(e);
        }
        Ok(h)
    }
}

fn assign_pid_to_job(job: HANDLE, pid: u32) -> io::Result<()> {
    unsafe {
        let proc = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if proc.is_null() {
            return Err(io::Error::last_os_error());
        }
        let ok = AssignProcessToJobObject(job, proc);
        CloseHandle(proc);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
