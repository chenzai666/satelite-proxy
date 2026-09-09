//! Detect applications that bypass the Windows system proxy.
//!
//! This module is deliberately read-only.  It samples the OS TCP table when
//! the generated proxy is running in system-proxy mode and reports public TCP
//! connections owned by processes other than Satelite and its cores.  A
//! process making such a connection may not understand the Windows system
//! proxy; the UI can then suggest TUN, but this module never enables it.

use crate::runtime::ProxyStatus;
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 32;

#[derive(Debug, Clone, Serialize, Default)]
pub struct ProxyBypassReport {
    /// Whether the current settings are eligible for this detection.
    pub active: bool,
    /// Whether the current platform has an implementation.
    pub supported: bool,
    /// Unix timestamp in milliseconds of the sample.
    pub sampled_at: i64,
    pub entries: Vec<ProxyBypassEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyBypassEntry {
    pub pid: u32,
    pub process: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub local: String,
    pub remote: String,
    pub network: String,
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

pub fn empty_report(active: bool) -> ProxyBypassReport {
    ProxyBypassReport {
        active,
        supported: cfg!(windows),
        sampled_at: now_millis(),
        entries: Vec::new(),
    }
}

/// Sample the OS connection table when a system-proxy bypass can matter.
pub fn detect(status: &ProxyStatus, managed_pids: &[u32]) -> ProxyBypassReport {
    let active = status.running
        && status.system_proxy
        && !status.tun_enabled
        && status.capture_mode.eq_ignore_ascii_case("system")
        && !status.outbound_mode.eq_ignore_ascii_case("direct")
        && !status.runtime_source.eq_ignore_ascii_case("singbox");

    if !active {
        return empty_report(false);
    }

    #[cfg(windows)]
    let entries = windows::collect(managed_pids);

    #[cfg(not(windows))]
    let entries = Vec::new();

    ProxyBypassReport {
        active: true,
        supported: cfg!(windows),
        sampled_at: now_millis(),
        entries,
    }
}

#[cfg(windows)]
mod windows {
    use super::{IpAddr, Ipv4Addr, Ipv6Addr, ProxyBypassEntry, MAX_ENTRIES};
    use std::collections::{HashMap, HashSet};
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::path::Path;
    use std::ptr::null_mut;

    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    const AF_INET: u32 = 2;
    const AF_INET6: u32 = 23;
    const TCP_TABLE_OWNER_PID_ALL: u32 = 5;
    const MIB_TCP_STATE_SYN_SENT: u32 = 3;
    const MIB_TCP_STATE_ESTAB: u32 = 5;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Tcp4Row {
        state: u32,
        local_addr: u32,
        local_port: u32,
        remote_addr: u32,
        remote_port: u32,
        owning_pid: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Tcp6Row {
        local_addr: [u8; 16],
        local_scope_id: u32,
        local_port: u32,
        remote_addr: [u8; 16],
        remote_scope_id: u32,
        remote_port: u32,
        state: u32,
        owning_pid: u32,
    }

    #[link(name = "iphlpapi")]
    extern "system" {
        fn GetExtendedTcpTable(
            table: *mut c_void,
            size: *mut u32,
            order: i32,
            address_family: u32,
            table_class: u32,
            reserved: u32,
        ) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;
        fn QueryFullProcessImageNameW(
            process: *mut c_void,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn CloseHandle(object: *mut c_void) -> i32;
    }

    fn read_table(address_family: u32) -> Option<Vec<u8>> {
        let mut size = 0u32;
        let first = unsafe {
            GetExtendedTcpTable(
                null_mut(),
                &mut size,
                0,
                address_family,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
        if first != 0 && first != ERROR_INSUFFICIENT_BUFFER {
            return None;
        }
        if size == 0 {
            return Some(Vec::new());
        }

        let mut buffer = vec![0u8; size as usize];
        loop {
            let mut actual_size = buffer.len() as u32;
            let result = unsafe {
                GetExtendedTcpTable(
                    buffer.as_mut_ptr().cast(),
                    &mut actual_size,
                    0,
                    address_family,
                    TCP_TABLE_OWNER_PID_ALL,
                    0,
                )
            };
            if result == 0 {
                buffer.truncate(actual_size as usize);
                return Some(buffer);
            }
            if result != ERROR_INSUFFICIENT_BUFFER || actual_size == 0 {
                return None;
            }
            buffer.resize(actual_size as usize, 0);
        }
    }

    fn row_count(buffer: &[u8], row_size: usize) -> usize {
        if buffer.len() < size_of::<u32>() || row_size == 0 {
            return 0;
        }
        let count = u32::from_ne_bytes(buffer[..4].try_into().unwrap()) as usize;
        count.min((buffer.len() - 4) / row_size)
    }

    fn tcp_port(raw: u32) -> u16 {
        u16::from_be(raw as u16)
    }

    fn process_info(pid: u32) -> (String, Option<String>) {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return (format!("PID {pid}"), None);
        }

        let mut buffer = vec![0u16; 32_768];
        let mut length = buffer.len() as u32;
        let ok =
            unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } != 0;
        unsafe {
            CloseHandle(handle);
        }
        if !ok || length == 0 || length as usize > buffer.len() {
            return (format!("PID {pid}"), None);
        }

        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        let process = Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("PID {pid}"));
        (process, Some(path))
    }

    fn is_ignored_process(process: &str) -> bool {
        matches!(
            process.to_ascii_lowercase().as_str(),
            "system" | "system idle process" | "registry" | "memory compression"
        )
    }

    fn is_public_ipv4(ip: Ipv4Addr) -> bool {
        let octets = ip.octets();
        if octets[0] == 0
            || ip.is_loopback()
            || ip.is_private()
            || ip.is_link_local()
            || ip.is_broadcast()
            || ip.is_multicast()
        {
            return false;
        }
        let value = u32::from_be_bytes(octets);
        // Carrier-grade NAT 100.64.0.0/10.
        if (value & 0xffc0_0000) == 0x6440_0000 {
            return false;
        }
        // Documentation, benchmarking, and special-use ranges.
        if (value & 0xffffff00) == 0xc000_0000
            || (value & 0xffffff00) == 0xc000_0200
            || (value & 0xfffffe00) == 0xc612_0000
            || (value & 0xffffff00) == 0xc633_6400
            || (value & 0xffffff00) == 0xcb00_7100
        {
            return false;
        }
        true
    }

    fn is_public_ipv6(ip: Ipv6Addr) -> bool {
        let segments = ip.segments();
        if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
            return false;
        }
        // Unique-local fc00::/7 and link-local fe80::/10.
        if (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80 {
            return false;
        }
        // Documentation range 2001:db8::/32.
        if segments[0] == 0x2001 && segments[1] == 0x0db8 {
            return false;
        }
        true
    }

    pub(super) fn is_public(ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(ip) => is_public_ipv4(ip),
            IpAddr::V6(ip) => is_public_ipv6(ip),
        }
    }

    fn format_endpoint(ip: IpAddr, port: u16) -> String {
        match ip {
            IpAddr::V4(ip) => format!("{ip}:{port}"),
            IpAddr::V6(ip) => format!("[{ip}]:{port}"),
        }
    }

    fn collect_v4(buffer: &[u8], entries: &mut Vec<(u32, IpAddr, u16, IpAddr, u16)>) {
        let count = row_count(buffer, size_of::<Tcp4Row>());
        for index in 0..count {
            let row = unsafe {
                buffer
                    .as_ptr()
                    .add(4 + index * size_of::<Tcp4Row>())
                    .cast::<Tcp4Row>()
                    .read_unaligned()
            };
            if !matches!(row.state, MIB_TCP_STATE_SYN_SENT | MIB_TCP_STATE_ESTAB) {
                continue;
            }
            let remote_port = tcp_port(row.remote_port);
            if remote_port == 0 {
                continue;
            }
            let local = IpAddr::V4(Ipv4Addr::from(row.local_addr.to_ne_bytes()));
            let remote = IpAddr::V4(Ipv4Addr::from(row.remote_addr.to_ne_bytes()));
            if is_public(remote) {
                entries.push((
                    row.owning_pid,
                    local,
                    tcp_port(row.local_port),
                    remote,
                    remote_port,
                ));
            }
        }
    }

    fn collect_v6(buffer: &[u8], entries: &mut Vec<(u32, IpAddr, u16, IpAddr, u16)>) {
        let count = row_count(buffer, size_of::<Tcp6Row>());
        for index in 0..count {
            let row = unsafe {
                buffer
                    .as_ptr()
                    .add(4 + index * size_of::<Tcp6Row>())
                    .cast::<Tcp6Row>()
                    .read_unaligned()
            };
            if !matches!(row.state, MIB_TCP_STATE_SYN_SENT | MIB_TCP_STATE_ESTAB) {
                continue;
            }
            let remote_port = tcp_port(row.remote_port);
            if remote_port == 0 {
                continue;
            }
            let local = IpAddr::V6(Ipv6Addr::from(row.local_addr));
            let remote = IpAddr::V6(Ipv6Addr::from(row.remote_addr));
            if is_public(remote) {
                entries.push((
                    row.owning_pid,
                    local,
                    tcp_port(row.local_port),
                    remote,
                    remote_port,
                ));
            }
        }
    }

    pub fn collect(managed_pids: &[u32]) -> Vec<ProxyBypassEntry> {
        let mut ignored_pids: HashSet<u32> = managed_pids.iter().copied().collect();
        ignored_pids.insert(std::process::id());

        let mut sockets = Vec::new();
        if let Some(buffer) = read_table(AF_INET) {
            collect_v4(&buffer, &mut sockets);
        }
        if let Some(buffer) = read_table(AF_INET6) {
            collect_v6(&buffer, &mut sockets);
        }

        let mut process_cache: HashMap<u32, (String, Option<String>)> = HashMap::new();
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for (pid, local_ip, local_port, remote_ip, remote_port) in sockets {
            if pid == 0 || ignored_pids.contains(&pid) {
                continue;
            }
            let (process, path) = process_cache
                .entry(pid)
                .or_insert_with(|| process_info(pid))
                .clone();
            if is_ignored_process(&process) {
                continue;
            }
            let remote = format_endpoint(remote_ip, remote_port);
            if !seen.insert((pid, remote.clone())) {
                continue;
            }
            result.push(ProxyBypassEntry {
                pid,
                process,
                path,
                local: format_endpoint(local_ip, local_port),
                remote,
                network: "tcp".into(),
            });
        }

        result.sort_by(|left, right| {
            left.process
                .to_ascii_lowercase()
                .cmp(&right.process.to_ascii_lowercase())
                .then_with(|| left.remote.cmp(&right.remote))
        });
        result.truncate(MAX_ENTRIES);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_status_does_not_report_bypasses() {
        let mut status = ProxyStatus {
            running: true,
            core_state: crate::core::manager::CoreState::Running,
            system_proxy: true,
            tun_enabled: false,
            capture_mode: "system".into(),
            outbound_mode: "direct".into(),
            mixed_port: 2080,
            api_port: 19090,
            current_node_id: None,
            error: None,
            core_path: None,
            config_path: None,
            upload_speed: 0,
            download_speed: 0,
            upload_total: 0,
            download_total: 0,
            connections: 0,
            smart_switch: false,
            auto_select: "off".into(),
            core_started_at: None,
            runtime_source: "generated".into(),
            runtime_profile_id: None,
            runtime_profile_name: None,
            custom_has_clash_api: false,
            custom_has_tun: false,
            custom_inbound_port: None,
            core_memory_bytes: None,
            core_type: "singbox".into(),
            core_elevated: false,
            sidecar_running: false,
        };
        let report = detect(&status, &[]);
        assert!(!report.active);
        assert!(report.entries.is_empty());

        status.outbound_mode = "rule".into();
        status.tun_enabled = true;
        let report = detect(&status, &[]);
        assert!(!report.active);
    }

    #[cfg(windows)]
    #[test]
    fn private_and_special_addresses_are_not_public() {
        assert!(!windows::is_public("127.0.0.1".parse().unwrap()));
        assert!(!windows::is_public("10.0.0.1".parse().unwrap()));
        assert!(!windows::is_public("100.64.0.1".parse().unwrap()));
        assert!(!windows::is_public("192.0.2.1".parse().unwrap()));
        assert!(windows::is_public("1.1.1.1".parse().unwrap()));
    }
}
