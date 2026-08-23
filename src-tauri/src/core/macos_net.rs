//! Detect the "system DNS bypasses TUN" failure mode (problem 4 in the bug
//! report): when the OS resolver's DNS server sits on the same subnet as the
//! default gateway (the common DHCP setup of "router doubles as DNS"), the
//! resolver's queries go out the physical interface's host route to that
//! subnet — a route more specific than anything `auto_route` adds — so they
//! never enter the tun and never get `hijack-dns`. The app cannot fix this
//! safely (see the bug report's two failed approaches: adding the gateway
//! /32 to `route_address` breaks the default route entirely, and AEWP can
//! silently fail to elevate on newer macOS). So we only detect and suggest;
//! we never touch the user's system DNS.
//!
//! The detection itself (`dns_bypasses_tun`) is a pure function over parsed
//! addresses — fully unit-testable without touching the network. Only the
//! two small readers at the bottom shell out (`route -n get default`,
//! `scutil --dns`), and callers on non-macOS platforms never invoke them.

use std::net::Ipv4Addr;
use std::process::Command;

/// One diagnosed issue to surface in the UI. Never auto-applied.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct NetDiagnostic {
    pub issue: String,
    pub suggestion: String,
}

/// True when `dns` sits on the same /24 as `gateway` (or is the gateway
/// itself). This is a heuristic for "DNS is answered by the router itself",
/// which is the shape of network that breaks TUN's DNS hijack: the OS
/// resolver's query to that address takes the physical interface's
/// subnet-local route, never the tun's default route.
///
/// A /24 is deliberately conservative (most home LANs are /24); a coarser
/// mask would flag setups where the DNS server is merely "somewhere on the
/// LAN" but not actually gateway-adjacent, which is not the failure mode
/// this detects.
pub fn same_subnet_24(a: Ipv4Addr, b: Ipv4Addr) -> bool {
    let a = a.octets();
    let b = b.octets();
    a[0] == b[0] && a[1] == b[1] && a[2] == b[2]
}

/// Diagnose whether any configured DNS server is gateway-adjacent (same /24
/// as the default gateway) — the condition under which TUN's `auto_route`
/// cannot pull system DNS queries into the tunnel. Returns `None` when no
/// configured DNS server matches (including when either input is missing —
/// nothing to diagnose without both).
pub fn dns_bypasses_tun(
    gateway: Option<Ipv4Addr>,
    dns_servers: &[Ipv4Addr],
) -> Option<NetDiagnostic> {
    let gateway = gateway?;
    let hit = dns_servers
        .iter()
        .find(|dns| same_subnet_24(**dns, gateway))?;
    Some(NetDiagnostic {
        issue: format!(
            "检测到系统 DNS（{hit}）与默认网关（{gateway}）同网段，TUN 模式下这类 DNS 查询会绕过隧道直接走物理网卡，\
             可能导致域名解析结果被污染（连接建立但无响应、只上传无下载）。"
        ),
        suggestion: "建议手动把当前网络的 DNS 服务器改为公共解析器（如 223.5.5.5 / 119.29.29.29 / 1.1.1.1），\
                     系统偏好设置 → 网络 → 你的网络服务 → 高级 → DNS。应用不会自动修改此设置。"
            .into(),
    })
}

/// Read the current default gateway (`route -n get default`). macOS-only;
/// best-effort — returns `None` on any parse/exec failure rather than erroring,
/// since this only ever backs an optional UI hint.
#[cfg(target_os = "macos")]
pub fn read_default_gateway() -> Option<Ipv4Addr> {
    let out = Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_route_gateway(&text)
}

/// Read the DNS servers for the currently active resolver (`scutil --dns`,
/// first `nameserver[...]` block — that's the resolver actually consulted for
/// unqualified lookups). macOS-only, best-effort like `read_default_gateway`.
#[cfg(target_os = "macos")]
pub fn read_system_dns_servers() -> Vec<Ipv4Addr> {
    let Ok(out) = Command::new("scutil").arg("--dns").output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_scutil_dns(&text)
}

/// Best-effort diagnosis using the live system state. `None` when the
/// gateway can't be determined or no DNS server is gateway-adjacent.
#[cfg(target_os = "macos")]
pub fn diagnose_system_dns_bypass() -> Option<NetDiagnostic> {
    let gateway = read_default_gateway()?;
    let dns = read_system_dns_servers();
    dns_bypasses_tun(Some(gateway), &dns)
}

/// Parse the gateway address out of `route -n get default` output.
///
/// Expected line shape (macOS):
/// ```text
///    route to: default
/// destination: default
///        mask: default
///     gateway: 192.168.10.1
///   interface: en1
/// ```
fn parse_route_gateway(text: &str) -> Option<Ipv4Addr> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("gateway:") {
            if let Ok(addr) = rest.trim().parse::<Ipv4Addr>() {
                return Some(addr);
            }
        }
    }
    None
}

/// Parse nameserver addresses out of the first resolver block in
/// `scutil --dns` output.
///
/// Expected shape (macOS):
/// ```text
/// DNS configuration
///
/// resolver #1
///   nameserver[0] : 192.168.10.1
///   nameserver[1] : 8.8.8.8
///   ...
/// resolver #2
///   ...
/// ```
/// Only resolver #1 is read: it's the one consulted for plain (unscoped,
/// unqualified) lookups, which is what the OS resolver actually uses.
fn parse_scutil_dns(text: &str) -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    let mut in_first_resolver = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("resolver #") {
            // Only #1; stop collecting once a later resolver block starts.
            in_first_resolver = trimmed == "resolver #1";
            if !in_first_resolver && !out.is_empty() {
                break;
            }
            continue;
        }
        if !in_first_resolver {
            continue;
        }
        if let Some(idx) = trimmed.find(':') {
            let key = trimmed[..idx].trim();
            if key.starts_with("nameserver[") {
                let value = trimmed[idx + 1..].trim();
                if let Ok(addr) = value.parse::<Ipv4Addr>() {
                    out.push(addr);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_dns_on_same_subnet_as_gateway() {
        let gateway: Ipv4Addr = "192.168.10.1".parse().unwrap();
        let dns = vec![gateway]; // classic "router doubles as DNS" DHCP setup
        let diag = dns_bypasses_tun(Some(gateway), &dns);
        assert!(diag.is_some());
        let diag = diag.unwrap();
        assert!(diag.issue.contains("192.168.10.1"));
        assert!(diag.suggestion.contains("223.5.5.5"));
    }

    #[test]
    fn detects_dns_gateway_adjacent_but_not_identical() {
        // DNS is a different host on the same /24 as the gateway (e.g. a
        // Pi-hole at .5 while the router is .1) — still gateway-adjacent,
        // still takes the physical interface's subnet-local route.
        let gateway: Ipv4Addr = "192.168.10.1".parse().unwrap();
        let dns: Ipv4Addr = "192.168.10.5".parse().unwrap();
        assert!(dns_bypasses_tun(Some(gateway), &[dns]).is_some());
    }

    #[test]
    fn public_dns_on_a_different_subnet_is_not_flagged() {
        let gateway: Ipv4Addr = "192.168.10.1".parse().unwrap();
        let dns: Ipv4Addr = "223.5.5.5".parse().unwrap();
        assert_eq!(dns_bypasses_tun(Some(gateway), &[dns]), None);
    }

    #[test]
    fn missing_gateway_or_empty_dns_list_is_not_flagged() {
        let gateway: Ipv4Addr = "192.168.10.1".parse().unwrap();
        assert_eq!(dns_bypasses_tun(None, &[gateway]), None);
        assert_eq!(dns_bypasses_tun(Some(gateway), &[]), None);
    }

    #[test]
    fn mixed_dns_list_flags_when_any_entry_is_gateway_adjacent() {
        // Multiple configured DNS servers (common: router + a public
        // fallback) — flag as soon as one of them is gateway-adjacent,
        // since the OS may query either.
        let gateway: Ipv4Addr = "10.0.0.1".parse().unwrap();
        let public: Ipv4Addr = "1.1.1.1".parse().unwrap();
        let router: Ipv4Addr = "10.0.0.1".parse().unwrap();
        let diag = dns_bypasses_tun(Some(gateway), &[public, router]);
        assert!(diag.is_some());
    }

    #[test]
    fn same_subnet_24_matches_only_the_first_three_octets() {
        let a: Ipv4Addr = "192.168.10.1".parse().unwrap();
        let b: Ipv4Addr = "192.168.10.254".parse().unwrap();
        let c: Ipv4Addr = "192.168.11.1".parse().unwrap();
        assert!(same_subnet_24(a, b));
        assert!(!same_subnet_24(a, c));
    }

    #[test]
    fn parses_gateway_from_route_get_default_output() {
        let sample = "   route to: default\ndestination: default\n       mask: default\n    gateway: 192.168.10.1\n  interface: en1\n";
        assert_eq!(
            parse_route_gateway(sample),
            Some("192.168.10.1".parse().unwrap())
        );
    }

    #[test]
    fn parses_gateway_returns_none_on_unexpected_output() {
        assert_eq!(parse_route_gateway("no gateway here\n"), None);
        assert_eq!(parse_route_gateway(""), None);
    }

    #[test]
    fn parses_dns_servers_from_first_resolver_block_only() {
        let sample = "DNS configuration\n\n\
resolver #1\n  nameserver[0] : 192.168.10.1\n  nameserver[1] : 8.8.8.8\n\n\
resolver #2\n  nameserver[0] : 10.0.0.53\n";
        let dns = parse_scutil_dns(sample);
        assert_eq!(
            dns,
            vec![
                "192.168.10.1".parse::<Ipv4Addr>().unwrap(),
                "8.8.8.8".parse::<Ipv4Addr>().unwrap(),
            ]
        );
    }

    #[test]
    fn parses_dns_servers_returns_empty_on_no_resolver_blocks() {
        assert_eq!(
            parse_scutil_dns("DNS configuration\n\nNo results found\n"),
            Vec::<Ipv4Addr>::new()
        );
    }
}
