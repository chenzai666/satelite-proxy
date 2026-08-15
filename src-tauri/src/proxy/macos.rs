//! macOS system proxy via `networksetup`.

use super::{SystemProxy, SystemProxySnapshot};
use crate::error::{AppError, AppResult};
use std::process::Command;

#[derive(Default)]
pub struct MacSystemProxy;

#[derive(Debug, Default)]
struct ProxyProbe {
    enabled: bool,
    server: String,
    port: u16,
}

impl MacSystemProxy {
    fn services() -> AppResult<Vec<String>> {
        let out = Command::new("networksetup")
            .arg("-listallnetworkservices")
            .output()
            .map_err(|e| AppError::Core(format!("networksetup: {e}")))?;
        if !out.status.success() {
            return Err(AppError::Core("networksetup list services failed".into()));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut list = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('*') || line.contains("An asterisk") {
                continue;
            }
            // skip disabled marker lines like "*Ethernet" — actually disabled start with *
            list.push(line.to_string());
        }
        if list.is_empty() {
            // fallback common service
            list.push("Wi-Fi".into());
        }
        Ok(list)
    }

    fn run(args: &[&str]) -> AppResult<()> {
        let status = Command::new("networksetup")
            .args(args)
            .status()
            .map_err(|e| AppError::Core(format!("networksetup: {e}")))?;
        if !status.success() {
            return Err(AppError::Core(format!("networksetup {:?} failed", args)));
        }
        Ok(())
    }

    fn probe(service: &str, command: &str) -> AppResult<ProxyProbe> {
        let output = Command::new("networksetup")
            .args([command, service])
            .output()
            .map_err(|error| AppError::Core(format!("networksetup: {error}")))?;
        if !output.status.success() {
            return Err(AppError::Core(format!(
                "networksetup {command} {service:?} failed"
            )));
        }
        let mut probe = ProxyProbe::default();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            match key.trim() {
                "Enabled" => probe.enabled = value.trim().eq_ignore_ascii_case("yes"),
                "Server" => probe.server = value.trim().to_string(),
                "Port" => probe.port = value.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
        Ok(probe)
    }
}

impl SystemProxy for MacSystemProxy {
    fn enable(&self, host: &str, port: u16) -> AppResult<SystemProxySnapshot> {
        let services = Self::services()?;
        let port_s = port.to_string();
        let mut enabled_services = Vec::new();

        for svc in &services {
            // Prefer Wi-Fi / Ethernet; try all that accept the command
            let web = Command::new("networksetup")
                .args(["-setwebproxy", svc, host, &port_s])
                .status();
            if !matches!(web, Ok(s) if s.success()) {
                continue;
            }
            let _ = Self::run(&["-setsecurewebproxy", svc, host, &port_s]);
            let _ = Self::run(&["-setwebproxystate", svc, "on"]);
            let _ = Self::run(&["-setsecurewebproxystate", svc, "on"]);
            // SOCKS optional — mixed supports SOCKS too
            let _ = Command::new("networksetup")
                .args(["-setsocksfirewallproxy", svc, host, &port_s])
                .status();
            let _ = Command::new("networksetup")
                .args(["-setsocksfirewallproxystate", svc, "on"])
                .status();
            enabled_services.push(svc.clone());
        }

        if enabled_services.is_empty() {
            return Err(AppError::Core(
                "failed to enable system proxy on any network service".into(),
            ));
        }

        Ok(SystemProxySnapshot {
            detail: enabled_services.join("|"),
        })
    }

    fn disable(&self, snapshot: Option<&SystemProxySnapshot>) -> AppResult<()> {
        let services: Vec<String> = if let Some(s) = snapshot {
            s.detail
                .split('|')
                .filter(|x| !x.is_empty())
                .map(|s| s.to_string())
                .collect()
        } else {
            Self::services().unwrap_or_else(|_| vec!["Wi-Fi".into()])
        };

        let mut first_error = None;
        for svc in services {
            for command in [
                "-setwebproxystate",
                "-setsecurewebproxystate",
                "-setsocksfirewallproxystate",
            ] {
                if let Err(error) = Self::run(&[command, &svc, "off"]) {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }

    fn detect_owned(&self, host: &str, port: u16) -> AppResult<Option<SystemProxySnapshot>> {
        let mut owned_services = Vec::new();
        for service in Self::services()? {
            let probes: Vec<_> = [
                "-getwebproxy",
                "-getsecurewebproxy",
                "-getsocksfirewallproxy",
            ]
            .into_iter()
            .filter_map(|command| Self::probe(&service, command).ok())
            .filter(|probe| probe.enabled)
            .collect();
            let has_owned = probes
                .iter()
                .any(|probe| probe.server.eq_ignore_ascii_case(host) && probe.port == port);
            let has_foreign = probes
                .iter()
                .any(|probe| !probe.server.eq_ignore_ascii_case(host) || probe.port != port);
            if has_owned && !has_foreign {
                owned_services.push(service);
            }
        }
        if owned_services.is_empty() {
            Ok(None)
        } else {
            Ok(Some(SystemProxySnapshot {
                detail: owned_services.join("|"),
            }))
        }
    }
}
