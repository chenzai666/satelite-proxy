//! Resident-memory (RSS) reads for the core process — no subprocesses on the
//! platforms where it matters.
//!
//! The dashboard polls this every few seconds, so the implementation matters:
//! the previous version shelled out to `ps` / `tasklist` on every cache miss,
//! which forked a console child (a visible window flash on Windows, where the
//! parent is a GUI-subsystem process) and parsed locale-dependent text.
//!
//! Per-OS strategy:
//! - Windows: `NtQuerySystemInformation(SystemProcessInformation)` — a single
//!   syscall over the kernel's process table, no handle required (so it still
//!   works when sing-box runs elevated for TUN and opening a handle would be
//!   denied), nothing to parse. This is the same table Task Manager shows.
//! - Linux: `/proc/<pid>/status` `VmRSS` — one file read; the kernel reports
//!   kB so there is no page-size arithmetic.
//! - macOS: `ps -o rss=`. A subprocess here is deliberate: a setuid-root
//!   sing-box's task port cannot be obtained by its unprivileged parent
//!   (`sysinfo` / `proc_pid_rusage` come back empty or denied), while `ps`
//!   reads the kernel process table, which is not privilege-gated. macOS also
//!   has no console-window problem to hide.

/// Resident set size of `pid`, in bytes. `None` when it cannot be determined
/// (process gone, or the OS surface denied us).
pub fn read_process_rss_bytes(pid: u32) -> Option<u64> {
    read_rss(pid)
}

/// Aggregate memory of the app's own WebView process tree (Windows: the six
/// msedgewebview2.exe processes; other platforms: `None`). Surfaced in the
/// settings core tab so memory regressions are visible without external tools.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct WebViewTreeMemory {
    pub process_count: u32,
    pub total_ws_bytes: u64,
    pub total_private_bytes: u64,
}

pub fn read_webview_tree_memory() -> Option<WebViewTreeMemory> {
    read_webview_tree()
}

#[cfg(target_os = "windows")]
fn read_webview_tree() -> Option<WebViewTreeMemory> {
    use windows::Wdk::System::SystemInformation::{
        NtQuerySystemInformation, SystemProcessInformation,
    };
    use windows::Win32::System::WindowsProgramming::SYSTEM_PROCESS_INFORMATION;

    struct ProcEntry {
        pid: u32,
        parent: u32,
        name: String,
        ws: u64,
        private: u64,
    }

    let mut len = 256 * 1024usize;
    for _ in 0..4 {
        let mut buf = vec![0u64; len / 8];
        let mut needed = 0u32;
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buf.as_mut_ptr().cast(),
                len as u32,
                &mut needed,
            )
        };
        if status.0 >= 0 {
            let base = buf.as_ptr().cast::<u8>();
            let mut offset = 0usize;
            let mut entries: Vec<ProcEntry> = Vec::new();
            loop {
                let entry =
                    unsafe { &*base.add(offset).cast::<SYSTEM_PROCESS_INFORMATION>() };
                // ImageName buffers live inside this same table allocation,
                // so the read is valid while `buf` is alive.
                let name = unsafe {
                    let s = &entry.ImageName;
                    if s.Buffer.is_null() || s.Length == 0 {
                        String::new()
                    } else {
                        let chars = (s.Length as usize / 2).min(256);
                        let slice =
                            std::slice::from_raw_parts(s.Buffer.as_ptr(), chars);
                        String::from_utf16_lossy(slice).to_ascii_lowercase()
                    }
                };
                entries.push(ProcEntry {
                    pid: entry.UniqueProcessId.0 as u32,
                    // windows-rs folds InheritedFromUniqueProcessId (a
                    // ULONG_PTR at this exact NT offset) into an opaque
                    // Reserved2 pointer slot — the ABI position is fixed.
                    parent: entry.Reserved2 as u32,
                    name,
                    ws: entry.WorkingSetSize as u64,
                    private: entry.PrivatePageCount as u64,
                });
                let next = entry.NextEntryOffset as usize;
                if next == 0 {
                    break;
                }
                offset += next;
            }

            // Descendants of the current process (walks the parent chain).
            let self_pid = std::process::id();
            let mut report = WebViewTreeMemory::default();
            for entry in &entries {
                if !entry.name.contains("msedgewebview2") {
                    continue;
                }
                let mut cursor = entry.pid;
                let mut belongs = false;
                for _ in 0..16 {
                    if cursor == self_pid {
                        belongs = true;
                        break;
                    }
                    match entries.iter().find(|e| e.pid == cursor) {
                        Some(parent) => cursor = parent.parent,
                        None => break,
                    }
                }
                if belongs {
                    report.process_count += 1;
                    report.total_ws_bytes += entry.ws;
                    report.total_private_bytes += entry.private;
                }
            }
            return Some(report);
        }
        let want = needed as usize;
        if want <= len {
            return None;
        }
        len = want + 64 * 1024;
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn read_webview_tree() -> Option<WebViewTreeMemory> {
    None
}

#[cfg(target_os = "windows")]
fn read_rss(pid: u32) -> Option<u64> {
    use windows::Wdk::System::SystemInformation::{
        NtQuerySystemInformation, SystemProcessInformation,
    };
    use windows::Win32::System::WindowsProgramming::SYSTEM_PROCESS_INFORMATION;

    // The table for a few hundred processes is ~100–300 KB; start generous
    // and grow when the kernel reports STATUS_INFO_LENGTH_MISMATCH.
    let mut len = 256 * 1024usize;
    for _ in 0..4 {
        // u64 elements give the buffer the pointer alignment the NT structs
        // require (a Vec<u8> would only guarantee byte alignment).
        let mut buf = vec![0u64; len / 8];
        let mut needed = 0u32;
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buf.as_mut_ptr().cast(),
                len as u32,
                &mut needed,
            )
        };
        // NTSTATUS success is a non-negative severity (>= 0).
        if status.0 >= 0 {
            // Entries sit back-to-back; NextEntryOffset chains them and a
            // zero offset marks the last one. Offsets are pointer multiples,
            // so walking with cast pointers stays well-aligned. Walk the
            // buffer as raw bytes — `add` is in units of the pointee.
            let base = buf.as_ptr().cast::<u8>();
            let mut offset = 0usize;
            loop {
                let entry =
                    unsafe { &*base.add(offset).cast::<SYSTEM_PROCESS_INFORMATION>() };
                if entry.UniqueProcessId.0 as u32 == pid {
                    return Some(entry.WorkingSetSize as u64);
                }
                let next = entry.NextEntryOffset as usize;
                if next == 0 {
                    return None;
                }
                offset += next;
            }
        }
        // Buffer too small (or the table grew mid-query): retry with the size
        // the kernel asked for plus slack.
        let want = needed as usize;
        if want <= len {
            return None; // unexpected failure status, growing won't help
        }
        len = want + 64 * 1024;
    }
    None
}

#[cfg(target_os = "linux")]
fn read_rss(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    // "VmRSS:\t  12345 kB" — kernel-provided, locale-independent, always kB.
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.trim_end_matches("kB").trim().parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn read_rss(pid: u32) -> Option<u64> {
    // Subprocess on purpose — see the module docs for the privilege story.
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim().parse::<u64>().ok().map(|kb| kb * 1024)
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn read_rss(_pid: u32) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_own_process_rss() {
        let rss = read_process_rss_bytes(std::process::id());
        assert!(
            rss.unwrap_or(0) > 0,
            "own RSS should be readable, got {rss:?}"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn missing_pid_reports_none() {
        // Windows PIDs are multiples of 4, so u32::MAX can never be live.
        assert_eq!(read_process_rss_bytes(u32::MAX), None);
    }
}
