//! macOS system proxy via `networksetup`.

use super::{SystemProxy, SystemProxySnapshot};
use crate::error::{AppError, AppResult};
use std::process::Command;

#[derive(Default)]
pub struct MacSystemProxy;

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

        for svc in services {
            let _ = Self::run(&["-setwebproxystate", &svc, "off"]);
            let _ = Self::run(&["-setsecurewebproxystate", &svc, "off"]);
            let _ = Command::new("networksetup")
                .args(["-setsocksfirewallproxystate", &svc, "off"])
                .status();
        }
        Ok(())
    }
}
